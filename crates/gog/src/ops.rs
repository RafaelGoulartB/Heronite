//! Game operations through gogdl: install and update as a download-queue
//! job, repair, uninstall and import.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use heronite_data::DataPaths;
use heronite_epic::cmd::Output;
use heronite_epic::manager::{Job, JobResult};
use heronite_epic::ops::DownloadSettings;
use heronite_epic::progress::{verification_percent, Progress, ToolProgressParser};
use heronite_epic::queue::{DmElement, ElementKind};
use heronite_epic::util::file_size;
use heronite_i18n::tr;
use serde_json::Value;

use crate::cmd::{last_json_object, manifest_file, support_dir, Cancel, Gogdl};
use crate::disk::{dir_size, remove_tree, safe_to_remove};
use crate::gamesdb::id_text;
use crate::info::{self, os_name};
use crate::installed::{self, Installed};
use crate::{library, updates};

pub use crate::moves::move_install;

const DEFAULT_LANGUAGE: &str = "en-US";

/// Installs and updates for the download queue.
pub struct GogJob {
    pub paths: DataPaths,
    pub gogdl: Gogdl,
}

/// `--with-dlcs --dlcs a,b` or `--skip-dlcs`.
fn dlc_args(dlcs: &[String]) -> Vec<String> {
    if dlcs.is_empty() {
        vec!["--skip-dlcs".into()]
    } else {
        vec!["--with-dlcs".into(), "--dlcs".into(), dlcs.join(",")]
    }
}

/// Arguments of `download`, `update` and `repair`, which share their options.
struct Transfer<'a> {
    verb: &'a str,
    app_name: &'a str,
    platform: &'a str,
    path: &'a str,
    support: &'a Path,
    language: &'a str,
    dlcs: &'a [String],
    max_workers: u32,
}

impl Transfer<'_> {
    fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec![
            self.verb.into(),
            self.app_name.into(),
            "--platform".into(),
            self.platform.into(),
            "--path".into(),
            self.path.into(),
            "--support".into(),
            self.support.display().to_string(),
            "--lang".into(),
            self.language.into(),
        ];
        args.extend(dlc_args(self.dlcs));
        if self.max_workers > 0 {
            args.extend(["--max-workers".into(), self.max_workers.to_string()]);
        }
        args
    }
}

/// Why a finished run failed, for the user.
fn failure(out: &Output) -> anyhow::Error {
    if out.stdout.contains("Not enough disk space") {
        return anyhow!(tr!("Not enough disk space"));
    }
    let why = out.error_lines();
    if !why.is_empty() {
        return anyhow!(why);
    }
    anyhow!(tr!("gogdl exited with {code}", code = format!("{:?}", out.code)))
}

/// `Ok(true)` when the run succeeded, `Ok(false)` when it was cancelled.
fn finished(out: &Output) -> Result<bool> {
    if out.aborted {
        Ok(false)
    } else if out.success() {
        Ok(true)
    } else {
        Err(failure(out))
    }
}

fn entry_size(path: &Path) -> String {
    file_size(dir_size(path) as f64)
}

