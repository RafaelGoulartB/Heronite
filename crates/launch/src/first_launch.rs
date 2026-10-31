//! The first launch of a game in a Wine prefix (port of the part of Heroic's
//! `prepareWineLaunch` that follows `verifyWinePrefix`, `legendarySetup` and
//! `installFixes`).
//!
//! The prefix keeps an `installed_games` file with the app names that already
//! went through this. When the game is not in it yet, this runs and the game
//! is added to the file only AFTER the setup ran to the end (an interrupted,
//! cancelled or impossible-to-run setup is retried at the next launch; an
//! installer that runs and exits with an error is a warning, not a retry):
//!
//! * Epic: the `com.epicgames.launcher` registry key, the game's
//!   prerequisites (`manifest.prerequisites` of `legendary info`), the EA App
//!   / Ubisoft Connect installers for third-party managed games, and turning
//!   the EOS Overlay on for the prefix when it is installed (Linux);
//! * GOG and Amazon Games: the store's installers for the game (Amazon's
//!   `fuel.json` `PostInstall`; GOG's redistributables and install scripts,
//!   through [`StoreSetup`]);
//! * the game's `known_fixes.json` entry: winetricks verbs and programs to run
//!   in the prefix.
//!
//! Every failure is a warning: the game starts anyway, like in Heroic.

use heronite_i18n::tr;
use std::cell::Cell;
use std::path::Path;
use std::time::Duration;

use crate::env::join;
use crate::plan::{FirstLaunch, PlanContext, PrepSpec, SetupStep};
use crate::prepare::{run_wait, Flow, Notice, Prep};
use crate::settings::GameSettings;
use crate::wine_tools::{wine_command_for, winetricks_install_command, winetricks_path};

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

/// What the setup needs from the GOG side (redistributables, the game's
/// install scripts), implemented by the app because the store crates depend
/// on this one.
pub trait StoreSetup: Send + Sync {
    /// Programs to run in the prefix the first time `app_name` of `runner`
    /// starts there. Blocking; may download (`Err` = try again next launch).
    fn steps(&self, runner: &str, app_name: &str, install_path: &str) -> Result<Vec<SetupStep>, String>;
}

/// The app names in the `installed_games` file of `prefix` (empty when the
/// file is missing; a damaged file counts as empty).
fn recorded_games(prefix: &str) -> Vec<String> {
    let file = Path::new(prefix).join("installed_games");
    std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Whether `app_name` already went through the first-launch setup in `prefix`.
pub fn is_recorded(prefix: &str, app_name: &str) -> bool {
    recorded_games(prefix).iter().any(|g| g == app_name)
}

/// Adds `app_name` to the `installed_games` file of `prefix` (created with
/// the folder when missing), atomically. `true` when it was not there yet.
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
    let text = serde_json::to_string(&games).unwrap_or_else(|_| "[]".into());
    heronite_data::temps::write_atomic(&file, text.as_bytes(), None)?;
    Ok(true)
}

/// The prefix folder the EOS overlay commands take (`getWinePrefixFolder`):
/// `pfx` inside a Proton prefix; `None` when it is not a Wine prefix yet.
pub fn eos_prefix_folder(settings: &GameSettings, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let folder = if settings.wine_version.kind == "proton" { join(&settings.wine_prefix, "pfx") } else { settings.wine_prefix.clone() };
    exists(&join(&folder, "user.reg")).then_some(folder)
}

