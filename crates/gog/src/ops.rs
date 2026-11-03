//! Game operations: install and update as a download-queue job, repair,
//! uninstall and import, all native ([`crate::download`]).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use heronite_data::proc::Cancel;
use heronite_data::DataPaths;
use heronite_epic::manager::{Job, JobResult};
use heronite_epic::ops::DownloadSettings;
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, ElementKind};
use heronite_epic::util::file_size;
use heronite_gog_api::plan::Mode;
use heronite_i18n::tr;
use serde_json::Value;

use crate::disk::{dir_size, remove_tree, safe_to_remove};
use crate::download::{self, Outcome, Request, Step};
use crate::gamesdb::id_text;
use crate::info::os_name;
use crate::installed::{self, Installed};
use crate::{library, manifest_file, support_dir, updates};

pub use crate::moves::{move_install, relocate};

const DEFAULT_LANGUAGE: &str = "en-US";

/// Installs and updates for the download queue.
pub struct GogJob {
    pub paths: DataPaths,
}

fn entry_size(path: &Path) -> String {
    file_size(dir_size(path) as f64)
}

/// The installed game's language, `en-US` when it has none.
fn language_of(entry: &Installed) -> String {
    if entry.language.is_empty() {
        DEFAULT_LANGUAGE.to_string()
    } else {
        entry.language.clone()
    }
}

