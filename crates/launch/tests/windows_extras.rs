//! Real Windows scripts and native programs, all confined to temporary folders.
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::models::GameInfo;
use heronite_data::testing::{FakeTool, Reply};
use heronite_data::DataPaths;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::settings::{GameSettings, GlobalSettings};
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    companions, scripts, Command, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, PlanContext, ProcessPlan, SystemProbe,
    SystemSpawner,
};
use serde_json::json;

struct Rig {
    launcher: Launcher,
    events: Receiver<LaunchEvent>,
    game: FakeTool,
    folder: PathBuf,
    _temp: tempfile::TempDir,
}

impl Rig {
    fn new(natural_exit: bool, companion_delay: u32) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Game with spaces \u{e9}");
        fs::create_dir_all(&folder).unwrap();
        let game = FakeTool::new(&folder, "Native game").otherwise(
            Reply::new()
                .write_file("game.started", "started")
                .sleep(Duration::from_secs(if natural_exit { 10 } else { 30 }))
                .write_file("game.finished", "finished"),
        );
        let before = folder.join("Before game.bat");
        fs::write(
            &before,
            "@echo off\r\nif exist game.started exit /b 1\r\n\
             >before.txt cmd /d /u /c echo %HERONITE_GAME_SCRIPT_TYPE%\r\n\
             >>before.txt cmd /d /u /c echo %HEROIC_GAME_TITLE%\r\n\
             >>before.txt cmd /d /u /c cd\r\n",
        )
        .unwrap();
        let companion = folder.join("Companion program.ps1");
        fs::write(
            &companion,
            "$ErrorActionPreference = 'Stop'\n\
             $deadline = [DateTime]::UtcNow.AddSeconds(20)\n\
             while (-not (Test-Path game.started)) {\n\
               if ([DateTime]::UtcNow -ge $deadline) { throw 'game never started' }\n\
               Start-Sleep -Milliseconds 10\n\
             }\n\
             @($env:HERONITE_GAME_TITLE, $env:HEROIC_GAME_APP_NAME, (Get-Location).Path) | Set-Content -Encoding Unicode companion.txt\n\
             Set-Content companion.pid $PID\n\
             Start-Sleep -Seconds 30\n\
             Set-Content companion.finished 'unexpected'\n",
        )
        .unwrap();
        let after = folder.join("After game.ps1");
        fs::write(
            &after,
            "$ErrorActionPreference = 'Stop'\n\
             if (-not (Test-Path before.txt)) { throw 'before script did not finish' }\n\
             @($env:HERONITE_GAME_SCRIPT_TYPE, $env:HEROIC_GAME_TITLE, (Get-Location).Path) | Set-Content -Encoding Unicode after.txt\n",
        )
        .unwrap();
        let paths = DataPaths::at(temp.path().join("data"));
        let home = temp.path().join("home");
        let steam = SteamLocator::detect(&GlobalSettings::default(), &heronite_launch::probe::FakeProbe::new(&home.to_string_lossy()));
        let (tx, events) = channel();
        let procs = Arc::new(SystemProcs);
        let launcher = Launcher::new(
            paths.clone(),
            steam,
            Arc::new(SystemSpawner::new(procs.clone())),
            procs,
            LauncherConfig {
                poll_interval: Duration::from_millis(10),
                kill_grace: Duration::from_millis(200),
                min_session_seconds: 0,
                ..Default::default()
            },
            tx,
        );
        let info: GameInfo = serde_json::from_value(json!({
            "runner": "sideload", "app_name": "native-extras", "title": "Native test game",
            "install": {"executable": game.bin(), "platform": "Windows", "install_path": folder}
        }))
        .unwrap();
        let settings = GameSettings::from_map(
            json!({
                "beforeLaunchScriptPath": before,
                "afterLaunchScriptPath": after,
                "heroniteCompanions": [{"path": companion, "delay": companion_delay}]
            })
            .as_object()
            .unwrap(),
            &home.to_string_lossy(),
        );
        let mut plan = LaunchPlan::Native(ProcessPlan {
            command: Command {
                program: game.bin().to_string_lossy().into_owned(),
                cwd: Some(folder.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        });
        scripts::attach(&mut plan, &SystemProbe, &info, &settings);
        companions::attach(&mut plan, &PlanContext::load(&paths), &SystemProbe, &info, &settings);
        launcher.launch(LaunchRequest { app_name: info.app_name, runner: info.runner, title: info.title, plan }).unwrap();
        Self { launcher, events, game, folder, _temp: temp }
    }

    fn wait_file(&self, name: &str) -> PathBuf {
        let path = self.folder.join(name);
        wait_until(name, || path.is_file() && fs::metadata(&path).is_ok_and(|m| m.len() > 0));
        path
    }

    fn exited(&self) -> bool {
        loop {
            match self.events.recv_timeout(Duration::from_secs(40)).expect("launch event") {
                LaunchEvent::Exited { stopped, exit, record, .. } => {
                    if !stopped {
                        assert_eq!(exit.unwrap().code, Some(0));
                    }
                    assert!(record.is_some(), "the session was not recorded");
                    return stopped;
                }
                LaunchEvent::Error { message, .. } => panic!("launch failed: {message}"),
                _ => {}
            }
        }
    }

    fn companion_pid(&self) -> u32 {
        fs::read_to_string(self.wait_file("companion.pid")).unwrap().trim().parse().unwrap()
    }

    fn assert_scripts(&self) {
        for (file, kind) in [("before.txt", "before"), ("after.txt", "after")] {
            let text = read_script_output(&self.folder.join(file));
            let lines: Vec<_> = text.lines().map(str::trim).collect();
            assert_eq!(lines.len(), 3, "{file}: {text}");
            assert_eq!(lines[0], kind);
            assert_eq!(lines[1], "Native test game");
            // Windows script hosts can expand 8.3 names inherited through TEMP.
            assert_eq!(fs::canonicalize(lines[2]).unwrap(), fs::canonicalize(&self.folder).unwrap());
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Stop only this test's session, including on an assertion failure.
        self.launcher.stop("native-extras");
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.launcher.is_running("native-extras") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn read_script_output(path: &Path) -> String {
    // cmd /u and PowerShell -Encoding Unicode emit UTF-16LE; only the latter adds a BOM.
    let bytes = fs::read(path).unwrap();
    let (pairs, remainder) = bytes.as_chunks::<2>();
    assert!(remainder.is_empty(), "incomplete UTF-16 output: {}", path.display());
    let units: Vec<_> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16(units.strip_prefix(&[0xfeff]).unwrap_or(&units)).unwrap()
}

#[test]
fn scripts_and_companion_follow_a_native_games_natural_exit() {
    let rig = Rig::new(true, 1);
    let companion = rig.companion_pid();
    assert!(SystemProcs.is_alive(companion));
    assert!(!rig.folder.join("after.txt").exists(), "after script ran while the game was alive");
    assert!(!rig.exited());
    assert!(rig.folder.join("game.finished").is_file());
    wait_until("companion stopped", || !SystemProcs.is_alive(companion));
    assert!(!rig.folder.join("companion.finished").exists());
    rig.assert_scripts();
    let text = read_script_output(&rig.folder.join("companion.txt"));
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3, "companion.txt: {text}");
    assert_eq!(&lines[..2], ["Native test game", "native-extras"]);
    assert_eq!(fs::canonicalize(lines[2]).unwrap(), fs::canonicalize(&rig.folder).unwrap());
    assert_eq!(rig.game.calls().len(), 1);
}

#[test]
fn stopping_a_native_game_closes_its_companion_and_runs_the_after_script() {
    let rig = Rig::new(false, 0);
    let companion = rig.companion_pid();
    let game = rig.launcher.pid_of("native-extras").unwrap();
    assert!(SystemProcs.is_alive(game) && SystemProcs.is_alive(companion));
    assert!(rig.launcher.stop("native-extras"));
    assert!(rig.exited());
    wait_until("owned processes stopped", || !SystemProcs.is_alive(game) && !SystemProcs.is_alive(companion));
    assert!(!rig.folder.join("game.finished").exists());
    assert!(!rig.folder.join("companion.finished").exists());
    rig.assert_scripts();
}

#[test]
fn stopping_before_a_companions_delay_cancels_it() {
    let rig = Rig::new(false, 60);
    rig.wait_file("game.started");
    assert!(rig.launcher.stop("native-extras"));
    assert!(rig.exited());
    assert!(!rig.folder.join("companion.pid").exists());
    assert!(!rig.folder.join("companion.txt").exists());
    rig.assert_scripts();
}
