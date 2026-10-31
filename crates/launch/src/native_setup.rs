//! The first launch of a store game that runs natively (Windows builds on
//! Windows): the store's installers for the game (GOG's redistributables and
//! install scripts, Amazon's `fuel.json` `PostInstall`) run once on this PC,
//! as they are, with no Wine prefix.
//!
//! [`crate::first_launch`] sends here a plan's [`FirstLaunch`] that has no
//! Wine settings. The steps run in order: the plan's own
//! ([`FirstLaunch::post_install`]), then the ones [`StoreSetup`] gives.
//! Each step is started like any program; one Windows refuses to start
//! without administrator rights (error 740) is started again through the
//! administrator prompt ([`heronite_platform::shell::run_elevated`]), and so
//! is one marked [`SetupStep::elevate`].
//!
//! The game is recorded as set up in `<data>/store_setup/installed_games.json`
//! (`{"<runner>:<app>": "<install folder>"}`) once every step ran: an
//! installer that ends with an error code is only a warning (they often do
//! when what they install is already there), while a step that could not
//! start, a declined administrator prompt or a store that could not list its
//! installers leaves the game unrecorded, so the setup runs again at the next
//! launch. A game installed again in another folder is set up again.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use heronite_data::{json_store, DataPaths};
use heronite_i18n::tr;
use heronite_platform::shell::ElevatedError;
use serde_json::{json, Value};

use crate::first_launch::StoreSetup;
use crate::plan::{Command, FirstLaunch, SetupStep};
use crate::prepare::{Flow, Notice, Prep};

/// How long one installer may take.
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long an installer stopped by a cancel gets to exit before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// `ERROR_ELEVATION_REQUIRED`: the program's manifest asks for administrator rights.
const ELEVATION_REQUIRED: i32 = 740;

/// Runs a program as administrator (Windows asks the user first) and waits
/// for it. A trait so tests need no prompt.
pub trait Elevate: Send + Sync {
    fn run(&self, program: &str, args: &[String], cwd: Option<&str>) -> Result<u32, ElevatedError>;
}

/// The system's administrator prompt.
pub struct SystemElevate;

impl Elevate for SystemElevate {
    fn run(&self, program: &str, args: &[String], cwd: Option<&str>) -> Result<u32, ElevatedError> {
        heronite_platform::shell::run_elevated(Path::new(program), args, cwd.map(Path::new))
    }
}

fn marker_file(paths: &DataPaths) -> PathBuf {
    paths.root.join("store_setup").join("installed_games.json")
}

fn key(runner: &str, app_name: &str) -> String {
    format!("{runner}:{app_name}")
}

/// Whether `app_name` of `runner` was already set up in `install_path`.
pub fn is_recorded(paths: &DataPaths, runner: &str, app_name: &str, install_path: &str) -> bool {
    let doc = json_store::read_value(&marker_file(paths)).unwrap_or(Value::Null);
    doc.get(key(runner, app_name)).and_then(Value::as_str).is_some_and(|p| same_folder(p, install_path))
}

fn same_folder(a: &str, b: &str) -> bool {
    let trim = |s: &str| s.trim_end_matches(['/', '\\']).replace('/', "\\").to_lowercase();
    trim(a) == trim(b)
}

/// Records that the game was set up in `install_path`.
pub fn record(paths: &DataPaths, runner: &str, app_name: &str, install_path: &str) -> anyhow::Result<()> {
    json_store::update_value(&marker_file(paths), |doc| {
        if let Some(obj) = doc.as_object_mut() {
            obj.insert(key(runner, app_name), json!(install_path));
        }
        Ok(())
    })
}

/// Forgets that the game was set up (it was uninstalled): the next install
/// runs the setup again.
pub fn forget(paths: &DataPaths, runner: &str, app_name: &str) {
    let file = marker_file(paths);
    if !file.exists() {
        return;
    }
    let res = json_store::update_value(&file, |doc| {
        if let Some(obj) = doc.as_object_mut() {
            obj.remove(&key(runner, app_name));
        }
        Ok(())
    });
    if let Err(e) = res {
        log::warn!("{}: {e:#}", file.display());
    }
}

/// How one step ended.
#[derive(Debug, PartialEq, Eq)]
enum Ran {
    /// It ran to its end with this exit code.
    Code(i32),
    /// It could not start, or the administrator prompt was declined.
    NotRun,
    Cancelled,
}

