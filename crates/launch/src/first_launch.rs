//! The first launch of a game in a Wine prefix (port of the part of Heroic's
//! `prepareWineLaunch` that follows `verifyWinePrefix`, `legendarySetup` and
//! `installFixes`).
//!
//! The prefix keeps an `installed_games` file with the app names that already
//! went through this. When the game is not in it yet, it is added (before
//! anything runs, so a failure never repeats) and then:
//!
//! * Epic: the `com.epicgames.launcher` registry key, the game's
//!   prerequisites (`manifest.prerequisites` of `legendary info`), the EA App
//!   / Ubisoft Connect installers for third-party managed games, and turning
//!   the EOS Overlay on for the prefix when it is installed (Linux);
//! * the game's `known_fixes.json` entry: winetricks verbs and programs to run
//!   in the prefix.
//!
//! Every failure is a warning: the game starts anyway, like in Heroic.

use std::path::Path;
use std::time::Duration;

use crate::env::join;
use crate::plan::{FirstLaunch, PlanContext, PrepSpec};
use crate::prepare::{run_wait, Flow, Notice, Prep};
use crate::settings::GameSettings;
use crate::wine_tools::{build_tool_command, wine_command_for, winetricks_path, WineTool};

/// How long one installer may take (Heroic does not limit it).
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const REG_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// The program a game asks to run before it starts (`manifest.prerequisites`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerequisite {
    /// Relative to the install folder.
    pub path: String,
    /// Arguments as one string (split like a shell would).
    pub args: String,
}

/// What the setup needs from the Epic side (legendary), implemented by the
/// app because the Epic crate depends on this one.
pub trait EpicSetup: Send + Sync {
    /// `getInstallInfo(...).manifest.prerequisites` (`Ok(None)`: none).
    fn prerequisites(&self, app_name: &str, platform: &str) -> Result<Option<Prerequisite>, String>;
    /// `eos_overlay.getStatus().isInstalled`.
    fn eos_overlay_installed(&self) -> bool;
    /// `eos_overlay.isEnabled(appName)` for a valid prefix folder.
    fn eos_overlay_enabled(&self, prefix: &str) -> bool;
    /// `eos_overlay.enable(appName)` for a valid prefix folder.
    fn eos_overlay_enable(&self, prefix: &str) -> Result<(), String>;
}

/// Adds `app_name` to the `installed_games` file of `prefix` (created with
/// the folder when missing). `true` when it was not there yet.
pub fn record_game(prefix: &str, app_name: &str) -> std::io::Result<bool> {
    let dir = Path::new(prefix);
    let file = dir.join("installed_games");
    let mut games: Vec<String> = if file.exists() {
        // A damaged file starts over rather than breaking the launch.
        serde_json::from_str(&std::fs::read_to_string(&file)?).unwrap_or_default()
    } else {
        std::fs::create_dir_all(dir)?;
        Vec::new()
    };
    if games.iter().any(|g| g == app_name) {
        return Ok(false);
    }
    games.push(app_name.to_string());
    std::fs::write(&file, serde_json::to_string(&games).unwrap_or_else(|_| "[]".into()))?;
    Ok(true)
}

/// The prefix folder the EOS overlay commands take (`getWinePrefixFolder`):
/// `pfx` inside a Proton prefix; `None` when it is not a Wine prefix yet.
pub fn eos_prefix_folder(settings: &GameSettings, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let folder = if settings.wine_version.kind == "proton" {
        join(&settings.wine_prefix, "pfx")
    } else {
        settings.wine_prefix.clone()
    };
    exists(&join(&folder, "user.reg")).then_some(folder)
}