impl GogJob {
    fn transfer(
        &self,
        req: &Request,
        mode: Mode,
        cancel: &Cancel,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Option<download::Installed>> {
        let outcome = download::run(&self.paths, req, mode, cancel, &mut |step| {
            if let Step::Downloading(p) = step {
                on_progress(Progress::from(&p));
            }
        })?;
        Ok(match outcome {
            Outcome::Done(done) => Some(done),
            Outcome::Cancelled => None,
        })
    }

    fn install(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> Result<bool> {
        let p = &el.params;
        let language = if p.install_language.is_empty() { DEFAULT_LANGUAGE } else { p.install_language.as_str() };
        let req = Request {
            app_name: p.app_name.clone(),
            os: os_name(&p.platform_to_install).to_string(),
            language: language.to_string(),
            dlcs: (!p.install_dlcs.is_empty()).then(|| p.install_dlcs.clone()),
            base_path: p.path.clone(),
            max_workers: DownloadSettings::load(&self.paths).max_workers,
            ignore_free_space: p.ignore_free_space,
            ..Default::default()
        };
        // A build left by an earlier install would turn this download into an update.
        let _ = std::fs::remove_file(manifest_file(&self.paths.gogdl_config_dir(), &p.app_name));
        let Some(done) = self.transfer(&req, Mode::Install, cancel, on_progress)? else { return Ok(false) };

        let mut entry = Installed::new(&p.app_name);
        entry.platform = req.os.clone();
        entry.install_path = done.game_dir.display().to_string();
        entry.install_size = entry_size(&done.game_dir);
        entry.version = done.version_name;
        entry.language = language.to_string();
        entry.installed_dlcs = if done.dlcs.is_empty() { p.install_dlcs.clone() } else { done.dlcs };
        entry.version_etag = done.version_etag;
        entry.build_id = done.build_id;
        installed::upsert(&self.paths, &entry)?;
        updates::forget(&self.paths, &p.app_name);
        library::sync_install_state(&self.paths, &p.app_name, Some(&done.folder_name));
        Ok(true)
    }

    fn update(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> Result<bool> {
        let app = el.params.app_name.as_str();
        let Some(mut entry) = installed::get(&self.paths, app) else { bail!(tr!("{app} is not installed", app = app)) };
        if entry.from_galaxy() {
            bail!(tr!("GOG Galaxy installed this game: update it in GOG Galaxy"));
        }
        let req = Request {
            app_name: app.to_string(),
            os: os_name(&entry.platform).to_string(),
            language: language_of(&entry),
            dlcs: Some(entry.installed_dlcs.clone()).filter(|d| !d.is_empty()),
            game_dir: Some(PathBuf::from(&entry.install_path)),
            max_workers: DownloadSettings::load(&self.paths).max_workers,
            ignore_free_space: el.params.ignore_free_space,
            ..Default::default()
        };
        let Some(done) = self.transfer(&req, Mode::Update, cancel, on_progress)? else { return Ok(false) };
        entry.build_id = done.build_id;
        entry.version = done.version_name;
        entry.version_etag = done.version_etag;
        if !done.dlcs.is_empty() {
            entry.installed_dlcs = done.dlcs;
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

/// Checks the files of an installed game against its recorded build and
/// fetches what is broken or missing. `on_percent` is `0..=100`: the check,
/// then the download.
pub fn repair(paths: &DataPaths, app_name: &str, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<()> {
    let Some(mut entry) = installed::get(paths, app_name) else { bail!(tr!("{app} is not installed", app = app_name)) };
    if entry.from_galaxy() {
        bail!(tr!("GOG Galaxy installed this game: repair it in GOG Galaxy"));
    }
    let req = Request {
        app_name: app_name.to_string(),
        os: os_name(&entry.platform).to_string(),
        language: language_of(&entry),
        dlcs: Some(entry.installed_dlcs.clone()).filter(|d| !d.is_empty()),
        game_dir: Some(PathBuf::from(&entry.install_path)),
        build_id: Some(entry.build_id.clone()).filter(|b| !b.is_empty()),
        max_workers: DownloadSettings::load(paths).max_workers,
        ignore_free_space: true,
        ..Default::default()
    };
    let outcome = download::run(paths, &req, Mode::Repair, cancel, &mut |step| match step {
        Step::Verifying(p) => on_percent(p),
        Step::Downloading(p) => on_percent(p.percent()),
    })?;
    if outcome == Outcome::Cancelled {
        bail!(tr!("The repair was canceled"));
    }
    entry.install_size = entry_size(Path::new(&entry.install_path));
    installed::upsert(paths, &entry)?;
    on_percent(100.);
    Ok(())
}

/// Deletes the game's files and the records of its build. Never runs the game's own
/// uninstaller. A folder that is already gone only drops the records. A game
/// GOG Galaxy installed is only taken off the list (its files are Galaxy's):
/// the note says so.
pub fn uninstall(paths: &DataPaths, app_name: &str) -> Result<Option<String>> {
    if let Some(entry) = installed::get(paths, app_name) {
        if entry.from_galaxy() {
            forget(paths, app_name)?;
            return Ok(Some(tr!("Its files stay where GOG Galaxy put them: uninstall it in GOG Galaxy to delete them.").to_string()));
        }
        let folder = Path::new(&entry.install_path);
        let home = crate::disk::home();
        if entry.install_path.is_empty() || !safe_to_remove(folder, home.as_deref()) {
            bail!(tr!("Refusing to delete the game at {path}", path = entry.install_path));
        }
        if folder.exists() {
            remove_tree(folder).map_err(|e| anyhow!(tr!("Could not delete {path}: {error}", path = entry.install_path, error = e)))?;
        }
    }
    forget(paths, app_name)?;
    Ok(None)
}

/// Takes the game off the installed list and drops the records of its build,
/// leaving any files where they are (a folder deleted or moved by hand).
pub fn forget(paths: &DataPaths, app_name: &str) -> Result<()> {
    let config_dir = paths.gogdl_config_dir();
    let _ = std::fs::remove_file(manifest_file(&config_dir, app_name));
    let _ = remove_tree(&support_dir(&config_dir, app_name));
    installed::remove(paths, app_name)?;
    updates::forget(paths, app_name);
    library::sync_install_state(paths, app_name, None);
    // Installed again, it is set up again at its first launch.
    heronite_launch::native_setup::forget(paths, "gog", app_name);
    Ok(())
}

/// Adds a game that is already on disk. Returns its app name.
pub fn import(paths: &DataPaths, folder: &str) -> Result<String> {
    // The version name of the build comes from GOG; offline the build id stands in.
    let client = download::signed_in_client(paths).unwrap_or_else(|err| {
        log::warn!("gog: importing without a sign-in: {err:#}");
        heronite_gog_api::Client::anonymous()
    });
    let doc = heronite_gog_api::import::import(&client, Path::new(folder))?;
    let Some(app_name) = id_text(doc.get("appName")) else {
        bail!(tr!("No GOG game was found in that folder"));
    };
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
    use crate::testing::setup;
    use heronite_epic::cache_store::CacheStore;
    use heronite_epic::queue::{DmStatus, InstallParams};
    use serde_json::json;

    fn element(kind: ElementKind, base: &Path) -> DmElement {
        DmElement::new(
            kind,
            InstallParams {
                app_name: "42".into(),
                path: base.display().to_string(),
                install_language: "de-DE".into(),
                runner: "gog".into(),
                platform_to_install: "Windows".into(),
                ..Default::default()
            },
        )
    }

    fn library_with_game(paths: &DataPaths) {
        CacheStore::new(paths, "gog_library", None)
            .set("games", json!([{"runner": "gog", "app_name": "42", "title": "The Game", "is_installed": false, "install": {"is_dlc": false}, "folder_name": ""}]))
            .unwrap();
    }

    /// A sign-in that is still valid (no refresh, so no network).
    fn signed_in(paths: &DataPaths) {
        std::fs::create_dir_all(paths.gog_auth().parent().unwrap()).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        std::fs::write(
            paths.gog_auth(),
            format!(
                r#"{{"46899977096215655":{{"access_token":"t","refresh_token":"r","user_id":"1","expires_in":3600,"loginTime":{now}}}}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn jobs_need_the_sign_in_and_an_installed_game_to_update() {
        let (dir, paths) = setup();
        let job = GogJob { paths: paths.clone() };
        let result = job.run(&element(ElementKind::Install, &dir.path().join("Games")), &Cancel::new(), &mut |_| {});
        assert_eq!(result.status, DmStatus::Error);
        assert!(result.error.unwrap().contains("not logged in"));
        assert!(installed::get(&paths, "42").is_none());
        let result = job.run(&element(ElementKind::Update, &dir.path().join("Games")), &Cancel::new(), &mut |_| {});
        assert!(result.error.unwrap().contains("not installed"));
    }

    #[test]
    fn repair_needs_a_record_of_the_build() {
        let (dir, paths) = setup();
        signed_in(&paths);
        let mut entry = Installed::new("42");
        entry.platform = "windows".into();
        entry.install_path = dir.path().join("Games/The Game").display().to_string();
        installed::upsert(&paths, &entry).unwrap();
        let err = repair(&paths, "42", &Cancel::new(), &mut |_| {}).unwrap_err().to_string();
        assert!(err.contains("cannot be repaired"), "{err}");
        assert!(repair(&paths, "other", &Cancel::new(), &mut |_| {}).unwrap_err().to_string().contains("not installed"));
    }

    #[test]
    fn uninstall_removes_files_records_and_the_install_state() {
        let (dir, paths) = setup();
        library_with_game(&paths);
        let game_dir = dir.path().join("Games").join("The Game");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("a.bin"), "x").unwrap();
        let mut entry = Installed::new("42");
        entry.install_path = game_dir.display().to_string();
        installed::upsert(&paths, &entry).unwrap();
        library::sync_install_state(&paths, "42", None);
        let cfg = paths.gogdl_config_dir();
        std::fs::create_dir_all(manifest_file(&cfg, "42").parent().unwrap()).unwrap();
        std::fs::write(manifest_file(&cfg, "42"), "{}").unwrap();
        std::fs::create_dir_all(support_dir(&cfg, "42")).unwrap();
        std::fs::write(support_dir(&cfg, "42").join("setup.exe"), "x").unwrap();
        updates::save(&paths, &["42".into()]).unwrap();
        heronite_launch::native_setup::record(&paths, "gog", "42", &entry.install_path).unwrap();

        uninstall(&paths, "42").unwrap();
        assert!(!game_dir.exists());
        assert!(dir.path().join("Games").exists());
        assert!(!manifest_file(&cfg, "42").exists() && !support_dir(&cfg, "42").exists());
        assert!(installed::get(&paths, "42").is_none());
        assert!(updates::cached_updates(&paths).is_empty());
        assert_eq!(library::game_info(&paths, "42").unwrap()["is_installed"], false);
        // Installed again, the game is set up again.
        assert!(!heronite_launch::native_setup::is_recorded(&paths, "gog", "42", &entry.install_path));
        // Nothing left to do is fine.
        uninstall(&paths, "42").unwrap();
    }

    #[test]
    fn uninstall_and_forget_work_when_the_folder_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        for app in ["1", "2"] {
            let mut e = Installed::new(app);
            e.install_path = dir.path().join(format!("gone-{app}")).display().to_string();
            installed::upsert(&paths, &e).unwrap();
        }
        uninstall(&paths, "1").unwrap();
        assert!(installed::get(&paths, "1").is_none());
        // Forgetting keeps the files of a folder that is there.
        let kept = dir.path().join("kept");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::write(kept.join("save.dat"), "x").unwrap();
        let mut e = installed::get(&paths, "2").unwrap();
        e.install_path = kept.display().to_string();
        installed::upsert(&paths, &e).unwrap();
        forget(&paths, "2").unwrap();
        assert!(installed::get(&paths, "2").is_none());
        assert!(kept.join("save.dat").exists());
    }

    #[test]
    fn uninstall_refuses_dangerous_folders() {
        let (_dir, paths) = setup();
        let home = crate::disk::home().map(|h| h.display().to_string()).unwrap_or_else(|| "/home/nobody".into());
        let mut bad = vec!["", "/", "relative/dir", "/tmp", home.as_str(), "/tmp/a/../b"];
        if cfg!(windows) {
            bad.extend([r"C:\", r"C:\Games", r"C:\Users", r"C:\Games\..\Windows\System32", r"\Games\Game"]);
        }
        for (i, bad) in bad.into_iter().enumerate() {
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
        let folder = dir.path().join("existing").join("The Game");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("gameinfo"), "The Game\n3.0\n\nfr-FR\n42\n1\n123\n").unwrap();
        std::fs::write(folder.join("a.bin"), [0u8; 1024]).unwrap();
        let name = import(&paths, &folder.display().to_string()).unwrap();
        assert_eq!(name, "42");
        let entry = installed::get(&paths, "42").unwrap();
        assert_eq!(entry.install_path, folder.display().to_string());
        assert_eq!((entry.platform.as_str(), entry.version.as_str(), entry.build_id.as_str()), ("linux", "3.0", "123"));
        assert_eq!(entry.language, "fr-FR");
        assert!(entry.install_size.ends_with("KiB"), "{}", entry.install_size);
        assert_eq!(library::game_info(&paths, "42").unwrap()["is_installed"], true);
    }

    #[test]
    fn import_rejects_unknown_and_unowned_games() {
        let (dir, paths) = setup();
        library_with_game(&paths);
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let err = import(&paths, &empty.display().to_string()).unwrap_err().to_string();
        assert!(err.contains("No GOG game was found"), "{err}");
        let foreign = dir.path().join("foreign");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("gameinfo"), "Other\n1\n\nen-US\n999\n1\n1\n").unwrap();
        let err = import(&paths, &foreign.display().to_string()).unwrap_err().to_string();
        assert!(err.contains("not in your GOG library"), "{err}");
        assert!(installed::load(&paths).is_empty());
    }
}