impl Prep<'_> {
    /// Runs the first-launch work if the prefix has not seen this game.
    pub fn first_launch(&self, spec: &PrepSpec, epic: Option<&dyn EpicSetup>, store: Option<&dyn StoreSetup>) -> Flow {
        let Some(fl) = &spec.first_launch else { return Flow::Go };
        // A game that runs without Wine (Windows builds on Windows).
        let Some(settings) = &spec.wine else {
            return match fl.runner.as_str() {
                "legendary" => self.epic_native_first_launch(fl, epic),
                _ => self.native_first_launch(fl, store),
            };
        };
        let prefix = settings.wine_prefix.trim();
        if prefix.is_empty() {
            return Flow::Go;
        }
        if is_recorded(prefix, &fl.app_name) {
            return Flow::Go;
        }
        let ctx = PlanContext::load(self.paths);
        // Set when a step could not even be run (so the setup is not done).
        let incomplete = Cell::new(false);
        let setup = match fl.runner.as_str() {
            "gog" | "nile" => self.store_setup(&ctx, settings, fl, store, &incomplete),
            _ => self.epic_setup(&ctx, settings, fl, epic, &incomplete),
        };
        if setup != Flow::Go {
            return setup;
        }
        let flow = self.install_fixes(&ctx, settings, fl, &incomplete);
        if flow == Flow::Go {
            if incomplete.get() {
                log::warn!("the setup of {} in {prefix} was incomplete: it runs again at the next launch", fl.app_name);
            } else {
                match record_game(prefix, &fl.app_name) {
                    Ok(_) => log::info!("Created/Updated Wineprefix at {prefix}"),
                    Err(e) => log::warn!("{prefix}/installed_games: {e}"),
                }
            }
        }
        flow
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
        incomplete: &Cell<bool>,
    ) -> bool {
        let cmd = match wine_command_for(ctx, self.probe, settings, parts, install_path) {
            Ok(c) => c,
            Err(e) => {
                self.notice(Notice::warn(
                    format!("Could not run a Wine command: {e}"),
                    tr!("Could not run a Wine command: {error}", error = e),
                ));
                incomplete.set(true);
                return false;
            }
        };
        let ok = run_wait(self.spawner, &cmd, timeout, self.cancel);
        // A sandbox only ends with its last program (see `crate::sandbox`).
        if wait && !self.cancelled() && !settings.sandbox.enabled {
            if let Some(server) = &settings.wine_version.wineserver {
                let wait_cmd =
                    crate::plan::Command { program: server.clone(), args: vec!["--wait".into()], env: cmd.env.clone(), cwd: None };
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
        incomplete: &Cell<bool>,
    ) -> Flow {
        self.notice(Notice::info("Installing Redistributables (EPIC)", tr!("Installing Redistributables (EPIC)")));
        let go = |this: &Self| if this.cancelled() { Flow::Cancelled } else { Flow::Go };

        // Fixes games like Fallout New Vegas and Dishonored: Death of the Outsider.
        let reg = ["reg", "add", "HKEY_CLASSES_ROOT\\com.epicgames.launcher", "/f"];
        self.wine_run(ctx, settings, reg.map(String::from).to_vec(), None, true, REG_TIMEOUT, incomplete);
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
                        self.wine_run(ctx, settings, parts, Some(&fl.install_path), true, INSTALLER_TIMEOUT, incomplete);
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
                if !self.wine_run(ctx, settings, parts, None, true, INSTALLER_TIMEOUT, incomplete) {
                    log::warn!("the installer of {what} did not finish cleanly");
                }
            } else {
                let shown = if fl.ea_managed {
                    tr!("The installer of the EA App was not found: {file}", file)
                } else {
                    tr!("The installer of Ubisoft Connect was not found: {file}", file)
                };
                self.notice(Notice::warn(format!("The installer of {what} was not found: {file}"), shown));
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
                                self.notice(Notice::warn(
                                    format!("Could not enable the EOS Overlay: {e}"),
                                    tr!("Could not enable the EOS Overlay: {error}", error = e),
                                ));
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

    /// The installers of a GOG or Amazon game: Amazon's are in the plan,
    /// GOG's come from `store`.
    fn store_setup(
        &self,
        ctx: &PlanContext,
        settings: &GameSettings,
        fl: &FirstLaunch,
        store: Option<&dyn StoreSetup>,
        incomplete: &Cell<bool>,
    ) -> Flow {
        let mut steps = fl.post_install.clone();
        if let Some(store) = store {
            // gogdl may download the redistributables first: it can take a while.
            self.notice(Notice::info("Getting the game's installers...", tr!("Getting the game's installers...")));
            match store.steps(&fl.runner, &fl.app_name, &fl.install_path) {
                Ok(more) => steps.extend(more),
                Err(e) => {
                    self.notice(Notice::warn(
                        format!("Could not prepare the game's installers: {e}"),
                        tr!("Could not prepare the game's installers: {error}", error = e),
                    ));
                    incomplete.set(true);
                }
            }
        }
        if steps.is_empty() {
            return if self.cancelled() { Flow::Cancelled } else { Flow::Go };
        }
        let store_name = if fl.runner == "gog" { "GOG" } else { "AMAZON" };
        self.notice(Notice::info(
            format!("Installing Redistributables ({store_name})"),
            tr!("Installing Redistributables ({store})", store = store_name),
        ));
        for step in steps {
            if self.cancelled() {
                return Flow::Cancelled;
            }
            log::info!("first-launch setup of {}: {:?}", fl.app_name, step.parts);
            let cwd = step.cwd.as_deref().unwrap_or(&fl.install_path);
            self.wine_run(ctx, settings, step.parts, Some(cwd), true, INSTALLER_TIMEOUT, incomplete);
        }
        if self.cancelled() {
            Flow::Cancelled
        } else {
            Flow::Go
        }
    }

    /// `installFixes`: the winetricks verbs and programs of `known_fixes.json`.
    fn install_fixes(&self, ctx: &PlanContext, settings: &GameSettings, fl: &FirstLaunch, incomplete: &Cell<bool>) -> Flow {
        if !fl.winetricks.is_empty() {
            self.notice(Notice::info("Applying Winetricks fixes", tr!("Applying Winetricks fixes")));
            for verb in &fl.winetricks {
                if self.cancelled() {
                    return Flow::Cancelled;
                }
                self.winetricks_install(ctx, settings, verb, incomplete);
            }
        }
        if !fl.run_in_prefix.is_empty() {
            self.notice(Notice::info("Installing Redistributables (FIXES)", tr!("Installing Redistributables (FIXES)")));
            for file in &fl.run_in_prefix {
                if self.cancelled() {
                    return Flow::Cancelled;
                }
                let full = join(&fl.install_path, file);
                self.wine_run(ctx, settings, vec![full], Some(&fl.install_path), true, INSTALLER_TIMEOUT, incomplete);
            }
        }
        if self.cancelled() {
            Flow::Cancelled
        } else {
            Flow::Go
        }
    }

    /// `Winetricks.install`: `winetricks -q <verb>` in the game's prefix.
    fn winetricks_install(&self, ctx: &PlanContext, settings: &GameSettings, verb: &str, incomplete: &Cell<bool>) {
        // Heroic downloads the script the first time it is needed.
        if !self.probe.exists(&winetricks_path(ctx)) {
            if let Err(e) = crate::tools::download_winetricks(self.paths, self.net, self.cancel) {
                log::warn!("{e}");
                incomplete.set(true);
            }
        }
        match winetricks_install_command(ctx, self.probe, settings, verb) {
            Ok(tool) => {
                for note in &tool.notes {
                    log::warn!("winetricks: {note}");
                }
                if !run_wait(self.spawner, &tool.command, INSTALLER_TIMEOUT, self.cancel) && !self.cancelled() {
                    self.notice(Notice::warn(
                        format!("Winetricks could not install {verb}"),
                        tr!("Winetricks could not install {verb}", verb),
                    ));
                }
            }
            Err(e) => {
                self.notice(Notice::warn(
                    format!("Could not install {verb} with winetricks: {e}"),
                    tr!("Could not install {verb} with winetricks: {error}", verb, error = e),
                ));
                incomplete.set(true);
            }
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
    use heronite_data::DataPaths;
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
        _dir: tempfile::TempDir,
        paths: DataPaths,
        spawner: InstantSpawner,
        prefix: String,
    }

    impl Fx {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::at(dir.path().join("heroic"));
            let prefix = dir.path().join("pfx");
            std::fs::create_dir_all(&prefix).unwrap();
            Self { _dir: dir, paths, spawner: InstantSpawner::default(), prefix: prefix.to_string_lossy().into_owned() }
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
            self.run_with(spec, epic, None, probe)
        }

        fn run_with(
            &self,
            spec: &PrepSpec,
            epic: Option<&dyn EpicSetup>,
            store: Option<&dyn StoreSetup>,
            probe: &FakeProbe,
        ) -> (Flow, Vec<Notice>) {
            let seen = Mutex::new(Vec::new());
            let sink = |n: Notice| seen.lock().unwrap().push(n);
            let net = FakeNet::offline();
            let prep = Prep { paths: &self.paths, probe, net: &net, spawner: &self.spawner, cloud: None, cancel: &|| false, notify: &sink };
            let flow = prep.first_launch(spec, epic, store);
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
        FirstLaunch { app_name: "Calluna".into(), install_path: "/games/Control".into(), platform: "Windows".into(), ..Default::default() }
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

    // First-launch setup of a Wine prefix: Linux only.
    #[cfg(unix)]
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

    // First-launch setup of a Wine prefix: Linux only.
    #[cfg(unix)]
    #[test]
    fn prerequisites_run_in_the_prefix_once() {
        let fx = Fx::new();
        let epic = FakeEpic::new(
            Some(Prerequisite { path: "Engine\\Extras\\Redist\\UEPrereqSetup_x64.exe".into(), args: "/silent /name \"Some Name\"".into() }),
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
        assert_eq!(cmds[2].args, ["/games/Control/Engine/Extras/Redist/UEPrereqSetup_x64.exe", "/silent", "/name", "Some Name"]);
        assert_eq!(cmds[2].env["STEAM_COMPAT_INSTALL_PATH"], "/games/Control");
        assert_eq!(cmds.len(), 4);
        assert_eq!(std::fs::read_to_string(join(&fx.prefix, "installed_games")).unwrap(), r#"["Calluna"]"#);
        // The second launch finds the marker.
        fx.spawner.spawned.lock().unwrap().clear();
        let (_, notices) = fx.run(&spec, Some(&epic), &p);
        assert!(notices.is_empty() && fx.spawned().is_empty());
    }

    // First-launch setup of a Wine prefix: Linux only.
    #[cfg(unix)]
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
        assert_eq!(*epic.enable_calls.lock().unwrap(), std::slice::from_ref(&fx.prefix));

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

    // First-launch setup of a Wine prefix: Linux only.
    #[cfg(unix)]
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
        assert!(cmds.iter().any(|c| c.args == ["/games/Control/EpicOnlineServices/EpicOnlineServicesInstaller.exe"]));
        // The verbs come before the programs.
        let at = |needle: &str| cmds.iter().position(|c| c.args.iter().any(|a| a.contains(needle))).unwrap();
        assert!(at("d3dcompiler_47") < at("EpicOnlineServicesInstaller"));
    }

    #[test]
    fn failures_are_warnings_and_cancelling_stops_the_work() {
        // winetricks missing: warning, the launch goes on.
        let fx = Fx::new();
        let spec = fx.spec(FirstLaunch { winetricks: vec!["vcrun2022".into()], ..base() });
        let p = FakeProbe::new("/home/u").with_path("/w/bin/wine").with_path("/w/bin/wineserver").with_path(&fx.prefix);
        let (flow, notices) = fx.run(&spec, None, &p);
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().any(|n| n.warning && n.message.contains("vcrun2022")), "{notices:?}");
        // A step that could not run at all leaves the game unrecorded: retried next launch.
        assert!(!is_recorded(&fx.prefix, "Calluna"));

        // Cancelled while preparing.
        let fx = Fx::new();
        let spec = fx.spec(FirstLaunch { winetricks: vec!["vcrun2022".into()], ..base() });
        let net = FakeNet::offline();
        let sink = |_: Notice| {};
        let p = probe(&fx);
        let prep = Prep { paths: &fx.paths, probe: &p, net: &net, spawner: &fx.spawner, cloud: None, cancel: &|| true, notify: &sink };
        assert_eq!(prep.first_launch(&spec, None, None), Flow::Cancelled);
        assert!(!fx.spawned().iter().any(|c| c.program.ends_with("/winetricks")));
        assert!(!is_recorded(&fx.prefix, "Calluna"), "an interrupted setup must be retried");
    }

    #[test]
    fn the_game_is_recorded_only_after_the_setup_finished() {
        // The marker is not there while the setup runs: a spawner that looks for it at every command.
        struct Peek<'a> {
            inner: InstantSpawner,
            prefix: &'a str,
            seen: Mutex<Vec<bool>>,
        }
        impl crate::runtime::Spawner for Peek<'_> {
            fn spawn(&self, cmd: &Command, log: Option<&Path>) -> std::io::Result<Box<dyn crate::runtime::ProcessHandle>> {
                self.seen.lock().unwrap().push(is_recorded(self.prefix, "Calluna"));
                self.inner.spawn(cmd, log)
            }
            fn open_uri(&self, uri: &str) -> std::io::Result<()> {
                self.inner.open_uri(uri)
            }
            fn kill_pattern(&self, pattern: &str, since: u64) {
                self.inner.kill_pattern(pattern, since)
            }
        }
        let fx = Fx::new();
        let spec = fx.spec(FirstLaunch { run_in_prefix: vec!["a.exe".into(), "b.exe".into()], ..base() });
        let peek = Peek { inner: InstantSpawner::default(), prefix: &fx.prefix, seen: Mutex::default() };
        let p = probe(&fx);
        let net = FakeNet::offline();
        let sink = |_: Notice| {};
        let prep = Prep { paths: &fx.paths, probe: &p, net: &net, spawner: &peek, cloud: None, cancel: &|| false, notify: &sink };
        assert_eq!(prep.first_launch(&spec, None, None), Flow::Go);
        let seen = peek.seen.into_inner().unwrap();
        assert!(seen.len() >= 3 && seen.iter().all(|recorded| !recorded), "{seen:?}");
        assert!(is_recorded(&fx.prefix, "Calluna"));
        // Written atomically: nothing but the marker next to it.
        let tmp: Vec<_> =
            std::fs::read_dir(&fx.prefix).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(tmp.is_empty());
    }

    #[test]
    fn an_installer_that_exits_with_an_error_does_not_repeat_forever() {
        let mut fx = Fx::new();
        fx.spawner.exit_code = 1;
        let spec = fx.spec(FirstLaunch { run_in_prefix: vec!["a.exe".into()], ..base() });
        let (flow, _) = fx.run(&spec, None, &probe(&fx));
        assert_eq!(flow, Flow::Go);
        assert!(is_recorded(&fx.prefix, "Calluna"));
    }

    struct FakeStore(Result<Vec<SetupStep>, String>);

    impl StoreSetup for FakeStore {
        fn steps(&self, runner: &str, app_name: &str, install_path: &str) -> Result<Vec<SetupStep>, String> {
            assert_eq!((runner, app_name, install_path), ("gog", "Calluna", "/games/Control"));
            self.0.clone()
        }
    }

    fn step(parts: &[&str]) -> SetupStep {
        SetupStep { parts: parts.iter().map(|p| p.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn store_games_run_their_installers_instead_of_the_epic_setup() {
        let fx = Fx::new();
        let spec =
            fx.spec(FirstLaunch { runner: "nile".into(), post_install: vec![step(&["/games/Control/redist/vc.exe", "/quiet"])], ..base() });
        let (flow, notices) = fx.run(&spec, None, &probe(&fx));
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().any(|n| n.log == "Installing Redistributables (AMAZON)"), "{notices:?}");
        let spawned = fx.spawned();
        assert_eq!(spawned.len(), 2, "{spawned:?}");
        assert_eq!(spawned[0].args, ["/games/Control/redist/vc.exe", "/quiet"]);
        assert!(!spawned.iter().any(|c| c.args.iter().any(|a| a.contains("epicgames"))));
        assert!(is_recorded(&fx.prefix, "Calluna"));

        let fx = Fx::new();
        let store = FakeStore(Ok(vec![step(&["msiexec", "/i", "/r/physx.msi", "/qb"])]));
        let spec = fx.spec(FirstLaunch { runner: "gog".into(), ..base() });
        let (flow, _) = fx.run_with(&spec, None, Some(&store), &probe(&fx));
        assert_eq!(flow, Flow::Go);
        assert_eq!(fx.spawned()[0].args, ["msiexec", "/i", "/r/physx.msi", "/qb"]);
        assert!(is_recorded(&fx.prefix, "Calluna"));
    }

    #[test]
    fn a_store_setup_that_cannot_prepare_runs_again_next_time() {
        let fx = Fx::new();
        let store = FakeStore(Err("offline".into()));
        let spec = fx.spec(FirstLaunch { runner: "gog".into(), ..base() });
        let (flow, notices) = fx.run_with(&spec, None, Some(&store), &probe(&fx));
        assert_eq!(flow, Flow::Go);
        assert!(notices.iter().any(|n| n.log.contains("offline")));
        assert!(!is_recorded(&fx.prefix, "Calluna"));
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