impl Prep<'_> {
    /// The store setup of a game that runs without Wine (see the module docs).
    pub fn native_first_launch(&self, fl: &FirstLaunch, store: Option<&dyn StoreSetup>) -> Flow {
        self.native_first_launch_with(fl, store, &SystemElevate)
    }

    pub(crate) fn native_first_launch_with(&self, fl: &FirstLaunch, store: Option<&dyn StoreSetup>, elevate: &dyn Elevate) -> Flow {
        if fl.install_path.is_empty() || is_recorded(self.paths, &fl.runner, &fl.app_name, &fl.install_path) {
            return Flow::Go;
        }
        let mut incomplete = false;
        let mut steps = fl.post_install.clone();
        if let Some(store) = store {
            // gogdl may download the redistributables first: it can take a while.
            if fl.runner == "gog" {
                self.notice(Notice::info("Getting the game's installers...", tr!("Getting the game's installers...")));
            }
            match store.steps(&fl.runner, &fl.app_name, &fl.install_path) {
                Ok(more) => steps.extend(more),
                Err(e) => {
                    self.notice(Notice::warn(
                        format!("Could not prepare the game's installers: {e}"),
                        tr!("Could not prepare the game's installers: {error}", error = e),
                    ));
                    incomplete = true;
                }
            }
        }
        if (self.cancel)() {
            return Flow::Cancelled;
        }
        if !steps.is_empty() {
            self.notice(Notice::info("Installing the game's requirements...", tr!("Installing the game's requirements...")));
        }
        for step in steps {
            if (self.cancel)() {
                return Flow::Cancelled;
            }
            log::info!("first-launch setup of {}: {:?}", fl.app_name, step.parts);
            let cwd = step.cwd.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| fl.install_path.clone());
            match self.run_step(&step, &cwd, elevate) {
                Ran::Code(0) => {}
                Ran::Code(code) => log::warn!("{:?} ended with exit code {code}", step.parts.first()),
                Ran::NotRun => incomplete = true,
                Ran::Cancelled => return Flow::Cancelled,
            }
        }
        if incomplete {
            log::warn!("the setup of {} was incomplete: it runs again at the next launch", fl.app_name);
        } else if let Err(e) = record(self.paths, &fl.runner, &fl.app_name, &fl.install_path) {
            log::warn!("recording the setup of {}: {e:#}", fl.app_name);
        }
        Flow::Go
    }

    fn run_step(&self, step: &SetupStep, cwd: &str, elevate: &dyn Elevate) -> Ran {
        let Some((program, args)) = step.parts.split_first() else { return Ran::Code(0) };
        if step.elevate {
            return self.run_elevated(elevate, program, args, cwd);
        }
        let cmd = Command { program: program.clone(), args: args.to_vec(), env: Default::default(), cwd: Some(cwd.to_string()) };
        let mut handle = match self.spawner.spawn(&cmd, None) {
            Ok(h) => h,
            Err(e) if e.raw_os_error() == Some(ELEVATION_REQUIRED) => return self.run_elevated(elevate, program, args, cwd),
            Err(e) => {
                self.notice(Notice::warn(
                    format!("Could not run {program}: {e}"),
                    tr!("Could not run {program}: {error}", program = program.as_str(), error = e),
                ));
                return Ran::NotRun;
            }
        };
        let deadline = Instant::now() + INSTALLER_TIMEOUT;
        loop {
            match handle.poll() {
                Ok(None) => {}
                Ok(Some(info)) => return Ran::Code(info.code.unwrap_or(-1)),
                Err(e) => {
                    log::warn!("{program}: {e}");
                    return Ran::NotRun;
                }
            }
            let cancelled = (self.cancel)();
            if cancelled || Instant::now() > deadline {
                handle.terminate();
                let grace = Instant::now() + STOP_GRACE;
                while Instant::now() < grace && matches!(handle.poll(), Ok(None)) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                handle.kill();
                return if cancelled { Ran::Cancelled } else { Ran::NotRun };
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Through the administrator prompt. Blocking: the prompt cannot be
    /// cancelled from here.
    fn run_elevated(&self, elevate: &dyn Elevate, program: &str, args: &[String], cwd: &str) -> Ran {
        let name = Path::new(program).file_name().map_or(program.to_string(), |n| n.to_string_lossy().into_owned());
        self.notice(
            Notice::info(
                format!("{name} needs administrator rights: answer the Windows prompt"),
                tr!("{program} needs administrator rights: answer the Windows prompt", program = name.as_str()),
            )
            .uncancellable(),
        );
        match elevate.run(program, args, Some(cwd)) {
            Ok(code) => Ran::Code(code as i32),
            Err(ElevatedError::Declined) => {
                self.notice(Notice::warn(
                    format!("{name} was not installed: the administrator prompt was declined. It is asked again at the next launch."),
                    tr!(
                        "{program} was not installed: the administrator prompt was declined. It is asked again at the next launch.",
                        program = name.as_str()
                    ),
                ));
                Ran::NotRun
            }
            Err(ElevatedError::Failed(e)) => {
                self.notice(Notice::warn(
                    format!("Could not run {program}: {e}"),
                    tr!("Could not run {program}: {error}", program = program, error = e),
                ));
                Ran::NotRun
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::prepare::tests::InstantSpawner;
    use crate::probe::FakeProbe;
    use crate::runtime::{ProcessHandle, Spawner};
    use std::sync::Mutex;

    /// Spawns nothing; programs named in `needs_admin` fail with error 740.
    #[derive(Default)]
    struct Fx {
        inner: InstantSpawner,
        needs_admin: Vec<String>,
        missing: Vec<String>,
    }

    impl Spawner for Fx {
        fn spawn(&self, cmd: &Command, log: Option<&Path>) -> std::io::Result<Box<dyn ProcessHandle>> {
            if self.needs_admin.contains(&cmd.program) {
                return Err(std::io::Error::from_raw_os_error(ELEVATION_REQUIRED));
            }
            if self.missing.contains(&cmd.program) {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not found"));
            }
            self.inner.spawn(cmd, log)
        }
        fn open_uri(&self, _: &str) -> std::io::Result<()> {
            Ok(())
        }
        fn kill_pattern(&self, _: &str, _: u64) {}
    }

    #[derive(Default)]
    struct FakeElevate {
        calls: Mutex<Vec<(String, Vec<String>, Option<String>)>>,
        decline: bool,
    }

    impl Elevate for FakeElevate {
        fn run(&self, program: &str, args: &[String], cwd: Option<&str>) -> Result<u32, ElevatedError> {
            self.calls.lock().unwrap().push((program.into(), args.to_vec(), cwd.map(str::to_string)));
            if self.decline {
                Err(ElevatedError::Declined)
            } else {
                Ok(0)
            }
        }
    }

    struct Store(Result<Vec<SetupStep>, String>);

    impl StoreSetup for Store {
        fn steps(&self, runner: &str, app_name: &str, install_path: &str) -> Result<Vec<SetupStep>, String> {
            assert_eq!((runner, app_name, install_path), ("gog", "42", r"C:\Games\The Game"));
            self.0.clone()
        }
    }

    fn step(parts: &[&str]) -> SetupStep {
        SetupStep { parts: parts.iter().map(|p| p.to_string()).collect(), ..Default::default() }
    }

    fn fl() -> FirstLaunch {
        FirstLaunch { runner: "gog".into(), app_name: "42".into(), install_path: r"C:\Games\The Game".into(), ..Default::default() }
    }

    fn run(
        paths: &DataPaths,
        spawner: &Fx,
        fl: &FirstLaunch,
        store: Option<&dyn StoreSetup>,
        elevate: &FakeElevate,
    ) -> (Flow, Vec<Notice>) {
        let seen = Mutex::new(Vec::new());
        let sink = |n: Notice| seen.lock().unwrap().push(n);
        let (net, probe) = (FakeNet::offline(), FakeProbe::new("/h"));
        let prep = Prep { paths, probe: &probe, net: &net, spawner, cloud: None, cancel: &|| false, notify: &sink };
        let flow = prep.native_first_launch_with(fl, store, elevate);
        (flow, seen.into_inner().unwrap())
    }

    #[test]
    fn the_plan_steps_then_the_store_steps_run_once_in_the_install_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let spawner = Fx::default();
        let mut first = fl();
        first.post_install = vec![step(&[r"C:\Games\The Game\redist\vc.exe", "/quiet"])];
        let store = Store(Ok(vec![
            step(&["msiexec", "/i", r"C:\r\physx.msi", "/qb"]),
            SetupStep { cwd: Some(r"C:\other".into()), ..step(&[r"C:\r\isi.exe", "/VERYSILENT"]) },
        ]));
        let elevate = FakeElevate::default();
        let (flow, notices) = run(&paths, &spawner, &first, Some(&store), &elevate);
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().all(|n| !n.warning), "{notices:?}");
        let spawned = spawner.inner.spawned.lock().unwrap().clone();
        let programs: Vec<&str> = spawned.iter().map(|c| c.program.as_str()).collect();
        assert_eq!(programs, [r"C:\Games\The Game\redist\vc.exe", "msiexec", r"C:\r\isi.exe"]);
        assert_eq!(spawned[1].args, ["/i", r"C:\r\physx.msi", "/qb"]);
        assert_eq!(spawned[0].cwd.as_deref(), Some(r"C:\Games\The Game"));
        assert_eq!(spawned[2].cwd.as_deref(), Some(r"C:\other"));
        assert!(elevate.calls.lock().unwrap().is_empty());
        assert!(is_recorded(&paths, "gog", "42", r"C:\Games\The Game\"));

        // The second launch does nothing.
        spawner.inner.spawned.lock().unwrap().clear();
        let (_, notices) = run(&paths, &spawner, &first, Some(&store), &elevate);
        assert!(notices.is_empty() && spawner.inner.spawned.lock().unwrap().is_empty());
        // Installed again elsewhere: set up again. Uninstalled: forgotten.
        assert!(!is_recorded(&paths, "gog", "42", r"D:\Games\The Game"));
        assert!(!is_recorded(&paths, "nile", "42", r"C:\Games\The Game"));
        forget(&paths, "gog", "42");
        assert!(!is_recorded(&paths, "gog", "42", r"C:\Games\The Game"));
    }

    #[test]
    fn programs_that_need_administrator_rights_go_through_the_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let spawner = Fx { needs_admin: vec![r"C:\r\isi.exe".into()], ..Default::default() };
        let store = Store(Ok(vec![
            step(&[r"C:\r\isi.exe", "/DIR=C:\\Games\\The Game"]),
            SetupStep { elevate: true, ..step(&[r"C:\r\dxsetup.exe", "/silent"]) },
        ]));
        let elevate = FakeElevate::default();
        let (flow, notices) = run(&paths, &spawner, &fl(), Some(&store), &elevate);
        assert_eq!(flow, Flow::Go);
        let calls = elevate.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], (r"C:\r\isi.exe".into(), vec!["/DIR=C:\\Games\\The Game".into()], Some(r"C:\Games\The Game".into())));
        assert_eq!(calls[1].0, r"C:\r\dxsetup.exe");
        assert!(spawner.inner.spawned.lock().unwrap().is_empty(), "a step marked elevate is not tried first");
        assert!(notices.iter().any(|n| n.log.contains("isi.exe needs administrator rights")), "{notices:?}");
        assert!(is_recorded(&paths, "gog", "42", r"C:\Games\The Game"));
    }

    #[test]
    fn a_declined_prompt_is_a_warning_and_the_setup_runs_again_next_time() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let spawner = Fx { needs_admin: vec!["vc.exe".into()], ..Default::default() };
        let store = Store(Ok(vec![step(&["vc.exe"]), step(&["next.exe"])]));
        let elevate = FakeElevate { decline: true, ..Default::default() };
        let (flow, notices) = run(&paths, &spawner, &fl(), Some(&store), &elevate);
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().any(|n| n.warning && n.log.contains("declined")), "{notices:?}");
        // The rest still runs; the game is not recorded.
        assert_eq!(spawner.inner.spawned.lock().unwrap()[0].program, "next.exe");
        assert!(!is_recorded(&paths, "gog", "42", r"C:\Games\The Game"));
    }

    #[test]
    fn failures_to_start_or_list_retry_but_error_codes_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let spawner = Fx { missing: vec!["gone.exe".into()], ..Default::default() };
        let (_, notices) = run(&paths, &spawner, &fl(), Some(&Store(Ok(vec![step(&["gone.exe"])]))), &FakeElevate::default());
        assert!(notices.iter().any(|n| n.warning && n.log.contains("gone.exe")));
        assert!(!is_recorded(&paths, "gog", "42", r"C:\Games\The Game"));

        let (_, notices) = run(&paths, &Fx::default(), &fl(), Some(&Store(Err("offline".into()))), &FakeElevate::default());
        assert!(notices.iter().any(|n| n.warning && n.log.contains("offline")));
        assert!(!is_recorded(&paths, "gog", "42", r"C:\Games\The Game"));

        let failing = Fx { inner: InstantSpawner { exit_code: 1638, ..Default::default() }, ..Default::default() };
        let (flow, _) = run(&paths, &failing, &fl(), Some(&Store(Ok(vec![step(&["vc.exe"])]))), &FakeElevate::default());
        assert_eq!(flow, Flow::Go);
        assert!(is_recorded(&paths, "gog", "42", r"C:\Games\The Game"));
    }

    #[test]
    fn cancelling_stops_before_the_next_step() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let spawner = Fx::default();
        let (net, probe) = (FakeNet::offline(), FakeProbe::new("/h"));
        let sink = |_: Notice| {};
        let prep = Prep { paths: &paths, probe: &probe, net: &net, spawner: &spawner, cloud: None, cancel: &|| true, notify: &sink };
        let store = Store(Ok(vec![step(&["vc.exe"])]));
        assert_eq!(prep.native_first_launch_with(&fl(), Some(&store), &FakeElevate::default()), Flow::Cancelled);
        assert!(spawner.inner.spawned.lock().unwrap().is_empty());
        assert!(!is_recorded(&paths, "gog", "42", r"C:\Games\The Game"));
    }
}
