//! Game operations through nile: install and update as download queue jobs,
//! and the one-off verify, uninstall and import. Moving an install lives in
//! [`crate::moves`].
//!
//! nile reports many failures only in its log (`ERROR [NAME]:\t message`) and
//! still exits 0, so success is also checked against `installed.json`.

use std::sync::OnceLock;

use anyhow::{bail, Result};
use heronite_data::DataPaths;
use heronite_epic::cmd::Output;
use heronite_epic::manager::{Job, JobResult};
use heronite_epic::ops::DownloadSettings;
use heronite_epic::progress::{Progress, ToolProgressParser};
use heronite_epic::queue::{DmElement, ElementKind};
use heronite_i18n::tr;
use regex::Regex;

use crate::cmd::{Cancel, Nile};
use crate::{installed, library, updates};

pub use crate::moves::{move_install, relocate};

/// Install and update jobs of the download queue.
pub struct NileJob {
    pub paths: DataPaths,
    pub nile: Nile,
}

/// Folder nile creates inside the base path of an install: the title without
/// the characters Windows file names forbid, or the product id.
pub fn install_folder_name(title: Option<&str>, app_name: &str) -> String {
    let clean: String = title.unwrap_or_default().chars().filter(|c| !":/*?<>\\|™\"®".contains(*c)).collect();
    if clean.is_empty() {
        app_name.to_string()
    } else {
        clean
    }
}

/// `nile install` arguments; `max_workers == 0` leaves nile's default.
fn install_args(app_name: &str, base_path: &str, max_workers: u32) -> Vec<String> {
    let mut args: Vec<String> = ["install", "--base-path", base_path, app_name].map(String::from).into();
    push_workers(&mut args, max_workers);
    args
}

/// `nile update` arguments. `--path` is always given: without it nile can
/// look the game up in the wrong folder.
fn update_args(app_name: &str, install_path: &str, max_workers: u32) -> Vec<String> {
    let mut args: Vec<String> = ["update", "--path", install_path, app_name].map(String::from).into();
    push_workers(&mut args, max_workers);
    args
}

fn push_workers(args: &mut Vec<String>, max_workers: u32) {
    if max_workers > 0 {
        args.extend(["--max-workers".into(), max_workers.to_string()]);
    }
}

/// Whether nile logged an error, whatever its exit code says.
fn logged_error(out: &Output) -> bool {
    out.stderr.lines().chain(out.stdout.lines()).any(|l| l.starts_with("ERROR ") || l.starts_with("CRITICAL "))
}

fn log_prefix() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?:ERROR|CRITICAL) \[[^\]]*\]:\s*").expect("valid regex"))
}

/// The error lines of the output, without nile's `ERROR [NAME]:` prefix.
fn error_text(out: &Output) -> String {
    out.error_lines().lines().map(|l| log_prefix().replace(l, "").into_owned()).collect::<Vec<_>>().join("\n")
}

fn exit_message(out: &Output) -> String {
    tr!("nile exited with {code}", code = format!("{:?}", out.code))
}

impl NileJob {
    fn args(&self, el: &DmElement) -> Result<Vec<String>, String> {
        let p = &el.params;
        let workers = DownloadSettings::load(&self.paths).max_workers;
        match el.kind {
            ElementKind::Install => Ok(install_args(&p.app_name, &p.path, workers)),
            ElementKind::Update => {
                let path =
                    Some(p.path.clone()).filter(|p| !p.is_empty()).or_else(|| installed::get(&self.paths, &p.app_name).map(|g| g.path));
                match path.filter(|p| !p.is_empty()) {
                    Some(path) => Ok(update_args(&p.app_name, &path, workers)),
                    None => Err(tr!("The game is not installed").to_string()),
                }
            }
        }
    }
}

impl Job for NileJob {
    fn run(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        let app_name = &el.params.app_name;
        let args = match self.args(el) {
            Ok(args) => args,
            Err(message) => return JobResult::error(message),
        };
        let mut parser = ToolProgressParser::new();
        let out = match self.nile.run_streaming(&args, cancel, |line| {
            if let Some(progress) = parser.feed(line) {
                on_progress(progress);
            }
        }) {
            Ok(out) => out,
            Err(err) => return JobResult::error(format!("{err:#}")),
        };
        if out.aborted {
            return JobResult::abort();
        }
        if format!("{}\n{}", out.stderr, out.stdout).contains("Not enough space available") {
            return JobResult::error(tr!("Not enough disk space to install this game"));
        }
        let worked = out.success()
            && match el.kind {
                ElementKind::Install => installed::get(&self.paths, app_name).is_some(),
                ElementKind::Update => !logged_error(&out),
            };
        if !worked {
            let why = error_text(&out);
            return JobResult::error(if !why.is_empty() {
                why
            } else if out.success() {
                tr!("nile finished, but the game was not installed").to_string()
            } else {
                exit_message(&out)
            });
        }
        if let Err(err) = library::write_library(&self.paths) {
            log::warn!("rebuilding the Amazon library: {err:#}");
        }
        updates::forget(&self.paths, app_name);
        JobResult::done()
    }
}