impl Prep<'_> {
    /// Runs the first-launch work if the prefix has not seen this game.
    pub fn first_launch(&self, spec: &PrepSpec, epic: Option<&dyn EpicSetup>) -> Flow {
        let (Some(fl), Some(settings)) = (&spec.first_launch, &spec.wine) else { return Flow::Go };
        let prefix = settings.wine_prefix.trim();
        if prefix.is_empty() {
            return Flow::Go;
        }
        match record_game(prefix, &fl.app_name) {
            Ok(true) => log::info!("Created/Updated Wineprefix at {prefix}"),
            Ok(false) => return Flow::Go,
            Err(e) => {
                log::warn!("{prefix}/installed_games: {e}");
                return Flow::Go;
            }
        }
        let ctx = PlanContext::load(self.paths);
        match self.epic_setup(&ctx, settings, fl, epic) {
            Flow::Go => {}
            other => return other,
        }
        self.install_fixes(&ctx, settings, fl)
    }

    fn cancelled(&self) -> bool {
        (self.cancel)()
    }

    /// `runWineCommand({ commandParts, wait, protonVerb: 'run' })`. `false`
    /// when it could not run or ended with an error (installers often do).
    fn wine_run(
        &self,
        ctx: &PlanContext,
        settings: &GameSettings,
        parts: Vec<String>,
        install_path: Option<&str>,
        wait: bool,
        timeout: Duration,
    ) -> bool {
        let cmd = match wine_command_for(ctx, self.probe, settings, parts, install_path) {
            Ok(c) => c,
            Err(e) => {
                self.notice(Notice::warn(format!("Could not run a Wine command: {e}")));
                return false;
            }
        };
        let ok = run_wait(self.spawner, &cmd, timeout, self.cancel);
        if wait && !self.cancelled() {
            if let Some(server) = &settings.wine_version.wineserver {
                let wait_cmd = crate::plan::Command {
                    program: server.clone(),
                    args: vec!["--wait".into()],
                    env: cmd.env.clone(),
                    cwd: None,
                };
                run_wait(self.spawner, &wait_cmd, timeout, self.cancel);
            }
        }
        ok
    }

    /// `legendarySetup`.
    fn epic_setup(
        &self,
        ctx: &PlanContext,
        settings: &GameSettings,
        fl: &FirstLaunch,
        epic: Option<&dyn EpicSetup>,
    ) -> Flow {
        self.notice(Notice::info("Installing Redistributables (EPIC)"));
        let go = |this: &Self| if this.cancelled() { Flow::Cancelled } else { Flow::Go };

        // Fixes games like Fallout New Vegas and Dishonored: Death of the Outsider.
        let reg = ["reg", "add", "HKEY_CLASSES_ROOT\\com.epicgames.launcher", "/f"];
        self.wine_run(ctx, settings, reg.map(String::from).to_vec(), None, true, REG_TIMEOUT);
        if let Flow::Cancelled = go(self) {
            return Flow::Cancelled;
        }

        let windows = ["Windows", "Win32", "windows"].contains(&fl.platform.as_str());
        if windows && !fl.ea_managed && !fl.ubisoft_managed {
            if let Some(epic) = epic {
                match epic.prerequisites(&fl.app_name, &fl.platform) {
                    Ok(Some(pre)) if !pre.path.is_empty() => {
                        // Manifests of Unreal games use Windows separators.
                        let program = join(&fl.install_path, &pre.path.replace('\\', "/"));
                        let mut parts = vec![program];
                        parts.extend(shlex::split(&pre.args).unwrap_or_default());
                        log::info!("running the prerequisites of {}: {parts:?}", fl.app_name);
                        self.wine_run(ctx, settings, parts, Some(&fl.install_path), true, INSTALLER_TIMEOUT);
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("getInstallInfo failed with {e}"),
                }
            }
            if let Flow::Cancelled = go(self) {
                return Flow::Cancelled;
            }
        }

        let redist = self.paths.tools_dir().join("redist").join("legendary");
        let installer = if fl.ea_managed {
            Some(("EAappInstaller.exe", vec!["EAX_LAUNCH_CLIENT=0", "IGNORE_INSTALLED=1"], "the EA App"))
        } else if fl.ubisoft_managed {
            Some(("UbisoftConnectInstaller.exe", vec!["/S"], "Ubisoft Connect"))
        } else {
            None
        };
        if let Some((file, args, what)) = installer {
            let path = redist.join(file).to_string_lossy().into_owned();
            if self.probe.exists(&path) {
                let mut parts = vec![path];
                parts.extend(args.into_iter().map(String::from));
                if !self.wine_run(ctx, settings, parts, None, true, INSTALLER_TIMEOUT) {
                    log::warn!("the installer of {what} did not finish cleanly");
                }
            } else {
                self.notice(Notice::warn(format!("The installer of {what} was not found: {file}")));
            }
            if let Flow::Cancelled = go(self) {
                return Flow::Cancelled;
            }
        }

        // The overlay is installed per prefix on Linux only (on Windows it is
        // global, on macOS it does not work).
        if cfg!(target_os = "linux") {
            if let Some(epic) = epic {
                if let Some(folder) = eos_prefix_folder(settings, &|p| self.probe.exists(p)) {
                    if !epic.eos_overlay_enabled(&folder) {
                        if epic.eos_overlay_installed() {
                            log::info!("EOS Overlay: Enabling");
                            if let Err(e) = epic.eos_overlay_enable(&folder) {
                                self.notice(Notice::warn(format!("Could not enable the EOS Overlay: {e}")));
                            }
                        } else {
                            log::info!("EOS Overlay: Not Installed");
                        }
                    }
                }
            }
        }
        go(self)
    }

    /// `installFixes`: the winetricks verbs and programs of `known_fixes.json`.
    fn install_fixes(&self, ctx: &PlanContext, settings: &GameSettings, fl: &FirstLaunch) -> Flow {
        if !fl.winetricks.is_empty() {
            self.notice(Notice::info("Applying Winetricks fixes"));
            for verb in &fl.winetricks {
                if self.cancelled() {
                    return Flow::Cancelled;
                }
                self.winetricks_install(ctx, settings, verb);
            }
        }
        if !fl.run_in_prefix.is_empty() {
            self.notice(Notice::info("Installing Redistributables (FIXES)"));
            for file in &fl.run_in_prefix {
                if self.cancelled() {
                    return Flow::Cancelled;
                }
                let full = join(&fl.install_path, file);
                self.wine_run(ctx, settings, vec![full], Some(&fl.install_path), true, INSTALLER_TIMEOUT);
            }
        }
        if self.cancelled() {
            Flow::Cancelled
        } else {
            Flow::Go
        }
    }

    /// `Winetricks.install`: `winetricks -q <verb>` in the game's prefix.
    fn winetricks_install(&self, ctx: &PlanContext, settings: &GameSettings, verb: &str) {
        // Heroic downloads the script the first time it is needed.
        if !self.probe.exists(&winetricks_path(ctx)) {
            if let Err(e) = crate::tools::download_winetricks(self.paths, self.net, self.cancel) {
                log::warn!("{e}");
            }
        }
        match build_tool_command(ctx, self.probe, settings, &WineTool::WinetricksInstall(verb.to_string())) {
            Ok(tool) => {
                for note in &tool.notes {
                    log::warn!("winetricks: {note}");
                }
                if !run_wait(self.spawner, &tool.command, INSTALLER_TIMEOUT, self.cancel) && !self.cancelled() {
                    self.notice(Notice::warn(format!("Winetricks could not install {verb}")));
                }
            }
            Err(e) => self.notice(Notice::warn(format!("Could not install {verb} with winetricks: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::plan::{Command, PrepSpec};
    use crate::prepare::tests::InstantSpawner;
    use crate::probe::FakeProbe;
    use crate::settings::WineVersion;
    use heronite_data::HeroicPaths;
    use std::sync::Mutex;

    struct FakeEpic {
        prereq: Result<Option<Prerequisite>, String>,
        installed: bool,
        enabled: Mutex<bool>,
        enable_calls: Mutex<Vec<String>>,
    }

    impl FakeEpic {
        fn new(prereq: Option<Prerequisite>, installed: bool, enabled: bool) -> Self {
            Self { prereq: Ok(prereq), installed, enabled: Mutex::new(enabled), enable_calls: Mutex::default() }
        }
    }

    impl EpicSetup for FakeEpic {
        fn prerequisites(&self, _: &str, _: &str) -> Result<Option<Prerequisite>, String> {
            self.prereq.clone()
        }
        fn eos_overlay_installed(&self) -> bool {
            self.installed
        }
        fn eos_overlay_enabled(&self, _: &str) -> bool {
            *self.enabled.lock().unwrap()
        }
        fn eos_overlay_enable(&self, prefix: &str) -> Result<(), String> {
            self.enable_calls.lock().unwrap().push(prefix.to_string());
            *self.enabled.lock().unwrap() = true;
            Ok(())
        }
    }

    struct Fx {
        dir: tempfile::TempDir,
        paths: HeroicPaths,
        spawner: InstantSpawner,
        prefix: String,
    }

    impl Fx {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = HeroicPaths::at(dir.path().join("heroic"));
            let prefix = dir.path().join("pfx");
            std::fs::create_dir_all(&prefix).unwrap();
            Self { dir, paths, spawner: InstantSpawner::default(), prefix: prefix.to_string_lossy().into_owned() }
        }

        fn spec(&self, fl: FirstLaunch) -> PrepSpec {
            PrepSpec {
                wine: Some(GameSettings {
                    wine_prefix: self.prefix.clone(),
                    wine_version: WineVersion {
                        bin: "/w/bin/wine".into(),
                        name: "W".into(),
                        kind: "wine".into(),
                        wineserver: Some("/w/bin/wineserver".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                first_launch: Some(fl),
                ..Default::default()
            }
        }

        fn run(&self, spec: &PrepSpec, epic: Option<&dyn EpicSetup>, probe: &FakeProbe) -> (Flow, Vec<Notice>) {
            let seen = Mutex::new(Vec::new());
            let sink = |n: Notice| seen.lock().unwrap().push(n);
            let net = FakeNet::offline();
            let prep = Prep {
                paths: &self.paths,
                probe,
                net: &net,
                spawner: &self.spawner,
                cloud: None,
                cancel: &|| false,
                notify: &sink,
            };
            let flow = prep.first_launch(spec, epic);
            let notices = seen.into_inner().unwrap();
            (flow, notices)
        }

        fn spawned(&self) -> Vec<Command> {
            self.spawner.spawned.lock().unwrap().clone()
        }
    }

    fn probe(fx: &Fx) -> FakeProbe {
        FakeProbe::new("/home/u")
            .with_path("/w/bin/wine")
            .with_path("/w/bin/wineserver")
            .with_path(&fx.prefix)
            .with_path(&join(&fx.prefix, "user.reg"))
            .with_path(&fx.paths.tools_dir().join("winetricks").to_string_lossy())
    }

    fn base() -> FirstLaunch {
        FirstLaunch {
            app_name: "Calluna".into(),
            install_path: "/games/Control".into(),
            platform: "Windows".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_marker_file_matches_heroics_format() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("a/b").to_string_lossy().into_owned();
        assert!(record_game(&prefix, "one").unwrap());
        let file = std::path::Path::new(&prefix).join("installed_games");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"["one"]"#);
        assert!(!record_game(&prefix, "one").unwrap());
        assert!(record_game(&prefix, "two").unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"["one","two"]"#);
        // Heroic's own formatting is read as well.
        std::fs::write(&file, r#"["x", "y"]"#).unwrap();
        assert!(!record_game(&prefix, "y").unwrap());
        // A damaged file starts over.
        std::fs::write(&file, "not json").unwrap();
        assert!(record_game(&prefix, "z").unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"["z"]"#);
    }

    #[test]
    fn a_game_already_in_the_prefix_does_nothing() {
        let fx = Fx::new();
        std::fs::write(join(&fx.prefix, "installed_games"), r#"["Calluna"]"#).unwrap();
        let spec = fx.spec(FirstLaunch { winetricks: vec!["vcrun2022".into()], ..base() });
        let (flow, notices) = fx.run(&spec, None, &probe(&fx));
        assert_eq!(flow, Flow::Go);
        assert!(notices.is_empty());
        assert!(fx.spawned().is_empty());
    }

    #[test]
    fn prerequisites_run_in_the_prefix_once() {
        let fx = Fx::new();
        let epic = FakeEpic::new(
            Some(Prerequisite {
                path: "Engine\\Extras\\Redist\\UEPrereqSetup_x64.exe".into(),
                args: "/silent /name \"Some Name\"".into(),
            }),
            false,
            false,
        );
        let spec = fx.spec(base());
        let p = probe(&fx);
        let (flow, notices) = fx.run(&spec, Some(&epic), &p);
        assert_eq!(flow, Flow::Go);
        assert_eq!(notices[0].message, "Installing Redistributables (EPIC)");
        let cmds = fx.spawned();
        // reg add, wineserver --wait, prerequisites, wineserver --wait
        assert_eq!(cmds[0].program, "/w/bin/wine");
        assert_eq!(cmds[0].args, ["reg", "add", "HKEY_CLASSES_ROOT\\com.epicgames.launcher", "/f"]);
        assert_eq!(cmds[0].env["WINEPREFIX"], fx.prefix);
        assert_eq!((cmds[1].program.as_str(), cmds[1].args.as_slice()), ("/w/bin/wineserver", ["--wait".to_string()].as_slice()));
        assert_eq!(cmds[2].program, "/w/bin/wine");
        assert_eq!(
            cmds[2].args,
            ["/games/Control/Engine/Extras/Redist/UEPrereqSetup_x64.exe", "/silent", "/name", "Some Name"]
        );
        assert_eq!(cmds[2].env["STEAM_COMPAT_INSTALL_PATH"], "/games/Control");
        assert_eq!(cmds.len(), 4);
        assert_eq!(std::fs::read_to_string(join(&fx.prefix, "installed_games")).unwrap(), r#"["Calluna"]"#);
        // The second launch finds the marker.
        fx.spawner.spawned.lock().unwrap().clear();
        let (_, notices) = fx.run(&spec, Some(&epic), &p);
        assert!(notices.is_empty() && fx.spawned().is_empty());
    }

    #[test]
    fn third_party_games_use_their_launcher_installers_not_prerequisites() {
        let fx = Fx::new();
        let redist = fx.paths.tools_dir().join("redist/legendary");
        let installer = redist.join("EAappInstaller.exe").to_string_lossy().into_owned();
        let epic = FakeEpic::new(Some(Prerequisite { path: "x.exe".into(), args: String::new() }), false, false);
        let spec = fx.spec(FirstLaunch { ea_managed: true, ..base() });
        let p = probe(&fx).with_path(&installer);
        fx.run(&spec, Some(&epic), &p);
        let cmds = fx.spawned();
        assert!(cmds.iter().all(|c| !c.args.iter().any(|a| a.ends_with("x.exe"))));
        let ea = cmds.iter().find(|c| c.args.first().map(String::as_str) == Some(installer.as_str())).unwrap();
        assert_eq!(ea.args[1..], ["EAX_LAUNCH_CLIENT=0", "IGNORE_INSTALLED=1"]);

        // Ubisoft, with its installer missing: a warning, nothing run for it.
        let fx2 = Fx::new();
        let spec = fx2.spec(FirstLaunch { ubisoft_managed: true, ..base() });
        let (flow, notices) = fx2.run(&spec, Some(&epic), &probe(&fx2));
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().any(|n| n.warning && n.message.contains("Ubisoft Connect")));
    }

    #[test]
    fn the_overlay_is_enabled_only_when_installed_and_off() {
        if !cfg!(target_os = "linux") {
            return;
        }
        // Installed and off: enabled for the prefix.
        let fx = Fx::new();
        let epic = FakeEpic::new(None, true, false);
        fx.run(&fx.spec(base()), Some(&epic), &probe(&fx));
        assert_eq!(*epic.enable_calls.lock().unwrap(), [fx.prefix.clone()]);

        // Not installed: left alone.
        let fx2 = Fx::new();
        let epic = FakeEpic::new(None, false, false);
        fx2.run(&fx2.spec(base()), Some(&epic), &probe(&fx2));
        assert!(epic.enable_calls.lock().unwrap().is_empty());

        // Already on: left alone.
        let fx3 = Fx::new();
        let epic = FakeEpic::new(None, true, true);
        fx3.run(&fx3.spec(base()), Some(&epic), &probe(&fx3));
        assert!(epic.enable_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn eos_prefix_folder_follows_proton_and_needs_user_reg() {
        let mut s = GameSettings { wine_prefix: "/pfx".into(), ..Default::default() };
        let has = |p: &str| p == "/pfx/user.reg" || p == "/pfx/pfx/user.reg";
        assert_eq!(eos_prefix_folder(&s, &has).as_deref(), Some("/pfx"));
        s.wine_version.kind = "proton".into();
        assert_eq!(eos_prefix_folder(&s, &has).as_deref(), Some("/pfx/pfx"));
        assert_eq!(eos_prefix_folder(&s, &|_| false), None);
    }

    #[test]
    fn known_fixes_install_winetricks_verbs_then_programs() {
        let fx = Fx::new();
        let spec = fx.spec(FirstLaunch {
            winetricks: vec!["vcrun2022".into(), "d3dcompiler_47".into()],
            run_in_prefix: vec!["EpicOnlineServices/EpicOnlineServicesInstaller.exe".into()],
            ..base()
        });
        let (flow, notices) = fx.run(&spec, None, &probe(&fx));
        assert_eq!(flow, Flow::Go);
        let messages: Vec<&str> = notices.iter().map(|n| n.message.as_str()).collect();
        assert!(messages.contains(&"Applying Winetricks fixes"), "{messages:?}");
        assert!(messages.contains(&"Installing Redistributables (FIXES)"), "{messages:?}");
        let cmds = fx.spawned();
        let tricks: Vec<_> = cmds.iter().filter(|c| c.program.ends_with("/winetricks")).collect();
        assert_eq!(tricks.len(), 2);
        assert_eq!(tricks[0].args, ["-q", "vcrun2022"]);
        assert_eq!(tricks[1].args, ["-q", "d3dcompiler_47"]);
        assert_eq!(tricks[0].env["WINEPREFIX"], fx.prefix);
        assert!(cmds
            .iter()
            .any(|c| c.args == ["/games/Control/EpicOnlineServices/EpicOnlineServicesInstaller.exe"]));
        // The verbs come before the programs.
        let at = |needle: &str| cmds.iter().position(|c| c.args.iter().any(|a| a.contains(needle))).unwrap();
        assert!(at("d3dcompiler_47") < at("EpicOnlineServicesInstaller"));
    }

    #[test]
    fn failures_are_warnings_and_cancelling_stops_the_work() {
        // winetricks missing: warning, the launch goes on.
        let fx = Fx::new();
        let spec = fx.spec(FirstLaunch { winetricks: vec!["vcrun2022".into()], ..base() });
        let p = FakeProbe::new("/home/u")
            .with_path("/w/bin/wine")
            .with_path("/w/bin/wineserver")
            .with_path(&fx.prefix);
        let (flow, notices) = fx.run(&spec, None, &p);
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().any(|n| n.warning && n.message.contains("vcrun2022")), "{notices:?}");

        // Cancelled while preparing.
        let fx = Fx::new();
        let spec = fx.spec(FirstLaunch { winetricks: vec!["vcrun2022".into()], ..base() });
        let net = FakeNet::offline();
        let sink = |_: Notice| {};
        let p = probe(&fx);
        let prep = Prep { paths: &fx.paths, probe: &p, net: &net, spawner: &fx.spawner, cloud: None, cancel: &|| true, notify: &sink };
        assert_eq!(prep.first_launch(&spec, None), Flow::Cancelled);
        assert!(!fx.spawned().iter().any(|c| c.program.ends_with("/winetricks")));
    }

    #[test]
    fn nothing_happens_without_a_spec() {
        let fx = Fx::new();
        let p = probe(&fx);
        let (flow, _) = fx.run(&PrepSpec::default(), None, &p);
        assert_eq!(flow, Flow::Go);
        let mut spec = fx.spec(base());
        spec.wine.as_mut().unwrap().wine_prefix = String::new();
        let (flow, notices) = fx.run(&spec, None, &p);
        assert_eq!(flow, Flow::Go);
        assert!(notices.is_empty());
    }
}
