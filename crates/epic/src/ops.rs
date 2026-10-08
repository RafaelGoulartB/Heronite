//! Game operations through legendary (port of the `install`, `update`,
//! `repair`, `uninstall`, `moveInstall` and `importGame` parts of
//! `LegendaryGame`).

use std::path::Path;

use anyhow::{bail, Result};
use heronite_data::game_config;
use heronite_data::HeroicPaths;
use serde_json::{json, Value};

use crate::cmd::{Cancel, Legendary, Output};
use crate::info::install_info;
use crate::library;
use crate::manager::{Job, JobResult};
use crate::progress::{verification_percent, Progress, ProgressParser};
use crate::queue::{DmElement, DmStatus, ElementKind};

/// Settings of `config.json` that change how legendary downloads.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DownloadSettings {
    pub max_workers: u32,
    pub no_https: bool,
}

impl DownloadSettings {
    pub fn load(heroic: &HeroicPaths) -> Self {
        let g = game_config::load_global(heroic, &home());
        Self {
            max_workers: g.get("maxWorkers").and_then(Value::as_u64).unwrap_or(0) as u32,
            no_https: g.get("downloadNoHttps").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    fn append(&self, args: &mut Vec<String>) {
        if self.max_workers > 0 {
            args.extend(["--max-workers".into(), self.max_workers.to_string()]);
        }
        if self.no_https {
            args.push("--no-https".into());
        }
    }
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// `legendary install` arguments (`commandToArgsArray` order).
pub fn install_args(app_name: &str, platform: &str, base_path: &str, sdl_tags: &[String], dl: &DownloadSettings) -> Vec<String> {
    let mut args: Vec<String> = vec!["install".into(), app_name.into()];
    let tags: Vec<&String> = sdl_tags.iter().filter(|t| !t.is_empty()).collect();
    if !tags.is_empty() {
        args.push("--install-tag=".into());
        for t in &tags {
            args.extend(["--install-tag".into(), (*t).clone()]);
        }
    }
    args.extend([
        "--platform".into(),
        platform.into(),
        "--base-path".into(),
        base_path.replace('\'', ""),
        "--skip-dlcs".into(),
        "-y".into(),
    ]);
    dl.append(&mut args);
    if tags.is_empty() {
        args.push("--skip-sdl".into());
    }
    args
}

/// `legendary update` / `repair` arguments.
pub fn update_args(verb: &str, app_name: &str, dl: &DownloadSettings) -> Vec<String> {
    let mut args: Vec<String> = vec![verb.into(), app_name.into(), "-y".into(), "--skip-sdl".into()];
    dl.append(&mut args);
    args
}

/// Port of `LegendaryGame.install` / `update` as a download-manager job.
pub struct LegendaryJob {
    pub heroic: HeroicPaths,
    pub legendary: Legendary,
}

impl LegendaryJob {
    fn run_install(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        let p = &el.params;
        let third_party = p.game_info.get("thirdPartyManagedApp").and_then(Value::as_str).filter(|s| !s.is_empty());
        if let Some(app) = third_party {
            return JobResult::error(format!("{} games ({app}) are not supported yet", p.title()));
        }
        let platform = if p.platform_to_install.is_empty() { "Windows" } else { p.platform_to_install.as_str() };
        // Heroic scales progress by the manifest's download size. An update only
        // fetches the changed files, so there legendary's own percent is used.
        let total = match el.kind {
            ElementKind::Install => install_info(&self.heroic, &self.legendary, &p.app_name, platform).map(|i| i.download_size()).ok(),
            ElementKind::Update => None,
        };
        let dl = DownloadSettings::load(&self.heroic);
        let mut args = match el.kind {
            ElementKind::Install => install_args(&p.app_name, platform, &p.path, &p.sdl_list, &dl),
            ElementKind::Update => update_args("update", &p.app_name, &dl),
        };
        let mut result = self.run_once(&args, total, cancel, on_progress);
        // Retry with a higher shared memory limit, like Heroic.
        if let Ok(out) = &result {
            if !out.success() && !out.aborted && out.stderr.contains("MemoryError:") && el.kind == ElementKind::Install {
                args.extend(["--max-shared-memory".into(), "5000".into()]);
                result = self.run_once(&args, total, cancel, on_progress);
            }
        }
        match result {
            Err(err) => JobResult::error(format!("{err:#}")),
            Ok(out) if out.aborted => JobResult::abort(),
            Ok(out) if out.success() => {
                if let Err(err) = library::write_library(&self.heroic) {
                    log::warn!("rebuilding the library: {err:#}");
                }
                JobResult::done()
            }
            Ok(out) => {
                let why = out.error_lines();
                JobResult::error(if why.is_empty() { format!("legendary exited with {:?}", out.code) } else { why })
            }
        }
    }

    fn run_once(
        &self,
        args: &[String],
        total: Option<f64>,
        cancel: &Cancel,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Output> {
        let mut parser = ProgressParser::new(total);
        self.legendary.run_streaming(args, &[], cancel, |line| {
            if let Some(p) = parser.feed(line) {
                on_progress(p);
            }
        })
    }
}

impl Job for LegendaryJob {
    fn run(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        self.run_install(el, cancel, on_progress)
    }
}

/// Result of a one-off operation, for the UI to show.
#[derive(Debug, Clone, Default)]
pub struct OpResult {
    pub ok: bool,
    pub aborted: bool,
    pub message: String,
}

impl OpResult {
    pub(crate) fn ok(message: impl Into<String>) -> Self {
        Self { ok: true, aborted: false, message: message.into() }
    }
    pub(crate) fn err(message: impl Into<String>) -> Self {
        Self { ok: false, aborted: false, message: message.into() }
    }
}

/// Percent of `repair` (file verification, then re-download of broken files).
#[derive(Debug, Clone, PartialEq)]
pub enum RepairUpdate {
    Verifying(f64),
    Downloading(Progress),
}

/// `repair`: verify the files and download what is missing or corrupted.
pub fn repair(
    heroic: &HeroicPaths,
    legendary: &Legendary,
    app_name: &str,
    cancel: &Cancel,
    mut on_update: impl FnMut(RepairUpdate),
) -> OpResult {
    // Repairs download only what is broken: legendary's own percent is used.
    let mut parser = ProgressParser::new(None);
    let args = update_args("repair", app_name, &DownloadSettings::load(heroic));
    let out = legendary.run_streaming(&args, &[], cancel, |line| {
        if let Some(p) = verification_percent(line) {
            on_update(RepairUpdate::Verifying(p));
        } else if let Some(p) = parser.feed(line) {
            on_update(RepairUpdate::Downloading(p));
        }
    });
    match out {
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(out) if out.aborted => OpResult { ok: false, aborted: true, message: "Cancelled".into() },
        Ok(out) if out.success() => {
            let _ = library::write_library(heroic);
            OpResult::ok("Finished repairing")
        }
        Ok(out) => OpResult::err(out.error_lines()),
    }
}

/// `uninstall` (`legendary uninstall -y`) and rebuilds the library file. The
/// caller removes the prefix / settings afterwards if the user asked for it.
pub fn uninstall(heroic: &HeroicPaths, legendary: &Legendary, app_name: &str) -> OpResult {
    match legendary.run(&["uninstall", app_name, "-y"], &[]) {
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(out) if out.success() => {
            let _ = library::write_library(heroic);
            OpResult::ok("Uninstalled")
        }
        Ok(out) => OpResult::err(out.error_lines()),
    }
}

/// `forceUninstall`: forget the install in `installed.json` but keep the files.
pub fn force_uninstall(heroic: &HeroicPaths, legendary: &Legendary, app_name: &str) -> OpResult {
    match legendary.run(&["uninstall", app_name, "-y", "--keep-files"], &[]) {
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(out) if out.success() => {
            let _ = library::write_library(heroic);
            OpResult::ok("Removed from the installed games")
        }
        Ok(out) => OpResult::err(out.error_lines()),
    }
}

/// `importGame`: tells legendary an existing folder is the install of a game.
pub fn import_game(
    heroic: &HeroicPaths,
    legendary: &Legendary,
    app_name: &str,
    folder: &str,
    platform: &str,
) -> OpResult {
    if !Path::new(folder).is_dir() {
        return OpResult::err("The folder does not exist");
    }
    let args = ["import", app_name, folder, "--with-dlcs", "--platform", platform];
    match legendary.run(&args, &[]) {
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(out) => {
            let errors: Vec<&str> = out.stderr.lines().filter(|l| l.contains("ERROR:")).collect();
            if out.success() && errors.is_empty() {
                let _ = library::write_library(heroic);
                OpResult::ok("Game imported")
            } else {
                OpResult::err(if errors.is_empty() { out.error_lines() } else { errors.join("\n") })
            }
        }
    }
}

/// `moveOnUnix` + `changeGameInstallPath`: moves the install folder into
/// `new_parent` and tells legendary where it is now (resumable, see
/// [`crate::moves`]).
pub fn move_install(
    heroic: &HeroicPaths,
    legendary: &Legendary,
    app_name: &str,
    install_path: &str,
    new_parent: &str,
) -> OpResult {
    crate::moves::move_install(heroic, legendary, app_name, install_path, new_parent)
}

/// `removePrefix`: deletes the game's Wine prefix, with Heroic's sanity checks.
pub fn remove_prefix(heroic: &HeroicPaths, app_name: &str) -> Result<bool> {
    let home = home();
    let settings = game_config::load_game(heroic, app_name, &home);
    let global = game_config::load_global(heroic, &home);
    let prefix = settings.get("winePrefix").and_then(Value::as_str).unwrap_or_default().to_string();
    if prefix.is_empty() || !Path::new(&prefix).exists() {
        return Ok(false);
    }
    let text = |k: &str| global.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let forbidden = [text("defaultInstallPath"), game_config::shared_wine_prefix(&home), text("defaultWinePrefix")];
    if prefix == "/" || prefix == home || forbidden.iter().any(|f| !f.is_empty() && *f == prefix) {
        log::info!("Can't delete prefix {prefix}: it is a shared or default folder");
        return Ok(false);
    }
    let p = Path::new(&prefix);
    let has_content = p.read_dir()?.next().is_some();
    if has_content && !p.join("drive_c").exists() && !p.join("pfx").exists() {
        log::info!("Can't delete folder {prefix}: no drive_c/pfx folder inside");
        return Ok(false);
    }
    std::fs::remove_dir_all(p)?;
    Ok(true)
}

/// `removeSettingsAndLogs`.
pub fn remove_settings_and_logs(heroic: &HeroicPaths, app_name: &str) {
    for ext in [".json", ".log", "-lastPlay.log"] {
        let f = heroic.games_config_dir().join(format!("{app_name}{ext}"));
        if f.exists() {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// Writes the Wine settings picked in the install dialog (Heroic's
/// `writeConfig` at install time on Linux).
pub fn write_wine_config(
    heroic: &HeroicPaths,
    app_name: &str,
    wine_prefix: &str,
    wine_version: Option<&Value>,
) -> Result<()> {
    if wine_prefix.is_empty() && wine_version.is_none() {
        return Ok(());
    }
    let mut changes: Vec<(String, Value)> = Vec::new();
    if !wine_prefix.is_empty() {
        changes.push(("winePrefix".into(), json!(wine_prefix)));
    }
    if let Some(v) = wine_version {
        changes.push(("wineVersion".into(), v.clone()));
        changes.push(("wineCrossoverBottle".into(), json!("")));
    }
    game_config::set_game_settings(heroic, app_name, &home(), &changes)
}

/// Removes `<path>/<folder_name>` (the partial download of a cancelled
/// install). Refuses anything that is not a plain folder name.
pub fn remove_partial_download(path: &str, folder_name: &str) -> bool {
    if path.is_empty()
        || folder_name.is_empty()
        || folder_name.contains('/')
        || folder_name == ".."
        || folder_name == "."
    {
        return false;
    }
    let target = Path::new(path.trim_matches('\'')).join(folder_name);
    target.is_dir() && std::fs::remove_dir_all(target).is_ok()
}

impl JobResult {
    pub fn done() -> Self {
        Self { status: DmStatus::Done, error: None }
    }
    pub fn abort() -> Self {
        Self { status: DmStatus::Abort, error: None }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self { status: DmStatus::Error, error: Some(message.into()) }
    }
}

/// Convenience: errors if legendary is not logged in.
pub fn ensure_logged_in(heroic: &HeroicPaths) -> Result<()> {
    if !crate::user::is_logged_in(heroic) {
        bail!("Log in to your Epic account first");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::{calls, fake};
    use crate::queue::InstallParams;

    fn dl() -> DownloadSettings {
        DownloadSettings::default()
    }

    #[test]
    fn install_arguments_follow_heroics_order() {
        assert_eq!(
            install_args("Calluna", "Windows", "/g/Con'trol", &[], &dl()),
            ["install", "Calluna", "--platform", "Windows", "--base-path", "/g/Control", "--skip-dlcs", "-y", "--skip-sdl"]
        );
        let tags = vec!["".to_string(), "voices".to_string()];
        let dls = DownloadSettings { max_workers: 4, no_https: true };
        assert_eq!(
            install_args("A", "Windows", "/g", &tags, &dls),
            [
                "install", "A", "--install-tag=", "--install-tag", "voices", "--platform", "Windows", "--base-path", "/g",
                "--skip-dlcs", "-y", "--max-workers", "4", "--no-https"
            ]
        );
        assert_eq!(update_args("update", "A", &dls), ["update", "A", "-y", "--skip-sdl", "--max-workers", "4", "--no-https"]);
        assert_eq!(update_args("repair", "A", &dl()), ["repair", "A", "-y", "--skip-sdl"]);
    }

    fn job(dir: &Path, body: &str) -> (LegendaryJob, HeroicPaths) {
        let heroic = HeroicPaths::at(dir.join("h"));
        std::fs::create_dir_all(heroic.legendary_config_dir()).unwrap();
        let lg = fake(dir, body);
        (LegendaryJob { heroic: heroic.clone(), legendary: Legendary::with_bin(lg.bin, heroic.legendary_config_dir()) }, heroic)
    }

    fn element(kind: ElementKind) -> DmElement {
        DmElement::new(
            kind,
            InstallParams {
                app_name: "Calluna".into(),
                path: "/games".into(),
                runner: "legendary".into(),
                platform_to_install: "Windows".into(),
                game_info: json!({"title": "Control"}),
                ..Default::default()
            },
        )
    }

    const REPORT: &str = "echo '[DLManager] INFO: = Progress: 50.00% (1/2), Running for 00:00:10, ETA: 00:00:10' >&2; \
        echo '[DLManager] INFO:  - Downloaded: 100.00 MiB, Written: 99.00 MiB' >&2; \
        printf '[DLManager] INFO:  + Download\\t- 5.00 MiB/s (raw) / 5.00 MiB/s (decompressed)\\n' >&2; \
        printf '[DLManager] INFO:  + Disk\\t- 4.00 MiB/s (write) / 0.00 MiB/s (read)\\n' >&2";

    #[test]
    fn install_job_reports_progress_and_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let info = r#"{"manifest": {"download_size": 209715200, "disk_size": 1}}"#;
        let body = format!("if [ \"$1\" = info ]; then echo '{info}'; else {REPORT}; fi");
        let (job, heroic) = job(dir.path(), &body);
        let mut seen = Vec::new();
        let r = job.run(&element(ElementKind::Install), &Cancel::new(), &mut |p| seen.push(p));
        assert_eq!(r.status, DmStatus::Done, "{:?}", r.error);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].percent, 50.);
        let c = calls(dir.path());
        assert_eq!(c[0], "info Calluna --json --platform Windows");
        assert_eq!(c[1], "install Calluna --platform Windows --base-path /games --skip-dlcs -y --skip-sdl");
        // The library file was rebuilt (empty metadata folder -> no games).
        assert!(heroic.legendary_library().exists());
    }

    #[test]
    fn update_job_uses_the_update_command() {
        let dir = tempfile::tempdir().unwrap();
        let (job, _) = job(dir.path(), "if [ \"$1\" = info ]; then echo '{\"manifest\": {\"download_size\": 1}}'; fi");
        let r = job.run(&element(ElementKind::Update), &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Done);
        assert_eq!(calls(dir.path()), ["update Calluna -y --skip-sdl"]);
    }

    #[test]
    fn failures_and_third_party_games_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let (job, _) = job(dir.path(), "if [ \"$1\" = info ]; then echo '{\"manifest\": {}}'; else echo '[cli] ERROR: disk full' >&2; exit 1; fi");
        let r = job.run(&element(ElementKind::Install), &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Error);
        assert!(r.error.unwrap().contains("disk full"));
        let mut third = element(ElementKind::Install);
        third.params.game_info = json!({"title": "FIFA", "thirdPartyManagedApp": "Origin"});
        let r = job.run(&third, &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Error);
        assert!(r.error.unwrap().contains("not supported"));
    }

    #[test]
    fn memory_errors_retry_with_more_shared_memory() {
        let dir = tempfile::tempdir().unwrap();
        let body = "if [ \"$1\" = info ]; then echo '{\"manifest\": {\"download_size\": 1}}'; \
            elif echo \"$@\" | grep -q max-shared-memory; then exit 0; else echo 'MemoryError: x' >&2; exit 1; fi";
        let (job, _) = job(dir.path(), body);
        let r = job.run(&element(ElementKind::Install), &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Done);
        assert!(calls(dir.path()).last().unwrap().ends_with("--max-shared-memory 5000"));
    }

    #[test]
    fn cancelled_jobs_abort() {
        let dir = tempfile::tempdir().unwrap();
        let (job, _) = job(dir.path(), "if [ \"$1\" = info ]; then echo '{\"manifest\": {\"download_size\": 1}}'; else sleep 30; fi");
        let cancel = Cancel::new();
        let c = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            c.request();
        });
        let r = job.run(&element(ElementKind::Install), &cancel, &mut |_| {});
        t.join().unwrap();
        assert_eq!(r.status, DmStatus::Abort);
    }

    #[test]
    fn uninstall_import_and_force_uninstall_commands() {
        let dir = tempfile::tempdir().unwrap();
        let (job, heroic) = job(dir.path(), "true");
        assert!(uninstall(&heroic, &job.legendary, "Calluna").ok);
        assert!(force_uninstall(&heroic, &job.legendary, "Calluna").ok);
        let game_dir = dir.path().join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        assert!(import_game(&heroic, &job.legendary, "Calluna", game_dir.to_str().unwrap(), "Windows").ok);
        assert!(!import_game(&heroic, &job.legendary, "Calluna", "/nonexistent-dir", "Windows").ok);
        let c = calls(dir.path());
        assert_eq!(c[0], "uninstall Calluna -y");
        assert_eq!(c[1], "uninstall Calluna -y --keep-files");
        assert_eq!(c[2], format!("import Calluna {} --with-dlcs --platform Windows", game_dir.display()));
    }

    #[test]
    fn import_errors_come_from_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let (job, heroic) = job(dir.path(), "echo '[cli] ERROR: Game is not in the library' >&2");
        let r = import_game(&heroic, &job.legendary, "x", dir.path().to_str().unwrap(), "Windows");
        assert!(!r.ok);
        assert!(r.message.contains("not in the library"));
    }

    #[test]
    fn repair_reports_verification_and_download() {
        let dir = tempfile::tempdir().unwrap();
        let body = format!(
            "if [ \"$1\" = info ]; then echo '{{\"manifest\": {{\"download_size\": 1}}}}'; else \
             echo '[cli] INFO: Verification progress: 1/2 (50.0%) [10 MiB/s]' >&2; {REPORT}; fi"
        );
        let (job, heroic) = job(dir.path(), &body);
        let mut updates = Vec::new();
        let r = repair(&heroic, &job.legendary, "Calluna", &Cancel::new(), |u| updates.push(u));
        assert!(r.ok, "{}", r.message);
        assert_eq!(updates[0], RepairUpdate::Verifying(50.));
        assert!(matches!(updates[1], RepairUpdate::Downloading(_)));
    }

    #[test]
    fn move_install_moves_the_folder_and_updates_legendary() {
        let dir = tempfile::tempdir().unwrap();
        let (job, heroic) = job(dir.path(), "true");
        let src = dir.path().join("old/Control");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/a.txt"), "x").unwrap();
        let new_parent = dir.path().join("new");
        let r = move_install(&heroic, &job.legendary, "Calluna", src.to_str().unwrap(), new_parent.to_str().unwrap());
        assert!(r.ok, "{}", r.message);
        assert!(new_parent.join("Control/sub/a.txt").exists());
        assert!(!src.exists());
        assert_eq!(calls(dir.path())[0], format!("move Calluna {} --skip-move", new_parent.display()));
        // Into itself, and onto a non-empty destination: refused.
        let again = move_install(&heroic, &job.legendary, "Calluna", new_parent.join("Control").to_str().unwrap(), new_parent.join("Control/x").to_str().unwrap());
        assert!(!again.ok);
        let clash = dir.path().join("clash/Control");
        std::fs::create_dir_all(&clash).unwrap();
        std::fs::write(clash.join("f"), "x").unwrap();
        let r = move_install(&heroic, &job.legendary, "Calluna", new_parent.join("Control").to_str().unwrap(), dir.path().join("clash").to_str().unwrap());
        assert!(!r.ok && r.message.contains("not empty"));
    }

    #[test]
    fn prefix_removal_has_sanity_checks() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(dir.path().join("h"));
        let prefix = dir.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        game_config::set_game_setting(&heroic, "A", &home(), "winePrefix", json!(prefix.to_str().unwrap())).unwrap();
        assert!(remove_prefix(&heroic, "A").unwrap());
        assert!(!prefix.exists());
        // A folder without drive_c/pfx is left alone.
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("file"), "x").unwrap();
        game_config::set_game_setting(&heroic, "B", &home(), "winePrefix", json!(other.to_str().unwrap())).unwrap();
        assert!(!remove_prefix(&heroic, "B").unwrap());
        assert!(other.exists());
        // The default install path is never deleted (a temp one: never touch real folders in tests).
        let default_install = dir.path().join("Games/Heroic");
        std::fs::create_dir_all(default_install.join("drive_c")).unwrap();
        game_config::set_global_setting(&heroic, &home(), "defaultInstallPath", json!(default_install.to_str().unwrap())).unwrap();
        game_config::set_game_setting(&heroic, "C", &home(), "winePrefix", json!(default_install.to_str().unwrap())).unwrap();
        assert!(!remove_prefix(&heroic, "C").unwrap());
        assert!(default_install.exists());
    }

    #[test]
    fn partial_downloads_are_removed_only_for_plain_folder_names() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Control");
        std::fs::create_dir_all(&game).unwrap();
        assert!(!remove_partial_download(dir.path().to_str().unwrap(), "../x"));
        assert!(!remove_partial_download(dir.path().to_str().unwrap(), ""));
        assert!(game.exists());
        assert!(remove_partial_download(dir.path().to_str().unwrap(), "Control"));
        assert!(!game.exists());
    }

    #[test]
    fn wine_choices_are_written_to_the_game_config() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(dir.path().join("h"));
        let wine = json!({"bin": "/p/proton", "name": "GE", "type": "proton"});
        write_wine_config(&heroic, "Calluna", "/pfx/Control", Some(&wine)).unwrap();
        let s = game_config::game_overrides(&heroic, "Calluna");
        assert_eq!(s["winePrefix"], "/pfx/Control");
        assert_eq!(s["wineVersion"]["name"], "GE");
        assert_eq!(s["wineCrossoverBottle"], "");
        let raw = std::fs::read_to_string(heroic.game_config("Calluna")).unwrap();
        assert!(raw.contains("\n  \"Calluna\""), "2-space indent expected");
    }
}
