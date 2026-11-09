//! The first launch of an Epic game that runs natively (Windows builds on
//! Windows): what `legendarySetup` does in a Wine prefix, the Windows way.
//!
//! * The game's prerequisites (`manifest.prerequisites` of `legendary info`:
//!   Unreal's or Visual C++'s installer) run once, through the
//!   administrator prompt. legendary itself skips them when it installs with
//!   `-y`.
//! * A game of the EA app or Ubisoft Connect is opened by the store's app:
//!   when that app is not installed, its installer (downloaded when the game
//!   was "installed", see `heronite_epic::third_party`) runs instead.
//! * No `com.epicgames.launcher` registry key (Windows does not need it),
//!   no winetricks, no known fixes, and no EOS Overlay step: on Windows the
//!   overlay is turned on for every game at once (`legendary eos-overlay
//!   enable`, from the Epic page).
//!
//! The steps run, and the game is recorded as set up, through
//! [`crate::native_setup`] (runner `legendary`): a declined administrator
//! prompt is a warning and the setup runs again at the next launch.

use std::path::Path;

use heronite_i18n::tr;

use crate::first_launch::EpicSetup;
use crate::native_setup::{Elevate, SystemElevate};
use crate::plan::{FirstLaunch, SetupStep};
use crate::prepare::{Flow, Notice, Prep};

/// The installer of the store app of an EA app / Ubisoft Connect game, its
/// silent-install arguments, and the link scheme the app registers.
fn store_app(fl: &FirstLaunch) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    if fl.ea_managed {
        Some(("EAappInstaller.exe", &["EAX_LAUNCH_CLIENT=0", "IGNORE_INSTALLED=1"], "link2ea:"))
    } else if fl.ubisoft_managed {
        Some(("UbisoftConnectInstaller.exe", &["/S"], "uplay:"))
    } else {
        None
    }
}

/// `path` under `folder` in the system's form (manifests use `\`).
fn under(folder: &str, path: &str) -> String {
    let path = if cfg!(windows) { path.replace('/', "\\") } else { path.replace('\\', "/") };
    Path::new(folder).join(path.trim_start_matches(['/', '\\'])).to_string_lossy().into_owned()
}

