//! Everything Heroic does around a launch besides running the game (port of
//! the relevant parts of `prepareLaunch`, `prepareWineLaunch` and
//! `launchEventCallback` in `backend/launcher.ts`):
//!
//! * cloud saves: download before the game starts, upload after it exits;
//! * the umu runtime: downloaded when it is wanted and missing;
//! * DXVK / DXVK-NVAPI / VKD3D installed into Wine prefixes
//!   (`autoInstallDxvk`, `autoInstallDxvkNvapi`, `autoInstallVkd3d`), the
//!   releases downloaded into `tools/` when missing;
//! * the EAC and BattlEye runtimes (`eacRuntime`, `battlEyeRuntime`).
//!
//! Creating a missing prefix is a `pre_command` of the plan (see
//! [`crate::wine`]); this runs right after it. Everything is blocking and runs
//! on the launch thread; the network, the machine and the processes are
//! injected so tests need none of them.

use std::time::{Duration, Instant};

use heronite_data::HeroicPaths;

use crate::dxvk::{self, Action, WineRunner};
use crate::net::{Net, CANCELLED};
use crate::plan::{PlanContext, PrepSpec};
use crate::probe::Probe;
use crate::runtime::Spawner;
use crate::runtimes::{self, Runtime};
use crate::settings::GameSettings;
use crate::tools::{self, Tool};
use crate::wine_tools::wine_command;

/// Something worth telling the user while a launch is being prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub message: String,
    /// Something failed (the launch carries on) rather than plain progress.
    pub warning: bool,
}

impl Notice {
    pub fn info(message: impl Into<String>) -> Self {
        Self { message: message.into(), warning: false }
    }
    pub fn warn(message: impl Into<String>) -> Self {
        Self { message: message.into(), warning: true }
    }
}

/// Cloud save synchronisation (`LegendaryGame.syncSaves`). Implemented by the
/// app on top of the Epic crate, which depends on this one.
pub trait CloudSync: Send + Sync {
    /// Runs the sync for `app_name` with `saves_path`; `flag` is
    /// `--skip-upload` (download) or `--skip-download` (upload). Returns the
    /// tool's output.
    fn sync(&self, app_name: &str, saves_path: &str, flag: &str) -> Result<String, String>;
}

/// How a preparation step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    Go,
    /// The user stopped the launch meanwhile.
    Cancelled,
    /// The launch cannot continue.
    Fail(String),
}

/// What preparing a launch can use.
pub struct Prep<'a> {
    pub paths: &'a HeroicPaths,
    pub probe: &'a dyn Probe,
    pub net: &'a dyn Net,
    pub spawner: &'a dyn Spawner,
    pub cloud: Option<&'a dyn CloudSync>,
    pub cancel: &'a dyn Fn() -> bool,
    pub notify: &'a dyn Fn(Notice),
}

/// Runs `cmd` and waits for it (killing it after `timeout` or on cancel).
/// False when it could not be started, timed out or was cancelled.
pub fn run_wait(spawner: &dyn Spawner, cmd: &crate::plan::Command, timeout: Duration, cancel: &dyn Fn() -> bool) -> bool {
    let mut handle = match spawner.spawn(cmd, None) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("{}: {e}", cmd.display());
            return false;
        }
    };
    let deadline = Instant::now() + timeout;
    loop {
        match handle.poll() {
            Ok(None) => {}
            Ok(Some(info)) => return info.code == Some(0),
            Err(_) => return false,
        }
        if cancel() || Instant::now() > deadline {
            handle.terminate();
            std::thread::sleep(Duration::from_millis(100));
            handle.kill();
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The first line of a legendary output that reports a failure.
pub fn sync_problem(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|l| l.contains("ERROR") || l.contains("CRITICAL") || l.starts_with("Traceback"))
        .map(|l| l.to_string())
}

/// Windows wine commands (`reg add`, `wineboot -u`) in the game's prefix.
struct PrefixRunner<'a> {
    ctx: &'a PlanContext,
    prep: &'a Prep<'a>,
    settings: &'a GameSettings,
}

impl WineRunner for PrefixRunner<'_> {
    fn run(&self, parts: &[String]) -> bool {
        match wine_command(self.ctx, self.prep.probe, self.settings, parts.to_vec()) {
            Ok(cmd) => run_wait(self.prep.spawner, &cmd, Duration::from_secs(120), self.prep.cancel),
            Err(e) => {
                log::warn!("wine command {parts:?}: {e}");
                false
            }
        }
    }
}