/// Errors nile logged before a Python traceback, which it can print at the
/// very end of a run that worked.
fn errors_before_traceback(out: &Output) -> Vec<String> {
    out.stderr
        .lines()
        .take_while(|l| !l.starts_with("Traceback"))
        .chain(out.stdout.lines().take_while(|l| !l.starts_with("Traceback")))
        .filter(|l| l.starts_with("ERROR ") || l.starts_with("CRITICAL "))
        .map(|l| log_prefix().replace(l, "").into_owned())
        .collect()
}

/// `nile verify`: checks the files and downloads what is missing or damaged.
/// `on_percent` gets `0..=100`.
pub fn verify(paths: &DataPaths, nile: &Nile, app_name: &str, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<()> {
    let Some(game) = installed::get(paths, app_name) else { bail!(tr!("The game is not installed")) };
    let mut parser = ToolProgressParser::new();
    // Something was checked: a progress report, or nile said it finished.
    let mut checked = false;
    let out = nile.run_streaming(&["verify", "--path", game.path.as_str(), app_name], cancel, |line| {
        if let Some(progress) = parser.feed(line) {
            checked = true;
            on_percent(progress.percent);
        }
        checked |= line.contains("Game is up to date") || line.contains("Download complete");
    })?;
    if out.aborted {
        bail!(tr!("The verification was cancelled"));
    }
    let errors = errors_before_traceback(&out);
    if !errors.is_empty() {
        bail!(errors.join("\n"));
    }
    if !out.success() && !checked {
        let why = error_text(&out);
        bail!(if why.is_empty() { exit_message(&out) } else { why });
    }
    on_percent(100.);
    // The size on disk may have changed.
    if let Err(err) = library::write_library(paths) {
        log::warn!("rebuilding the Amazon library: {err:#}");
    }
    Ok(())
}

/// `nile uninstall`: nile deletes the files of the game's manifest and its
/// entry. When it cannot, the entry (and the stored manifest, so a new install
/// downloads everything) is removed here; folders are never deleted here.
/// Then `Some(problem)` tells the user what nile said and where the game's
/// files may still be.
pub fn uninstall(paths: &DataPaths, nile: &Nile, app_name: &str) -> Result<Option<String>> {
    let folder = installed::get(paths, app_name).map(|g| g.path).unwrap_or_default();
    let out = nile.run(&["uninstall", app_name])?;
    let mut problem = None;
    if !out.success() || installed::get(paths, app_name).is_some() {
        let why = error_text(&out);
        log::warn!("nile could not uninstall {app_name} ({why}): removing its entry");
        installed::remove(paths, app_name)?;
        remove_manifest(nile, app_name);
        let why = match why.lines().rev().find(|l| !l.trim().is_empty()) {
            Some(last) => last.trim().to_string(),
            None if !out.success() => exit_message(&out),
            None => String::new(),
        };
        problem = Some(match (why.is_empty(), folder.is_empty()) {
            (false, false) => tr!("nile could not delete the game's files ({why}): they may still be in {folder}", why, folder),
            (false, true) => tr!("nile could not delete the game's files ({why})", why),
            (true, false) => tr!("nile could not delete the game's files: they may still be in {folder}", folder),
            (true, true) => tr!("nile could not delete the game's files").to_string(),
        });
    }
    if let Err(err) = library::write_library(paths) {
        log::warn!("rebuilding the Amazon library: {err:#}");
    }
    updates::forget(paths, app_name);
    Ok(problem)
}

/// Takes the game off the installed list (and drops nile's manifest of it)
/// without touching any folder: for a folder deleted or moved by hand.
pub fn forget(paths: &DataPaths, nile: &Nile, app_name: &str) -> Result<()> {
    installed::remove(paths, app_name)?;
    remove_manifest(nile, app_name);
    if let Err(err) = library::write_library(paths) {
        log::warn!("rebuilding the Amazon library: {err:#}");
    }
    updates::forget(paths, app_name);
    Ok(())
}

/// nile's copy of the installed manifest.
fn remove_manifest(nile: &Nile, app_name: &str) {
    for ext in ["raw", "json"] {
        let file = nile.data_dir().join("manifests").join(format!("{app_name}.{ext}"));
        if file.is_file() {
            if let Err(err) = std::fs::remove_file(&file) {
                log::warn!("removing {}: {err}", file.display());
            }
        }
    }
}

/// `nile import`: registers a game already on disk after nile checked its
/// files. The error is nile's reason.
pub fn import(paths: &DataPaths, nile: &Nile, app_name: &str, folder: &str) -> Result<()> {
    let out = nile.run(&["import", "--path", folder, app_name])?;
    if installed::get(paths, app_name).is_none() {
        let reasons: Vec<String> = out
            .stderr
            .lines()
            .chain(out.stdout.lines())
            .filter_map(|l| l.strip_prefix("ERROR [IMPORT]:").map(|r| r.trim().to_string()))
            .filter(|r| !r.is_empty())
            .collect();
        if !reasons.is_empty() {
            bail!(reasons.join("\n"));
        }
        let why = error_text(&out);
        bail!(if !why.is_empty() {
            why
        } else if out.success() {
            tr!("nile did not import the game").to_string()
        } else {
            exit_message(&out)
        });
    }
    if let Err(err) = library::write_library(paths) {
        log::warn!("rebuilding the Amazon library: {err:#}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::Env;
    use heronite_data::json_store;
    use heronite_epic::queue::InstallParams;
    use serde_json::json;
    use std::time::Duration;

    const ID: &str = "amzn1.adg.product.AAA";
    const LIBRARY: &str = r#"[{"id": "e1", "product": {"id": "amzn1.adg.product.AAA", "title": "Cool Game"}}]"#;
    const REPORT: &str = "echo 'INFO [PROGRESS]:\t = Progress: 50.00 50/100, Running for: 00:00:05, ETA: 00:00:05' >&2; \
         echo 'INFO [PROGRESS]:\t = Downloaded: 10.00 MiB, Written: 10.00 MiB' >&2; \
         printf 'INFO [PROGRESS]:\\t + Download\\t- 2.00 MiB/s\\n' >&2; \
         printf 'INFO [PROGRESS]:\\t + Disk\\t- 3.00 MiB/s\\n' >&2";
    /// What nile's install leaves behind.
    const WRITE_INSTALLED: &str = "mkdir -p \"$NILE_CONFIG_PATH/nile\"; \
         echo '[{\"id\": \"amzn1.adg.product.AAA\", \"version\": \"v2\", \"path\": \"/g/Cool Game\", \"size\": 1024}]' > \"$NILE_CONFIG_PATH/nile/installed.json\"";

    fn element(kind: ElementKind, path: &str) -> DmElement {
        DmElement::new(kind, InstallParams { app_name: ID.into(), path: path.into(), runner: "nile".into(), ..Default::default() })
    }

    fn make_job(env: &Env, body: &str) -> NileJob {
        NileJob { paths: env.paths.clone(), nile: env.nile(body) }
    }

    fn run(job: &NileJob, el: &DmElement) -> (JobResult, Vec<Progress>) {
        let mut reports = Vec::new();
        let result = job.run(el, &Cancel::new(), &mut |p| reports.push(p));
        (result, reports)
    }

    #[test]
    fn folder_names_drop_forbidden_characters() {
        assert_eq!(install_folder_name(Some("Cool: Game™ \"2\"®"), "id"), "Cool Game 2");
        assert_eq!(install_folder_name(Some("A/B\\C|D"), "id"), "ABCD");
        assert_eq!(install_folder_name(Some("::"), "id"), "id");
        assert_eq!(install_folder_name(None, "id"), "id");
    }

    #[test]
    fn install_runs_nile_reports_progress_and_needs_the_installed_entry() {
        let env = Env::new();
        env.write_nile_file("library.json", LIBRARY);
        let job = make_job(&env, &format!("{REPORT}; {WRITE_INSTALLED}"));
        let (result, reports) = run(&job, &element(ElementKind::Install, "/games"));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Done, "{:?}", result.error);
        assert_eq!(env.calls(), [format!("install --base-path /games {ID}")]);
        assert_eq!(reports.len(), 1);
        assert_eq!((reports[0].percent, reports[0].eta.as_str(), reports[0].down_speed, reports[0].disk_speed), (50., "00:00:05", 2., 3.));
        // The library was rebuilt with the game installed.
        let game = library::game_info(&env.paths, ID).unwrap();
        assert_eq!(game["is_installed"], true);
        assert_eq!(game["install"]["install_path"], "/g/Cool Game");
    }

    #[test]
    fn install_passes_the_worker_limit() {
        let env = Env::new();
        std::fs::create_dir_all(env.paths.global_config().parent().unwrap()).unwrap();
        std::fs::write(env.paths.global_config(), r#"{"defaultSettings": {"maxWorkers": 4}}"#).unwrap();
        let job = make_job(&env, WRITE_INSTALLED);
        let (result, _) = run(&job, &element(ElementKind::Install, "/games"));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Done);
        assert_eq!(env.calls(), [format!("install --base-path /games {ID} --max-workers 4")]);
    }

    #[test]
    fn install_without_the_installed_entry_is_a_failure_even_with_exit_zero() {
        let env = Env::new();
        let job = make_job(&env, "echo 'ERROR [DOWNLOAD]:\t Unable to load manifest' >&2");
        let (result, _) = run(&job, &element(ElementKind::Install, "/games"));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Error);
        assert_eq!(result.error.as_deref(), Some("Unable to load manifest"));

        let job = make_job(&env, "true");
        let (result, _) = run(&job, &element(ElementKind::Install, "/games"));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Error);
        assert!(result.error.unwrap().contains("not installed"));
    }

    #[test]
    fn not_enough_space_is_reported() {
        let env = Env::new();
        let job = make_job(&env, "echo 'ERROR [DOWNLOAD]:\t Not enough space available' >&2");
        let (result, _) = run(&job, &element(ElementKind::Install, "/games"));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Error);
        assert!(result.error.unwrap().contains("Not enough disk space"));
    }

    #[test]
    fn a_nonzero_exit_without_error_lines_says_so() {
        let env = Env::new();
        let job = make_job(&env, "exit 7");
        let (result, _) = run(&job, &element(ElementKind::Install, "/games"));
        assert_eq!(result.error.as_deref(), Some("nile exited with Some(7)"));
    }

    #[test]
    fn cancelling_aborts_the_install() {
        let env = Env::new();
        let job = make_job(&env, "echo started; sleep 60 & wait");
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c2.request();
        });
        let result = job.run(&element(ElementKind::Install, "/games"), &cancel, &mut |_| {});
        t.join().unwrap();
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Abort);
        assert!(result.error.is_none());
    }

    #[test]
    fn update_always_passes_the_install_path_and_clears_the_pending_update() {
        let env = Env::new();
        env.write_nile_file("library.json", LIBRARY);
        env.write_nile_file("installed.json", &format!(r#"[{{"id": "{ID}", "version": "v1", "path": "/g/Cool Game"}}]"#));
        json_store::write(&env.paths.nile_store("updates"), &json!({"updates": [ID, "other"]})).unwrap();
        let job = make_job(&env, WRITE_INSTALLED);
        // The queue element has the install folder...
        let (result, _) = run(&job, &element(ElementKind::Update, "/g/Cool Game"));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Done, "{:?}", result.error);
        // ...or nile's own record supplies it.
        let (result, _) = run(&job, &element(ElementKind::Update, ""));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Done);
        assert_eq!(env.calls(), [format!("update --path /g/Cool Game {ID}"), format!("update --path /g/Cool Game {ID}")]);
        assert_eq!(updates::cached_updates(&env.paths), ["other"]);
        assert_eq!(library::game_info(&env.paths, ID).unwrap()["install"]["version"], "v2");
    }

    #[test]
    fn update_with_a_logged_error_fails_even_with_exit_zero() {
        let env = Env::new();
        let job = make_job(&env, "echo 'ERROR [DOWNLOAD]:\t Unable to load manifest' >&2");
        let (result, _) = run(&job, &element(ElementKind::Update, "/g"));
        assert_eq!(result.status, heronite_epic::queue::DmStatus::Error);
        // An update of a game nile does not know has no folder to give.
        let (result, _) = run(&job, &element(ElementKind::Update, ""));
        assert_eq!(result.error.as_deref(), Some("The game is not installed"));
    }

    fn installed_game(env: &Env) {
        env.write_nile_file("library.json", LIBRARY);
        env.write_nile_file("installed.json", &format!(r#"[{{"id": "{ID}", "version": "v1", "path": "/g/Cool Game"}}]"#));
    }

    #[test]
    fn verify_reports_percent_and_runs_with_the_install_path() {
        let env = Env::new();
        installed_game(&env);
        let nile = env.nile(REPORT);
        let mut seen = Vec::new();
        verify(&env.paths, &nile, ID, &Cancel::new(), &mut |p| seen.push(p)).unwrap();
        assert_eq!(env.calls(), [format!("verify --path /g/Cool Game {ID}")]);
        assert_eq!(seen, [50., 100.]);
    }

    #[test]
    fn verify_tolerates_a_traceback_at_the_end_of_a_run_that_worked() {
        let env = Env::new();
        installed_game(&env);
        let nile = env.nile(&format!(
            "{REPORT}; echo 'INFO [CLI]:\t Download complete' >&2; echo 'Traceback (most recent call last):' >&2; \
             echo 'ERROR in the traceback is not counted' >&2; exit 1"
        ));
        verify(&env.paths, &nile, ID, &Cancel::new(), &mut |_| {}).unwrap();
    }

    #[test]
    fn verify_fails_on_errors_before_the_traceback_or_when_nothing_was_checked() {
        let env = Env::new();
        installed_game(&env);
        let nile =
            env.nile("echo 'ERROR [DOWNLOAD]:\t Unable to load manifest' >&2; echo 'Traceback (most recent call last):' >&2; exit 1");
        let err = verify(&env.paths, &nile, ID, &Cancel::new(), &mut |_| {}).unwrap_err().to_string();
        assert_eq!(err, "Unable to load manifest");
        let nile = env.nile("echo 'Traceback (most recent call last):' >&2; exit 1");
        assert!(verify(&env.paths, &nile, ID, &Cancel::new(), &mut |_| {}).is_err());
        // A game nile does not know is not run at all.
        let calls = env.calls().len();
        assert!(verify(&env.paths, &nile, "other", &Cancel::new(), &mut |_| {}).is_err());
        assert_eq!(env.calls().len(), calls);
    }

    #[test]
    fn uninstall_runs_nile_and_refreshes_the_library() {
        let env = Env::new();
        installed_game(&env);
        library::write_library(&env.paths).unwrap();
        json_store::write(&env.paths.nile_store("updates"), &json!({"updates": [ID]})).unwrap();
        let nile = env.nile("echo '[]' > \"$NILE_CONFIG_PATH/nile/installed.json\"");
        assert_eq!(uninstall(&env.paths, &nile, ID).unwrap(), None);
        assert_eq!(env.calls(), [format!("uninstall {ID}")]);
        assert_eq!(library::game_info(&env.paths, ID).unwrap()["is_installed"], false);
        assert!(updates::cached_updates(&env.paths).is_empty());
    }

    #[test]
    fn uninstall_falls_back_to_dropping_the_entry_without_touching_folders() {
        let env = Env::new();
        installed_game(&env);
        let folder = env.root().join("game-folder");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("keep.txt"), "x").unwrap();
        env.write_nile_file(&format!("manifests/{ID}.raw"), "manifest");
        env.write_nile_file(
            "installed.json",
            &format!(r#"[{{"id": "{ID}", "path": "{}"}}, {{"id": "other", "path": "/o"}}]"#, folder.display()),
        );
        let nile = env.nile("echo 'Traceback (most recent call last):' >&2; exit 1");
        let problem = uninstall(&env.paths, &nile, ID).unwrap().expect("a failed nile is reported");
        assert!(problem.contains(&folder.display().to_string()), "{problem}");
        assert!(problem.contains("Traceback"), "nile's last error line is shown: {problem}");
        let left = installed::load(&env.paths);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "other");
        assert!(folder.join("keep.txt").exists());
        assert!(!env.nile_dir().join(format!("manifests/{ID}.raw")).exists());
    }

    #[test]
    fn import_passes_the_folder_and_refreshes_the_library() {
        let env = Env::new();
        env.write_nile_file("library.json", LIBRARY);
        let nile = env.nile(WRITE_INSTALLED);
        import(&env.paths, &nile, ID, "/some/folder").unwrap();
        assert_eq!(env.calls(), [format!("import --path /some/folder {ID}")]);
        assert_eq!(library::game_info(&env.paths, ID).unwrap()["is_installed"], true);
    }

    #[test]
    fn import_returns_nile_s_reason() {
        let env = Env::new();
        let nile = env.nile(
            "printf 'ERROR [IMPORT]:\\t /x/game.exe is missing or corrupted\\nERROR [IMPORT]:\\t There are missing files. Failed import.\\n' >&2",
        );
        let err = import(&env.paths, &nile, ID, "/x").unwrap_err().to_string();
        assert_eq!(err, "/x/game.exe is missing or corrupted\nThere are missing files. Failed import.");
        let nile = env.nile("echo 'ERROR [CLI]:\t No game match' >&2");
        assert_eq!(import(&env.paths, &nile, ID, "/x").unwrap_err().to_string(), "No game match");
    }
}