impl Prep<'_> {
    /// See the module docs. `epic` gives the prerequisites; `client_installed`
    /// says whether a link scheme has a program (the store app is there).
    pub fn epic_native_first_launch(&self, fl: &FirstLaunch, epic: Option<&dyn EpicSetup>) -> Flow {
        self.epic_native_first_launch_with(fl, epic, &crate::legendary_windows::link_handler_installed, &SystemElevate)
    }

    pub(crate) fn epic_native_first_launch_with(
        &self,
        fl: &FirstLaunch,
        epic: Option<&dyn EpicSetup>,
        client_installed: &dyn Fn(&str) -> bool,
        elevate: &dyn Elevate,
    ) -> Flow {
        let mut fl = fl.clone();
        let mut steps = Vec::new();
        if let Some((file, args, scheme)) = store_app(&fl) {
            let redist = self.paths.tools_dir().join("redist").join("legendary");
            // Recorded under the installer's folder: these games have none of their own.
            fl.install_path = redist.to_string_lossy().into_owned();
            if crate::native_setup::is_recorded(self.paths, &fl.runner, &fl.app_name, &fl.install_path) || client_installed(scheme) {
                return Flow::Go;
            }
            let installer = redist.join(file).to_string_lossy().into_owned();
            if !self.probe.exists(&installer) {
                let shown = if fl.ea_managed {
                    tr!("The installer of the EA App was not found: {file}", file)
                } else {
                    tr!("The installer of Ubisoft Connect was not found: {file}", file)
                };
                self.notice(Notice::warn(format!("The installer of {file} was not found"), shown));
                return Flow::Go;
            }
            let mut parts = vec![installer];
            parts.extend(args.iter().map(|a| a.to_string()));
            steps.push(SetupStep { parts, cwd: Some(fl.install_path.clone()), elevate: true });
        } else {
            if fl.install_path.is_empty() || crate::native_setup::is_recorded(self.paths, &fl.runner, &fl.app_name, &fl.install_path) {
                return Flow::Go;
            }
            let Some(epic) = epic else { return Flow::Go };
            match epic.prerequisites(&fl.app_name, &fl.platform) {
                Ok(Some(pre)) if !pre.path.is_empty() => {
                    let program = under(&fl.install_path, &pre.path);
                    let cwd = Path::new(&program).parent().map(|d| d.to_string_lossy().into_owned());
                    let mut parts = vec![program];
                    parts.extend(shlex::split(&pre.args).unwrap_or_else(|| pre.args.split_whitespace().map(str::to_string).collect()));
                    log::info!("the prerequisites of {}: {parts:?}", fl.app_name);
                    steps.push(SetupStep { parts, cwd, elevate: true });
                }
                Ok(_) => {}
                Err(e) => {
                    // Asked again at the next launch.
                    log::warn!("getInstallInfo failed with {e}");
                    return Flow::Go;
                }
            }
            if !steps.is_empty() {
                self.notice(Notice::info("Installing Redistributables (EPIC)", tr!("Installing Redistributables (EPIC)")));
            }
        }
        fl.post_install = steps;
        self.native_first_launch_with(&fl, None, elevate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_launch::Prerequisite;
    use crate::net::FakeNet;
    use crate::prepare::tests::InstantSpawner;
    use crate::probe::SystemProbe;
    use heronite_data::DataPaths;
    use std::sync::Mutex;

    struct FakeEpic {
        prereq: Result<Option<Prerequisite>, String>,
        asked: Mutex<u32>,
    }

    impl EpicSetup for FakeEpic {
        fn prerequisites(&self, _: &str, _: &str) -> Result<Option<Prerequisite>, String> {
            *self.asked.lock().unwrap() += 1;
            self.prereq.clone()
        }
        fn eos_overlay_installed(&self) -> bool {
            panic!("the overlay is global on Windows")
        }
        fn eos_overlay_enabled(&self, _: &str) -> bool {
            panic!("the overlay is global on Windows")
        }
        fn eos_overlay_enable(&self, _: &str) -> Result<(), String> {
            panic!("the overlay is global on Windows")
        }
    }

    fn epic(prereq: Result<Option<Prerequisite>, String>) -> FakeEpic {
        FakeEpic { prereq, asked: Mutex::new(0) }
    }

    fn game(dir: &Path) -> FirstLaunch {
        FirstLaunch {
            runner: "legendary".into(),
            app_name: "Calluna".into(),
            install_path: dir.join("Control").to_string_lossy().into_owned(),
            platform: "Windows".into(),
            ..Default::default()
        }
    }

    /// The administrator prompt: records what it was asked and answers `answer`.
    struct FakeElevate {
        /// `None`: the prompt is declined.
        answer: Option<u32>,
        asked: Mutex<Vec<(String, Vec<String>, Option<String>)>>,
    }

    impl FakeElevate {
        fn new(answer: Option<u32>) -> Self {
            Self { answer, asked: Mutex::new(Vec::new()) }
        }
    }

    impl Elevate for FakeElevate {
        fn run(&self, program: &str, args: &[String], cwd: Option<&str>) -> Result<u32, heronite_platform::shell::ElevatedError> {
            self.asked.lock().unwrap().push((program.to_string(), args.to_vec(), cwd.map(str::to_string)));
            self.answer.ok_or(heronite_platform::shell::ElevatedError::Declined)
        }
    }

    /// Runs the setup; the notices it gave.
    fn run(paths: &DataPaths, fl: &FirstLaunch, epic: &FakeEpic, installed: bool, elevate: &FakeElevate) -> (Flow, Vec<Notice>) {
        let (net, probe, spawner) = (FakeNet::online(), SystemProbe, InstantSpawner::default());
        let notices = Mutex::new(Vec::new());
        let notify = |n: Notice| notices.lock().unwrap().push(n);
        let prep = Prep { paths, probe: &probe, net: &net, spawner: &spawner, cloud: None, cancel: &|| false, notify: &notify };
        let flow = prep.epic_native_first_launch_with(fl, Some(epic), &|_| installed, elevate);
        assert!(spawner.spawned.lock().unwrap().is_empty(), "every step goes through the administrator prompt");
        (flow, notices.into_inner().unwrap())
    }

    #[test]
    fn prerequisites_run_once_as_administrator() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("h"));
        let fl = game(dir.path());
        let pre = Prerequisite { path: r"Engine\Extras\Redist\UEPrereqSetup_x64.exe".into(), args: "/quiet /norestart".into() };
        let e = epic(Ok(Some(pre)));
        // Declined: a warning, asked again at the next launch.
        let declined = FakeElevate::new(None);
        let (flow, notices) = run(&paths, &fl, &e, false, &declined);
        assert_eq!(flow, Flow::Go, "the game starts anyway");
        assert!(notices.iter().any(|n| n.log == "Installing Redistributables (EPIC)"), "{notices:?}");
        assert!(notices.iter().any(|n| n.warning && n.log.contains("declined")), "{notices:?}");
        assert!(!crate::native_setup::is_recorded(&paths, "legendary", "Calluna", &fl.install_path));
        // Accepted: run in its own folder, and recorded.
        let yes = FakeElevate::new(Some(0));
        assert_eq!(run(&paths, &fl, &e, false, &yes).0, Flow::Go);
        let asked = yes.asked.lock().unwrap().clone();
        let program = under(&fl.install_path, r"Engine\Extras\Redist\UEPrereqSetup_x64.exe");
        let folder = Path::new(&program).parent().unwrap().to_string_lossy().into_owned();
        assert_eq!(asked, [(program, vec!["/quiet".to_string(), "/norestart".to_string()], Some(folder))]);
        assert!(crate::native_setup::is_recorded(&paths, "legendary", "Calluna", &fl.install_path));
        // Once recorded, nothing is asked.
        run(&paths, &fl, &e, false, &yes);
        assert_eq!(*e.asked.lock().unwrap(), 2);
        assert_eq!(yes.asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_prerequisite_program_is_inside_the_install_folder() {
        let program = under(r"C:\Games\Control", "Engine\\Extras\\Redist\\UEPrereqSetup_x64.exe");
        if cfg!(windows) {
            assert_eq!(program, r"C:\Games\Control\Engine\Extras\Redist\UEPrereqSetup_x64.exe");
        } else {
            assert!(program.ends_with("Engine/Extras/Redist/UEPrereqSetup_x64.exe"));
        }
    }

    #[test]
    fn a_game_without_prerequisites_is_recorded_and_a_failed_lookup_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("h"));
        let fl = game(dir.path());
        let elevate = FakeElevate::new(Some(0));
        let failed = epic(Err("offline".into()));
        assert_eq!(run(&paths, &fl, &failed, false, &elevate).0, Flow::Go);
        assert!(!crate::native_setup::is_recorded(&paths, "legendary", "Calluna", &fl.install_path), "asked again next time");
        let none = epic(Ok(None));
        assert_eq!(run(&paths, &fl, &none, false, &elevate).0, Flow::Go);
        assert!(crate::native_setup::is_recorded(&paths, "legendary", "Calluna", &fl.install_path));
        assert!(elevate.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn store_app_games_need_the_store_app_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("h"));
        let mut fl = game(dir.path());
        fl.install_path.clear();
        fl.ea_managed = true;
        let e = epic(Ok(None));
        let elevate = FakeElevate::new(Some(0));
        // The EA app is installed: nothing to do, nothing asked.
        assert_eq!(run(&paths, &fl, &e, true, &elevate).0, Flow::Go);
        // It is not, and its installer is missing: said, and tried again next time.
        let (flow, notices) = run(&paths, &fl, &e, false, &elevate);
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().any(|n| n.warning && n.message.contains("EAappInstaller.exe")), "{notices:?}");
        assert!(elevate.asked.lock().unwrap().is_empty());
        // The installer is there: it runs silently as administrator, once.
        let redist = paths.tools_dir().join("redist").join("legendary");
        std::fs::create_dir_all(&redist).unwrap();
        std::fs::write(redist.join("EAappInstaller.exe"), "MZ").unwrap();
        let installer = redist.join("EAappInstaller.exe").to_string_lossy().into_owned();
        assert_eq!(run(&paths, &fl, &e, false, &elevate).0, Flow::Go);
        let args = vec!["EAX_LAUNCH_CLIENT=0".to_string(), "IGNORE_INSTALLED=1".to_string()];
        let folder = redist.to_string_lossy().into_owned();
        assert_eq!(*elevate.asked.lock().unwrap(), [(installer, args, Some(folder.clone()))]);
        assert!(crate::native_setup::is_recorded(&paths, "legendary", "Calluna", &folder));
        run(&paths, &fl, &e, false, &elevate);
        assert_eq!(elevate.asked.lock().unwrap().len(), 1);
        assert_eq!(*e.asked.lock().unwrap(), 0, "no prerequisites for a store app game");
    }
}