impl Prep<'_> {
    fn notice(&self, n: Notice) {
        if n.warning {
            log::warn!("{}", n.message);
        } else {
            log::info!("{}", n.message);
        }
        (self.notify)(n);
    }

    /// `game.syncSaves(flag, path)` when online. Problems are warnings: the
    /// game starts anyway, like in Heroic.
    fn sync_saves(&self, app_name: &str, path: &str, flag: &str, what: &str) {
        let Some(cloud) = self.cloud else { return };
        if !self.net.is_online() {
            log::info!("offline, not {what} saves of {app_name}");
            return;
        }
        self.notice(Notice::info(format!("{} cloud saves...", if flag == "--skip-upload" { "Downloading" } else { "Uploading" })));
        match cloud.sync(app_name, path, flag) {
            Ok(out) => {
                if let Some(problem) = sync_problem(&out) {
                    self.notice(Notice::warn(format!("Cloud saves: {problem}")));
                } else {
                    log::info!("saves of {app_name} {what}");
                }
            }
            Err(e) => self.notice(Notice::warn(format!("Cloud saves failed: {e}"))),
        }
    }

    /// Before the prefix is created: cloud saves down, umu present.
    pub fn before_prefix(&self, app_name: &str, spec: &PrepSpec) -> Flow {
        if let Some(path) = &spec.cloud_saves {
            self.sync_saves(app_name, path, "--skip-upload", "downloaded");
            if (self.cancel)() {
                return Flow::Cancelled;
            }
        }
        if spec.umu {
            if let Err(e) = self.ensure_umu() {
                if e == CANCELLED {
                    return Flow::Cancelled;
                }
                return Flow::Fail(format!(
                    "The umu runtime is not installed and could not be downloaded ({e}). \
                     Enable \"Disable UMU\" in the game's settings to run Proton directly"
                ));
            }
        }
        Flow::Go
    }

    fn ensure_umu(&self) -> Result<(), String> {
        let script = runtimes::umu_script(self.paths);
        if self.probe.exists(&script.to_string_lossy()) {
            return Ok(());
        }
        if !self.net.is_online() {
            return Err("no internet connection".into());
        }
        self.notice(Notice::info("Downloading the umu runtime..."));
        runtimes::download(self.paths, self.net, Runtime::Umu, self.cancel)
    }

    /// After the prefix exists: DXVK & co. into Wine prefixes, EAC/BattlEye
    /// runtimes. Failures are warnings (Heroic shows a dialog and goes on).
    pub fn after_prefix(&self, spec: &PrepSpec) -> Flow {
        let Some(settings) = &spec.wine else { return Flow::Go };
        let ctx = PlanContext::load(self.paths);

        if settings.wine_version.kind == "wine" {
            let wanted = [
                (settings.auto_install_dxvk, Tool::Dxvk),
                (settings.auto_install_dxvk_nvapi, Tool::DxvkNvapi),
                (settings.auto_install_vkd3d, Tool::Vkd3d),
            ];
            for (_, tool) in wanted.iter().filter(|(on, _)| *on) {
                if (self.cancel)() {
                    return Flow::Cancelled;
                }
                match self.toggle_tool(&ctx, settings, *tool, Action::Install) {
                    Ok(dxvk::Outcome::Skipped(why)) => log::info!("{} not installed: {why}", tool.label()),
                    Ok(_) => {}
                    Err(e) if e == CANCELLED => return Flow::Cancelled,
                    Err(e) => self.notice(Notice::warn(format!("Could not install {}: {e}", tool.label()))),
                }
            }
        }

        for (on, rt) in [(settings.eac_runtime, Runtime::Eac), (settings.battleye_runtime, Runtime::BattlEye)] {
            if !on || (self.cancel)() {
                continue;
            }
            if runtimes::needs_download(self.paths, self.net, rt) {
                self.notice(Notice::info(format!("Downloading the {}...", rt.label())));
                match runtimes::download(self.paths, self.net, rt, self.cancel) {
                    Ok(()) => {}
                    Err(e) if e == CANCELLED => return Flow::Cancelled,
                    Err(e) => self.notice(Notice::warn(format!("Could not install the {}: {e}", rt.label()))),
                }
            }
        }
        Flow::Go
    }

    /// `DXVK.installRemove`: installs `tool` into (or removes it from) the
    /// prefix of `settings`. Also behind the toggles of the game settings.
    pub fn toggle_tool(
        &self,
        ctx: &PlanContext,
        settings: &GameSettings,
        tool: Tool,
        action: Action,
    ) -> Result<dxvk::Outcome, String> {
        let runner = PrefixRunner { ctx, prep: self, settings };
        let progress = |m: &str| self.notice(Notice::info(m));
        let env = dxvk::Env {
            paths: self.paths,
            net: self.net,
            probe: self.probe,
            runner: &runner,
            vulkan: tools::detect_vulkan(),
            cancel: self.cancel,
            progress: &progress,
        };
        dxvk::install_remove(&env, settings, tool, action)
    }

    /// After the game exited: upload the saves.
    pub fn after_exit(&self, app_name: &str, spec: &PrepSpec) {
        if let Some(path) = &spec.cloud_saves {
            self.sync_saves(app_name, path, "--skip-download", "uploaded");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::plan::Command;
    use crate::probe::FakeProbe;
    use crate::runtime::{ExitInfo, ProcessHandle};
    use crate::settings::WineVersion;
    use crate::tools::testing::{release, tarball};
    use crate::tools::Vulkan;
    use serde_json::json;
    use std::io;
    use std::path::Path;
    use std::sync::Mutex;

    /// Runs nothing: records the commands and "finishes" them at once.
    #[derive(Default)]
    pub struct InstantSpawner {
        pub spawned: Mutex<Vec<Command>>,
        pub exit_code: i32,
    }

    struct Done(i32);
    impl ProcessHandle for Done {
        fn pid(&self) -> u32 {
            1
        }
        fn poll(&mut self) -> io::Result<Option<ExitInfo>> {
            Ok(Some(ExitInfo { code: Some(self.0), signal: None }))
        }
        fn terminate(&mut self) {}
        fn kill(&mut self) {}
    }

    impl Spawner for InstantSpawner {
        fn spawn(&self, cmd: &Command, _log: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
            self.spawned.lock().unwrap().push(cmd.clone());
            Ok(Box::new(Done(self.exit_code)))
        }
        fn open_uri(&self, _uri: &str) -> io::Result<()> {
            Ok(())
        }
        fn kill_pattern(&self, _pattern: &str) {}
    }

    #[derive(Default)]
    struct FakeCloud {
        calls: Mutex<Vec<(String, String, String)>>,
        output: String,
        fail: bool,
    }
    impl CloudSync for FakeCloud {
        fn sync(&self, app: &str, path: &str, flag: &str) -> Result<String, String> {
            self.calls.lock().unwrap().push((app.into(), path.into(), flag.into()));
            if self.fail {
                Err("legendary is missing".into())
            } else {
                Ok(self.output.clone())
            }
        }
    }

    struct Fx {
        dir: tempfile::TempDir,
        paths: HeroicPaths,
        spawner: InstantSpawner,
    }

    impl Fx {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = HeroicPaths::at(dir.path().join("heroic"));
            Self { dir, paths, spawner: InstantSpawner::default() }
        }
    }

    fn collect<'a>(fx: &'a Fx, probe: &'a FakeProbe, net: &'a FakeNet, cloud: Option<&'a dyn CloudSync>, sink: &'a dyn Fn(Notice)) -> Prep<'a> {
        Prep { paths: &fx.paths, probe, net, spawner: &fx.spawner, cloud, cancel: &|| false, notify: sink }
    }

    fn wine_settings(prefix: &Path) -> GameSettings {
        GameSettings {
            wine_prefix: prefix.to_string_lossy().into_owned(),
            wine_version: WineVersion { bin: "/w/bin/wine".into(), name: "W".into(), kind: "wine".into(), ..Default::default() },
            ..Default::default()
        }
    }

    fn probe() -> FakeProbe {
        FakeProbe::new("/home/u").with_path("/w/bin/wine").with_path("/w/bin/wineserver")
    }

    #[test]
    fn cloud_saves_download_before_and_upload_after() {
        let fx = Fx::new();
        let cloud = FakeCloud::default();
        let (p, net) = (probe(), FakeNet::online());
        let seen = Mutex::new(Vec::new());
        let sink = |n: Notice| seen.lock().unwrap().push(n);
        let prep = collect(&fx, &p, &net, Some(&cloud), &sink);
        let spec = PrepSpec { cloud_saves: Some("/saves".into()), ..Default::default() };
        assert_eq!(prep.before_prefix("Calluna", &spec), Flow::Go);
        prep.after_exit("Calluna", &spec);
        assert_eq!(
            *cloud.calls.lock().unwrap(),
            [
                ("Calluna".to_string(), "/saves".to_string(), "--skip-upload".to_string()),
                ("Calluna".to_string(), "/saves".to_string(), "--skip-download".to_string())
            ]
        );
        let messages: Vec<String> = seen.lock().unwrap().iter().map(|n| n.message.clone()).collect();
        assert_eq!(messages, ["Downloading cloud saves...", "Uploading cloud saves..."]);
    }

    #[test]
    fn cloud_saves_are_skipped_offline_or_without_a_path() {
        let fx = Fx::new();
        let cloud = FakeCloud::default();
        let p = probe();
        let offline = FakeNet::offline();
        let sink = |_: Notice| {};
        let prep = collect(&fx, &p, &offline, Some(&cloud), &sink);
        let spec = PrepSpec { cloud_saves: Some("/saves".into()), ..Default::default() };
        prep.before_prefix("A", &spec);
        prep.after_exit("A", &spec);
        prep.before_prefix("A", &PrepSpec::default());
        assert!(cloud.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn cloud_save_problems_warn_but_never_stop_the_launch() {
        let fx = Fx::new();
        let (p, net) = (probe(), FakeNet::online());
        let spec = PrepSpec { cloud_saves: Some("/s".into()), ..Default::default() };
        for cloud in [
            FakeCloud { output: "[cli] INFO: x\n[cli] ERROR: Could not get remote saves\n".into(), ..Default::default() },
            FakeCloud { fail: true, ..Default::default() },
        ] {
            let seen = Mutex::new(Vec::new());
            let sink = |n: Notice| seen.lock().unwrap().push(n);
            let prep = collect(&fx, &p, &net, Some(&cloud), &sink);
            assert_eq!(prep.before_prefix("A", &spec), Flow::Go);
            let warnings: Vec<_> = seen.lock().unwrap().iter().filter(|n| n.warning).map(|n| n.message.clone()).collect();
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(warnings[0].starts_with("Cloud saves"), "{warnings:?}");
        }
        assert_eq!(sync_problem("all good\n[cli] INFO: done"), None);
        assert!(sync_problem("a\n  [Core] CRITICAL: boom").unwrap().contains("boom"));
    }

    fn lutris_list() -> serde_json::Value {
        json!([{"name": "umu", "url": "http://x/umu.tar.xz", "created_at": "T1"}])
    }

    #[test]
    fn missing_umu_is_downloaded_or_the_launch_fails_clearly() {
        let fx = Fx::new();
        let p = probe();
        let tar = tarball(fx.dir.path(), Some("umu"), &[("umu-run", "#!/bin/sh")]);
        let net = FakeNet::online().with_json(runtimes::RUNTIMES_URL, lutris_list()).with_file("http://x/umu.tar.xz", tar);
        let sink = |_: Notice| {};
        let spec = PrepSpec { umu: true, ..Default::default() };
        // The probe is the fake machine: tell it the script appears after the download.
        let prep = collect(&fx, &p, &net, None, &sink);
        assert_eq!(prep.before_prefix("g", &spec), Flow::Go);
        assert!(runtimes::runtimes_dir(&fx.paths).join("umu/umu-run").exists());

        // Offline with nothing installed.
        let fx2 = Fx::new();
        let offline = FakeNet::offline();
        let prep2 = collect(&fx2, &p, &offline, None, &sink);
        match prep2.before_prefix("g", &spec) {
            Flow::Fail(m) => assert!(m.contains("umu") && m.contains("Disable UMU"), "{m}"),
            other => panic!("{other:?}"),
        }

        // Already there: no request at all.
        let there = FakeProbe::new("/h").with_path(&runtimes::umu_script(&fx.paths).to_string_lossy());
        let quiet = FakeNet::online();
        let prep3 = collect(&fx, &there, &quiet, None, &sink);
        assert_eq!(prep3.before_prefix("g", &spec), Flow::Go);
        assert!(quiet.requests().is_empty());
    }

    /// A Wine prefix and an installed DXVK in the fixture heroic dir.
    fn dxvk_fixture(fx: &Fx) -> std::path::PathBuf {
        let prefix = fx.dir.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c/windows/system32")).unwrap();
        std::fs::create_dir_all(prefix.join("drive_c/windows/syswow64")).unwrap();
        std::fs::write(prefix.join(".update-timestamp"), "1").unwrap();
        let rel = tools::tool_dir(&fx.paths, Tool::Dxvk).join("dxvk-1");
        for d in ["x32", "x64"] {
            std::fs::create_dir_all(rel.join(d)).unwrap();
            std::fs::write(rel.join(d).join("d3d11.dll"), d).unwrap();
        }
        std::fs::write(tools::tool_dir(&fx.paths, Tool::Dxvk).join("latest_dxvk"), "dxvk-1").unwrap();
        prefix
    }

    #[test]
    fn dxvk_is_installed_into_wine_prefixes_through_the_games_wine() {
        let fx = Fx::new();
        let prefix = dxvk_fixture(&fx);
        let mut s = wine_settings(&prefix);
        s.auto_install_dxvk = true;
        let (p, net) = (probe(), FakeNet::offline());
        let seen = Mutex::new(Vec::new());
        let sink = |n: Notice| seen.lock().unwrap().push(n.message);
        let prep = collect(&fx, &p, &net, None, &sink);
        let spec = PrepSpec { wine: Some(s), ..Default::default() };
        assert_eq!(prep.after_prefix(&spec), Flow::Go);
        assert_eq!(std::fs::read_to_string(prefix.join("current_dxvk")).unwrap(), "dxvk-1");
        assert!(prefix.join("drive_c/windows/system32/d3d11.dll").exists());
        // The override went through the game's wine, in its prefix.
        let spawned = fx.spawner.spawned.lock().unwrap();
        assert_eq!(spawned.len(), 1);
        assert_eq!(spawned[0].program, "/w/bin/wine");
        assert_eq!(spawned[0].args[..2], ["reg", "add"]);
        assert_eq!(spawned[0].env["WINEPREFIX"], prefix.to_string_lossy());
        assert_eq!(*seen.lock().unwrap(), ["Installing DXVK in the prefix..."]);
    }

    #[test]
    fn the_settings_toggles_install_and_restore_through_the_same_path() {
        let fx = Fx::new();
        let prefix = dxvk_fixture(&fx);
        let s = wine_settings(&prefix);
        let (p, net) = (probe(), FakeNet::offline());
        let sink = |_: Notice| {};
        let prep = collect(&fx, &p, &net, None, &sink);
        let ctx = PlanContext::load(&fx.paths);
        let out = prep.toggle_tool(&ctx, &s, Tool::Dxvk, Action::Install).unwrap();
        assert_eq!(out, dxvk::Outcome::Installed("dxvk-1".into()));
        assert_eq!(prep.toggle_tool(&ctx, &s, Tool::Dxvk, Action::Restore).unwrap(), dxvk::Outcome::Removed);
        assert!(!prefix.join("current_dxvk").exists());
        assert!(!prefix.join("drive_c/windows/system32/d3d11.dll").exists());
        let spawned = fx.spawner.spawned.lock().unwrap();
        let last = spawned.last().unwrap();
        assert_eq!((last.program.as_str(), last.args.as_slice()), ("/w/bin/wine", ["wineboot".to_string(), "-u".to_string()].as_slice()));
    }

    #[test]
    fn nothing_is_installed_for_proton_or_when_the_toggles_are_off() {
        let fx = Fx::new();
        let prefix = dxvk_fixture(&fx);
        let (p, net) = (probe(), FakeNet::offline());
        let sink = |_: Notice| {};
        let prep = collect(&fx, &p, &net, None, &sink);
        let mut off = wine_settings(&prefix);
        off.auto_install_dxvk = false;
        let mut proton = wine_settings(&prefix);
        proton.auto_install_dxvk = true;
        proton.wine_version.kind = "proton".into();
        for s in [off, proton] {
            assert_eq!(prep.after_prefix(&PrepSpec { wine: Some(s), ..Default::default() }), Flow::Go);
        }
        assert!(!prefix.join("current_dxvk").exists());
        assert!(fx.spawner.spawned.lock().unwrap().is_empty());
        // No wine settings at all (native game): nothing happens.
        assert_eq!(prep.after_prefix(&PrepSpec::default()), Flow::Go);
    }

    #[test]
    fn a_missing_tool_is_downloaded_and_failures_only_warn() {
        let fx = Fx::new();
        let prefix = dxvk_fixture(&fx);
        std::fs::remove_dir_all(tools::tool_dir(&fx.paths, Tool::Dxvk)).unwrap();
        let mut s = wine_settings(&prefix);
        s.auto_install_dxvk = true;
        let p = probe();
        let spec = PrepSpec { wine: Some(s), ..Default::default() };

        // Offline: a warning, the launch goes on.
        let seen = Mutex::new(Vec::new());
        let sink = |n: Notice| seen.lock().unwrap().push(n);
        let offline = FakeNet::offline();
        assert_eq!(collect(&fx, &p, &offline, None, &sink).after_prefix(&spec), Flow::Go);
        assert!(seen.lock().unwrap().iter().any(|n| n.warning && n.message.contains("DXVK")));

        // Online: downloads the release first (Vulkan level comes from the machine, so serve both).
        let tar = tarball(fx.dir.path(), Some("dxvk-2"), &[("x32/d3d11.dll", "32"), ("x64/d3d11.dll", "64")]);
        let net = FakeNet::online()
            .with_json(Tool::Dxvk.release_url(Vulkan::Modern), release("dxvk-2.tar.gz", "http://x/dxvk-2.tar.gz"))
            .with_json(Tool::Dxvk.release_url(Vulkan::Legacy), release("dxvk-2.tar.gz", "http://x/dxvk-2.tar.gz"))
            .with_file("http://x/dxvk-2.tar.gz", tar);
        seen.lock().unwrap().clear();
        assert_eq!(collect(&fx, &p, &net, None, &sink).after_prefix(&spec), Flow::Go);
        assert_eq!(std::fs::read_to_string(prefix.join("current_dxvk")).unwrap(), "dxvk-2");
        let messages: Vec<String> = seen.lock().unwrap().iter().map(|n| n.message.clone()).collect();
        assert_eq!(messages, ["Downloading DXVK...", "Installing DXVK in the prefix..."]);
    }

    #[test]
    fn anti_cheat_runtimes_download_only_when_enabled_and_missing() {
        let fx = Fx::new();
        let p = probe();
        let tar = tarball(fx.dir.path(), Some("eac"), &[("v2/x", "x")]);
        let list = json!([
            {"name": "eac_runtime", "url": "http://x/eac.tar.xz", "created_at": "T"},
            {"name": "battleye_runtime", "url": "http://x/be.tar.xz", "created_at": "T"}
        ]);
        let net = FakeNet::online().with_json(runtimes::RUNTIMES_URL, list).with_file("http://x/eac.tar.xz", tar);
        let seen = Mutex::new(Vec::new());
        let sink = |n: Notice| seen.lock().unwrap().push(n);
        let prep = collect(&fx, &p, &net, None, &sink);
        let prefix = fx.dir.path().join("pfx");
        let mut s = wine_settings(&prefix);
        s.eac_runtime = true;
        s.battleye_runtime = true;
        let spec = PrepSpec { wine: Some(s.clone()), ..Default::default() };
        assert_eq!(prep.after_prefix(&spec), Flow::Go);
        assert!(runtimes::runtimes_dir(&fx.paths).join("eac_runtime/current_version").exists());
        // BattlEye's archive is not served: a warning, nothing else.
        assert!(!runtimes::runtimes_dir(&fx.paths).join("battleye_runtime").exists());
        let warnings: Vec<_> = seen.lock().unwrap().iter().filter(|n| n.warning).map(|n| n.message.clone()).collect();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("BattlEye"));

        // Off by default.
        let quiet = FakeNet::online();
        s.eac_runtime = false;
        s.battleye_runtime = false;
        let fx2 = Fx::new();
        let prep2 = collect(&fx2, &p, &quiet, None, &sink);
        prep2.after_prefix(&PrepSpec { wine: Some(s), ..Default::default() });
        assert!(quiet.requests().is_empty());
    }

    #[test]
    fn cancelling_stops_the_preparation() {
        let fx = Fx::new();
        let prefix = dxvk_fixture(&fx);
        let mut s = wine_settings(&prefix);
        s.auto_install_dxvk = true;
        let (p, net) = (probe(), FakeNet::offline());
        let sink = |_: Notice| {};
        let prep = Prep { cancel: &|| true, ..collect(&fx, &p, &net, None, &sink) };
        assert_eq!(prep.after_prefix(&PrepSpec { wine: Some(s), ..Default::default() }), Flow::Cancelled);
        assert!(!prefix.join("current_dxvk").exists());
    }

    #[test]
    fn run_wait_reports_success_failure_and_unstartable_commands() {
        let cmd = Command { program: "x".into(), ..Default::default() };
        let ok = InstantSpawner::default();
        assert!(run_wait(&ok, &cmd, Duration::from_secs(1), &|| false));
        let bad = InstantSpawner { exit_code: 1, ..Default::default() };
        assert!(!run_wait(&bad, &cmd, Duration::from_secs(1), &|| false));
    }
}