impl GogJob {
    fn run_transfer(&self, args: &[String], cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> Result<bool> {
        let mut parser = ToolProgressParser::new();
        let out = self.gogdl.run_streaming(args, cancel, |line| {
            if let Some(progress) = parser.feed(line) {
                on_progress(progress);
            }
        })?;
        finished(&out)
    }

    fn install(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> Result<bool> {
        let p = &el.params;
        let platform = os_name(&p.platform_to_install);
        let language = if p.install_language.is_empty() { DEFAULT_LANGUAGE } else { p.install_language.as_str() };
        let details = info::install_info(&self.paths, &self.gogdl, &p.app_name, platform)?;
        let folder = if details.folder_name.is_empty() { p.app_name.as_str() } else { details.folder_name.as_str() };

        // A manifest left by an earlier install would turn this download into an update.
        let _ = std::fs::remove_file(self.gogdl.manifest_file(&p.app_name));
        let support = self.gogdl.support_dir(&p.app_name);
        let args = Transfer {
            verb: "download",
            app_name: &p.app_name,
            platform,
            path: &p.path,
            support: &support,
            language,
            dlcs: &p.install_dlcs,
            max_workers: DownloadSettings::load(&self.paths).max_workers,
        }
        .args();
        if !self.run_transfer(&args, cancel, on_progress)? {
            return Ok(false);
        }

        // gogdl puts the game in a folder of its own below the chosen path.
        let install_path = Path::new(&p.path).join(folder);
        let mut entry = Installed::new(&p.app_name);
        entry.platform = platform.to_string();
        entry.install_path = install_path.display().to_string();
        entry.install_size = entry_size(&install_path);
        entry.version = details.version_name.clone();
        entry.language = language.to_string();
        entry.installed_dlcs = p.install_dlcs.clone();
        entry.version_etag = details.version_etag.clone();
        entry.build_id = details.build_id.clone();
        installed::upsert(&self.paths, &entry)?;
        updates::forget(&self.paths, &p.app_name);
        library::sync_install_state(&self.paths, &p.app_name, Some(folder));
        Ok(true)
    }

    fn update(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> Result<bool> {
        let app = el.params.app_name.as_str();
        let Some(mut entry) = installed::get(&self.paths, app) else { bail!(tr!("{app} is not installed", app = app)) };
        let language = if entry.language.is_empty() { DEFAULT_LANGUAGE.to_string() } else { entry.language.clone() };
        let support = self.gogdl.support_dir(app);
        let args = Transfer {
            verb: "update",
            app_name: app,
            platform: os_name(&entry.platform),
            path: &entry.install_path,
            support: &support,
            language: &language,
            dlcs: &entry.installed_dlcs,
            max_workers: DownloadSettings::load(&self.paths).max_workers,
        }
        .args();
        if !self.run_transfer(&args, cancel, on_progress)? {
            return Ok(false);
        }

        match info::install_info_fresh(&self.paths, &self.gogdl, app, &entry.platform) {
            Ok(details) => {
                entry.build_id = details.build_id;
                entry.version = details.version_name;
                entry.version_etag = details.version_etag;
            }
            Err(err) => log::warn!("reading the new build of GOG game {app}: {err:#}"),
        }
        entry.install_size = entry_size(Path::new(&entry.install_path));
        installed::upsert(&self.paths, &entry)?;
        updates::forget(&self.paths, app);
        library::sync_install_state(&self.paths, app, None);
        Ok(true)
    }
}

impl Job for GogJob {
    fn run(&self, element: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        let result = match element.kind {
            ElementKind::Install => self.install(element, cancel, on_progress),
            ElementKind::Update => self.update(element, cancel, on_progress),
        };
        match result {
            Ok(true) => JobResult::done(),
            Ok(false) => JobResult::abort(),
            Err(err) => JobResult::error(format!("{err:#}")),
        }
    }
}

/// Checks the files of an installed game against gogdl's manifest and fetches
/// what is broken or missing. `on_percent` is `0..=100`.
pub fn repair(paths: &DataPaths, gogdl: &Gogdl, app_name: &str, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<()> {
    let Some(entry) = installed::get(paths, app_name) else { bail!(tr!("{app} is not installed", app = app_name)) };
    if !gogdl.manifest_file(app_name).exists() {
        bail!(tr!("gogdl has no record of this game's files, so it cannot be repaired"));
    }
    let language = if entry.language.is_empty() { DEFAULT_LANGUAGE.to_string() } else { entry.language.clone() };
    let support = gogdl.support_dir(app_name);
    let mut args = Transfer {
        verb: "repair",
        app_name,
        platform: os_name(&entry.platform),
        path: &entry.install_path,
        support: &support,
        language: &language,
        dlcs: &entry.installed_dlcs,
        max_workers: DownloadSettings::load(paths).max_workers,
    }
    .args();
    if !entry.build_id.is_empty() {
        args.push(format!("-b={}", entry.build_id));
    }
    let mut parser = ToolProgressParser::new();
    let out = gogdl.run_streaming(&args, cancel, |line| {
        if let Some(progress) = parser.feed(line) {
            on_percent(progress.percent);
        } else if let Some(percent) = verification_percent(line) {
            on_percent(percent);
        }
    })?;
    if !finished(&out)? {
        bail!(tr!("The repair was canceled"));
    }
    let mut entry = entry;
    entry.install_size = entry_size(Path::new(&entry.install_path));
    installed::upsert(paths, &entry)?;
    on_percent(100.);
    Ok(())
}

/// Deletes the game's files and gogdl's records. Never runs the game's own
/// uninstaller.
pub fn uninstall(paths: &DataPaths, app_name: &str) -> Result<()> {
    let config_dir = paths.gogdl_config_dir();
    if let Some(entry) = installed::get(paths, app_name) {
        let folder = Path::new(&entry.install_path);
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        if entry.install_path.is_empty() || !safe_to_remove(folder, home.as_deref()) {
            bail!(tr!("Refusing to delete the game at {path}", path = entry.install_path));
        }
        remove_tree(folder).map_err(|e| anyhow!(tr!("Could not delete {path}: {error}", path = entry.install_path, error = e)))?;
    }
    let _ = std::fs::remove_file(manifest_file(&config_dir, app_name));
    let _ = remove_tree(&support_dir(&config_dir, app_name));
    installed::remove(paths, app_name)?;
    updates::forget(paths, app_name);
    library::sync_install_state(paths, app_name, None);
    Ok(())
}

/// Adds a game that is already on disk. Returns its app name.
pub fn import(paths: &DataPaths, gogdl: &Gogdl, folder: &str) -> Result<String> {
    let out = gogdl.run(&["import", folder])?;
    let found = last_json_object(&out.stdout);
    let Some(app_name) = found.as_ref().and_then(|d| id_text(d.get("appName"))) else {
        bail!(tr!("No GOG game was found in that folder"));
    };
    let doc = found.unwrap_or(Value::Null);
    if !library::owns(paths, &app_name) {
        bail!(tr!("That game is not in your GOG library"));
    }
    let text = |k: &str| id_text(doc.get(k)).unwrap_or_default();
    let mut entry = Installed::new(&app_name);
    entry.platform = if text("platform").is_empty() { "windows".to_string() } else { text("platform") };
    entry.install_path = folder.to_string();
    entry.install_size = entry_size(Path::new(folder));
    entry.version = text("versionName");
    entry.language = if text("installedLanguage").is_empty() { DEFAULT_LANGUAGE.to_string() } else { text("installedLanguage") };
    entry.build_id = text("buildId");
    entry.installed_dlcs =
        doc.get("dlcs").and_then(Value::as_array).map(|list| list.iter().filter_map(|d| id_text(Some(d))).collect()).unwrap_or_default();
    installed::upsert(paths, &entry)?;
    library::sync_install_state(paths, &app_name, None);
    Ok(app_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::*;
    use heronite_data::json_store;
    use heronite_epic::cache_store::CacheStore;
    use heronite_epic::queue::{DmStatus, InstallParams};
    use serde_json::json;
    use std::time::Duration;

    const INFO: &str = r#"{"size":{"*":{"download_size":1000,"disk_size":2000},"en-US":{"download_size":1,"disk_size":2}},"dlcs":[{"title":"DLC","id":"77","size":{"*":{"download_size":5,"disk_size":5}}}],"buildId":"900","languages":["en-US","de-DE"],"folder_name":"The Game","dependencies":[],"versionEtag":"\"tag1\"","versionName":"1.5","available_branches":[null]}"#;

    const PROGRESS: &str = r#"
echo '[PROGRESS] INFO: = Progress: 42.50 4250/10000, Running for: 00:00:05, ETA: 00:01:23' >&2
echo '[PROGRESS] INFO: = Downloaded: 123.45 MiB, Written: 150.00 MiB' >&2
echo '[PROGRESS] INFO:  + Download	- 5.12 MiB/s (raw) / 8.00 MiB/s (decompressed)' >&2
echo '[PROGRESS] INFO:  + Disk	- 9.00 MiB/s (write) / 0.00 MiB/s (read)' >&2
"#;

    fn element(kind: ElementKind, base: &Path, dlcs: &[&str]) -> DmElement {
        DmElement::new(
            kind,
            InstallParams {
                app_name: "42".into(),
                path: base.display().to_string(),
                install_dlcs: dlcs.iter().map(|s| s.to_string()).collect(),
                install_language: "de-DE".into(),
                runner: "gog".into(),
                platform_to_install: "Windows".into(),
                ..Default::default()
            },
        )
    }

    /// A gogdl that answers `info`, and for `download` fills the game folder.
    fn fake_gog(dir: &Path, paths: &DataPaths, download: &str) -> Gogdl {
        std::fs::write(dir.join("info.json"), INFO).unwrap();
        let body = format!("case \"$3\" in\ninfo) cat \"{info}\";;\n*) {download};;\nesac", info = dir.join("info.json").display());
        fake(dir, paths, &body)
    }

    fn library_with_game(paths: &DataPaths) {
        CacheStore::new(paths, "gog_library", None)
            .set("games", json!([{"runner": "gog", "app_name": "42", "title": "The Game", "is_installed": false, "install": {"is_dlc": false}, "folder_name": ""}]))
            .unwrap();
    }

    #[test]
    fn install_runs_gogdl_and_records_the_game() {
        let (dir, paths) = setup();
        let base = dir.path().join("Games");
        library_with_game(&paths);
        let download = format!("{PROGRESS}\nmkdir -p \"{b}/The Game\"; echo data > \"{b}/The Game/a.bin\"", b = base.display());
        let job = GogJob { paths: paths.clone(), gogdl: fake_gog(dir.path(), &paths, &download) };
        // A manifest of an earlier install is removed first.
        std::fs::create_dir_all(job.gogdl.manifest_file("42").parent().unwrap()).unwrap();
        std::fs::write(job.gogdl.manifest_file("42"), "{}").unwrap();

        let mut reports = Vec::new();
        let result = job.run(&element(ElementKind::Install, &base, &["77"]), &Cancel::new(), &mut |p| reports.push(p));
        assert_eq!(result.status, DmStatus::Done, "{:?}", result.error);
        assert!(!job.gogdl.manifest_file("42").exists());

        let calls = calls(dir.path());
        assert_eq!(calls[0], "info 42 --os windows");
        assert_eq!(
            calls[1],
            format!(
                "download 42 --platform windows --path {} --support {} --lang de-DE --with-dlcs --dlcs 77",
                base.display(),
                crate::cmd::support_dir(&paths.gogdl_config_dir(), "42").display()
            )
        );
        assert_eq!(reports.len(), 1);
        assert_eq!((reports[0].percent, reports[0].eta.as_str()), (42.5, "00:01:23"));

        let entry = installed::get(&paths, "42").unwrap();
        assert_eq!(entry.install_path, format!("{}/The Game", base.display()));
        assert_eq!((entry.platform.as_str(), entry.language.as_str(), entry.version.as_str()), ("windows", "de-DE", "1.5"));
        assert_eq!((entry.build_id.as_str(), entry.version_etag.as_str()), ("900", "\"tag1\""));
        assert_eq!(entry.installed_dlcs, ["77"]);
        assert!(!entry.pinned_version);
        assert!(entry.install_size.ends_with(" B"), "{}", entry.install_size);
        let doc = json_store::read_value(&paths.gog_installed()).unwrap();
        assert_eq!(doc["installed"][0]["executable"], "");
        assert_eq!(doc["installed"][0]["is_dlc"], false);

        let game = library::game_info(&paths, "42").unwrap();
        assert_eq!(game["folder_name"], "The Game");
        assert_eq!(game["is_installed"], true);
        let saved = json_store::read_value(&paths.gog_library()).unwrap();
        assert_eq!(saved["games"][0]["install"]["install_path"], entry.install_path.as_str());
    }

    #[test]
    fn install_without_dlcs_skips_them_and_passes_the_worker_limit() {
        let (dir, paths) = setup();
        std::fs::create_dir_all(paths.global_config().parent().unwrap()).unwrap();
        std::fs::write(paths.global_config(), r#"{"defaultSettings":{"maxWorkers":6}}"#).unwrap();
        let base = dir.path().join("Games");
        let job = GogJob { paths: paths.clone(), gogdl: fake_gog(dir.path(), &paths, "true") };
        let mut el = element(ElementKind::Install, &base, &[]);
        el.params.install_language = String::new();
        assert_eq!(job.run(&el, &Cancel::new(), &mut |_| {}).status, DmStatus::Done);
        let download = calls(dir.path()).remove(1);
        assert!(download.ends_with("--lang en-US --skip-dlcs --max-workers 6"), "{download}");
    }

    #[test]
    fn a_failed_install_reports_gogdl_errors() {
        let (dir, paths) = setup();
        let job = GogJob { paths: paths.clone(), gogdl: fake_gog(dir.path(), &paths, "echo '[V2] ERROR: chunk failed' >&2; exit 1") };
        let result = job.run(&element(ElementKind::Install, &dir.path().join("g"), &[]), &Cancel::new(), &mut |_| {});
        assert_eq!(result.status, DmStatus::Error);
        assert_eq!(result.error.as_deref(), Some("[V2] ERROR: chunk failed"));
        assert!(installed::get(&paths, "42").is_none());

        let silent = GogJob { paths: paths.clone(), gogdl: fake_gog(dir.path(), &paths, "exit 4") };
        let result = silent.run(&element(ElementKind::Install, &dir.path().join("g"), &[]), &Cancel::new(), &mut |_| {});
        assert!(result.error.unwrap().contains("gogdl exited with Some(4)"));
    }

    #[test]
    fn cancelling_an_install_aborts_it() {
        let (dir, paths) = setup();
        let job = GogJob { paths: paths.clone(), gogdl: fake_gog(dir.path(), &paths, "sleep 60 & wait") };
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            c2.request();
        });
        let result = job.run(&element(ElementKind::Install, &dir.path().join("g"), &[]), &cancel, &mut |_| {});
        t.join().unwrap();
        assert_eq!(result.status, DmStatus::Abort);
        assert!(installed::get(&paths, "42").is_none());
    }

    #[test]
    fn update_runs_in_place_and_refreshes_the_entry() {
        let (dir, paths) = setup();
        let game_dir = dir.path().join("Games/The Game");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("a.bin"), [0u8; 2048]).unwrap();
        let mut old = Installed::new("42");
        old.platform = "windows".into();
        old.install_path = game_dir.display().to_string();
        old.language = "en-US".into();
        old.installed_dlcs = vec!["77".into(), "78".into()];
        old.version = "1.0".into();
        old.build_id = "100".into();
        old.version_etag = "\"old\"".into();
        installed::upsert(&paths, &old).unwrap();
        updates::save(&paths, &["42".to_string(), "9".to_string()]).unwrap();

        let job = GogJob { paths: paths.clone(), gogdl: fake_gog(dir.path(), &paths, "true") };
        let result = job.run(&element(ElementKind::Update, &dir.path().join("ignored"), &[]), &Cancel::new(), &mut |_| {});
        assert_eq!(result.status, DmStatus::Done, "{:?}", result.error);
        let calls = calls(dir.path());
        assert_eq!(
            calls[0],
            format!(
                "update 42 --platform windows --path {} --support {} --lang en-US --with-dlcs --dlcs 77,78",
                game_dir.display(),
                crate::cmd::support_dir(&paths.gogdl_config_dir(), "42").display()
            )
        );
        assert_eq!(calls[1], "info 42 --os windows");
        let entry = installed::get(&paths, "42").unwrap();
        assert_eq!((entry.build_id.as_str(), entry.version.as_str(), entry.version_etag.as_str()), ("900", "1.5", "\"tag1\""));
        assert_eq!(entry.install_size, "2 KiB");
        assert_eq!(entry.install_path, game_dir.display().to_string());
        assert_eq!(updates::cached_updates(&paths), ["9"]);

        let missing = element(ElementKind::Update, &game_dir, &[]);
        let mut other = missing.clone();
        other.params.app_name = "nope".into();
        assert!(job.run(&other, &Cancel::new(), &mut |_| {}).error.unwrap().contains("not installed"));
    }

    #[test]
    fn repair_needs_a_manifest_and_passes_the_build() {
        let (dir, paths) = setup();
        let game_dir = dir.path().join("Games/The Game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let mut entry = Installed::new("42");
        entry.platform = "windows".into();
        entry.install_path = game_dir.display().to_string();
        entry.language = "pt-BR".into();
        entry.build_id = "900".into();
        installed::upsert(&paths, &entry).unwrap();
        let body = format!("echo '[VERIFY] INFO: Verification progress: 3/8 (37.5%)' >&2\n{PROGRESS}");
        let g = fake(dir.path(), &paths, &body);

        let mut percents = Vec::new();
        let err = repair(&paths, &g, "42", &Cancel::new(), &mut |p| percents.push(p)).unwrap_err().to_string();
        assert!(err.contains("cannot be repaired"), "{err}");
        assert!(calls(dir.path()).is_empty());

        std::fs::create_dir_all(g.manifest_file("42").parent().unwrap()).unwrap();
        std::fs::write(g.manifest_file("42"), "{}").unwrap();
        repair(&paths, &g, "42", &Cancel::new(), &mut |p| percents.push(p)).unwrap();
        assert_eq!(
            calls(dir.path()),
            [format!(
                "repair 42 --platform windows --path {} --support {} --lang pt-BR --skip-dlcs -b=900",
                game_dir.display(),
                g.support_dir("42").display()
            )]
        );
        assert_eq!(percents, [37.5, 42.5, 100.]);
        assert!(repair(&paths, &g, "other", &Cancel::new(), &mut |_| {}).is_err());
    }

    #[test]
    fn uninstall_removes_files_records_and_the_install_state() {
        let (dir, paths) = setup();
        library_with_game(&paths);
        let game_dir = dir.path().join("Games/The Game");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("a.bin"), "x").unwrap();
        let mut entry = Installed::new("42");
        entry.install_path = game_dir.display().to_string();
        installed::upsert(&paths, &entry).unwrap();
        library::sync_install_state(&paths, "42", None);
        let g = Gogdl::with_bin("gogdl", &paths);
        std::fs::create_dir_all(g.manifest_file("42").parent().unwrap()).unwrap();
        std::fs::write(g.manifest_file("42"), "{}").unwrap();
        std::fs::create_dir_all(g.support_dir("42")).unwrap();
        std::fs::write(g.support_dir("42").join("setup.exe"), "x").unwrap();
        updates::save(&paths, &["42".into()]).unwrap();

        uninstall(&paths, "42").unwrap();
        assert!(!game_dir.exists());
        assert!(dir.path().join("Games").exists());
        assert!(!g.manifest_file("42").exists() && !g.support_dir("42").exists());
        assert!(installed::get(&paths, "42").is_none());
        assert!(updates::cached_updates(&paths).is_empty());
        assert_eq!(library::game_info(&paths, "42").unwrap()["is_installed"], false);
        // Nothing left to do is fine.
        uninstall(&paths, "42").unwrap();
    }

    #[test]
    fn uninstall_refuses_dangerous_folders() {
        let (_dir, paths) = setup();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/nobody".into());
        for (i, bad) in ["", "/", "relative/dir", "/tmp", home.as_str(), "/tmp/a/../b"].into_iter().enumerate() {
            let app = format!("bad{i}");
            let mut entry = Installed::new(&app);
            entry.install_path = bad.to_string();
            installed::upsert(&paths, &entry).unwrap();
            assert!(uninstall(&paths, &app).is_err(), "{bad:?} was accepted");
            // The record stays so the user can fix it.
            assert!(installed::get(&paths, &app).is_some());
        }
    }

    #[test]
    fn import_records_a_game_found_on_disk() {
        let (dir, paths) = setup();
        library_with_game(&paths);
        let folder = dir.path().join("existing/The Game");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.bin"), [0u8; 1024]).unwrap();
        let reply = r#"{"appName":"42","buildId":"123","title":"The Game","tasks":null,"installedLanguage":"fr-FR","dlcs":["5","6"],"platform":"windows","versionName":"3.0"}"#;
        let g = fake(dir.path(), &paths, &format!("echo '{reply}'"));
        let name = import(&paths, &g, &folder.display().to_string()).unwrap();
        assert_eq!(name, "42");
        assert_eq!(calls(dir.path()), [format!("import {}", folder.display())]);
        let entry = installed::get(&paths, "42").unwrap();
        assert_eq!(entry.install_path, folder.display().to_string());
        assert_eq!((entry.platform.as_str(), entry.version.as_str(), entry.build_id.as_str()), ("windows", "3.0", "123"));
        assert_eq!((entry.language.as_str(), entry.install_size.as_str()), ("fr-FR", "1 KiB"));
        assert_eq!(entry.installed_dlcs, ["5", "6"]);
        assert_eq!(library::game_info(&paths, "42").unwrap()["is_installed"], true);
    }

    #[test]
    fn import_rejects_unknown_and_unowned_games() {
        let (dir, paths) = setup();
        library_with_game(&paths);
        let nothing = fake(dir.path(), &paths, "echo 'Error importing, no info file'");
        let err = import(&paths, &nothing, "/x").unwrap_err().to_string();
        assert!(err.contains("No GOG game was found"), "{err}");
        let null_name = fake(dir.path(), &paths, "echo '{\"appName\": null}'");
        assert!(import(&paths, &null_name, "/x").is_err());
        let foreign = fake(dir.path(), &paths, "echo '{\"appName\": \"999\", \"platform\": \"windows\"}'");
        let err = import(&paths, &foreign, "/x").unwrap_err().to_string();
        assert!(err.contains("not in your GOG library"), "{err}");
        assert!(installed::load(&paths).is_empty());
    }
}
