//! Opening and closing a client. It runs with its own settings (Wine,
//! prefix, environment) and is not tied to Heronite: it stays open when the
//! app quits, like a game.

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use heronite_data::clients::prefix_of;
use heronite_data::proc::Run;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::settings::{load_game_settings, GameSettings};
use heronite_launch::wine_tools::{alongside_command, kill_prefix_command, run_exe_command, UMU_NSENTER};
use heronite_launch::{Command, PlanContext, Probe, SystemProbe};

use crate::{find_exe, spawn, Client};

/// How long closing the client may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);

/// What the client is started with.
fn open_args(client: Client) -> Vec<String> {
    match client {
        // Without it the window stays white under Proton.
        Client::BattleNet => vec!["--in-process-gpu".into()],
        Client::Ea | Client::Ubisoft | Client::Rockstar => Vec::new(),
    }
}

/// The client's prefix and the settings it runs with.
fn setup_of(paths: &DataPaths, client: Client, home: &str, probe: &dyn Probe) -> Result<(PathBuf, PlanContext, GameSettings), String> {
    let prefix = prefix_of(paths, client, home).ok_or_else(|| not_set_up(client))?;
    let ctx = PlanContext::load(paths);
    let settings = load_game_settings(paths, client.config_app(), &ctx.global, probe);
    Ok((prefix, ctx, settings))
}

fn not_set_up(client: Client) -> String {
    tr!("{client} is not set up. Set it up in Stores → {client}.", client = client.name())
}

/// The command that opens the client's window: the client started with its
/// Wine (umu's container then takes the later requests of its games), or,
/// while it runs, a request to the open client.
pub fn open_command(paths: &DataPaths, client: Client, home: &str) -> Result<Command, String> {
    command_with(paths, client, home, &SystemProbe, &SystemProcs)
}

fn command_with(paths: &DataPaths, client: Client, home: &str, probe: &dyn Probe, procs: &dyn Procs) -> Result<Command, String> {
    let (prefix, ctx, settings) = setup_of(paths, client, home, probe)?;
    let exe = find_exe(client, &prefix)
        .ok_or_else(|| tr!("{client} is not installed in its prefix yet. Set it up in Stores → {client}.", client = client.name()))?;
    if settings.sandbox.enabled {
        return Err(tr!("The sandbox does not run {client}. Turn it off in Stores → {client}.", client = client.name()));
    }
    let exe = exe.to_string_lossy().into_owned();
    if running_in(procs, client, &prefix).0 {
        return alongside_command(&ctx, probe, client.config_app(), "sideload", &settings, &exe, true);
    }
    let mut command = run_exe_command(&ctx, probe, &settings, &exe, &open_args(client))?.command;
    // Its games, started later, join this container.
    command.env.insert(UMU_NSENTER.into(), "1".into());
    command.env.insert("PROTON_VERB".into(), "run".into());
    Ok(command)
}

/// Starts [`open_command`] in a process group of its own; the child is the
/// caller's to wait for (it lives as long as the client when it started it).
pub fn open(paths: &DataPaths, client: Client, home: &str) -> Result<std::process::Child, String> {
    let command = open_command(paths, client, home)?;
    let mut cmd = spawn::std_command(&command);
    // Not `proc::guard`: the client outlives the app.
    cmd.stdin(Stdio::null()).process_group(0);
    let log = spawn::create_log(&spawn::log_path(paths, &format!("{}.log", client.id()))).ok();
    let (out, err) = match log {
        Some(mut file) => {
            let _ = writeln!(file, "$ {}", command.display());
            match file.try_clone() {
                Ok(second) => (Stdio::from(file), Stdio::from(second)),
                Err(_) => (Stdio::from(file), Stdio::null()),
            }
        }
        None => (Stdio::null(), Stdio::null()),
    };
    cmd.stdout(out).stderr(err);
    heronite_data::proc::spawn(&mut cmd).map_err(|e| tr!("Could not start {client}: {error}", client = client.name(), error = e))
}

/// `(the client's main program runs, a program that is not the client's
/// runs)` in `prefix`, from one look at the processes.
pub fn running(client: Client, prefix: &Path) -> (bool, bool) {
    running_in(&SystemProcs, client, prefix)
}

/// Whether a program other than the client's runs in `prefix` (one of its
/// games, most likely).
pub fn game_running(client: Client, prefix: &Path) -> bool {
    running_in(&SystemProcs, client, prefix).1
}

/// `(the client's main program runs, a program that is not the client's
/// runs)` in `prefix`.
fn running_in(procs: &dyn Procs, client: Client, prefix: &Path) -> (bool, bool) {
    let programs = procs.prefix_programs(&prefix.to_string_lossy(), None);
    let main = programs.iter().any(|p| p.name == client.main_program());
    let game = programs.iter().any(|p| !client.programs().contains(&p.name.as_str()));
    (main, game)
}

/// Closes the client and everything running in its prefix
/// (`wineserver -k`), waiting for it (a few seconds).
pub fn close(paths: &DataPaths, client: Client, home: &str) -> Result<(), String> {
    let (_, ctx, settings) = setup_of(paths, client, home, &SystemProbe)?;
    let command = kill_prefix_command(&ctx, &SystemProbe, &settings)?;
    let out = Run::new()
        .timeout(CLOSE_TIMEOUT)
        .no_capture()
        .run(&mut spawn::std_command(&command))
        .map_err(|e| tr!("Could not close {client}: {error}", client = client.name(), error = e))?;
    // The server exits with an error when there is none to stop: the
    // client is closed all the same.
    if out.timed_out {
        return Err(tr!("{client} did not close in time", client = client.name()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_launch::probe::FakeProbe;
    use heronite_launch::procs::FakeProcs;
    use serde_json::json;

    const PROTON: &str = "/p/GE/proton";

    struct Rig {
        dir: tempfile::TempDir,
        paths: DataPaths,
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::at(dir.path());
            Self { dir, paths }
        }

        fn prefix(&self) -> PathBuf {
            self.dir.path().join("prefix")
        }

        /// The client's settings file, as a setup writes it.
        fn set_up(&self, client: Client, extra: serde_json::Value) {
            let app = client.config_app();
            let mut section = json!({"winePrefix": self.prefix(), "wineVersion": {"bin": PROTON, "name": "GE-Proton", "type": "proton"}});
            section.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            std::fs::create_dir_all(self.paths.games_config_dir()).unwrap();
            std::fs::write(self.paths.game_config(app), json!({ app: section, "version": "v0" }).to_string()).unwrap();
        }

        /// The client's program in the prefix.
        fn install(&self, client: Client) -> PathBuf {
            let exe = self.prefix().join("drive_c").join(client.exe_candidates()[0]);
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, "MZ").unwrap();
            exe
        }

        fn umu(&self) -> String {
            PlanContext::load(&self.paths).runtime_dir() + "/umu/umu_run.py"
        }

        fn probe(&self, exe: &Path) -> FakeProbe {
            FakeProbe::new("/home/u").with_path(PROTON).with_path(&self.umu()).with_path(&exe.to_string_lossy())
        }
    }

    #[test]
    fn a_client_that_is_not_set_up_or_installed_says_so() {
        let rig = Rig::new();
        let probe = FakeProbe::new("/home/u");
        let err = command_with(&rig.paths, Client::BattleNet, "/home/u", &probe, &FakeProcs::new()).unwrap_err();
        assert_eq!(err, "Battle.net is not set up. Set it up in Stores → Battle.net.");
        rig.set_up(Client::BattleNet, json!({}));
        let err = command_with(&rig.paths, Client::BattleNet, "/home/u", &probe, &FakeProcs::new()).unwrap_err();
        assert_eq!(err, "Battle.net is not installed in its prefix yet. Set it up in Stores → Battle.net.");
        assert!(open(&rig.paths, Client::BattleNet, "/home/u").is_err());
        assert!(close(&rig.paths, Client::Ea, "/home/u").is_err());
    }

    #[test]
    fn the_client_is_started_with_its_wine_and_asked_to_keep_its_container() {
        let rig = Rig::new();
        rig.set_up(Client::BattleNet, json!({}));
        let exe = rig.install(Client::BattleNet);
        let command = command_with(&rig.paths, Client::BattleNet, "/home/u", &rig.probe(&exe), &FakeProcs::new()).unwrap();
        let exe = exe.to_string_lossy().into_owned();
        assert_eq!(command.program, rig.umu());
        assert_eq!(command.args, [exe.as_str(), "--in-process-gpu"]);
        assert_eq!(command.env["PROTON_VERB"], "run");
        assert_eq!(command.env[UMU_NSENTER], "1");
        assert_eq!(command.env["WINEPREFIX"], rig.prefix().to_string_lossy());
        assert_eq!(command.cwd.as_deref(), Some(Path::new(&exe).parent().unwrap().to_str().unwrap()));

        // The other clients need no arguments.
        rig.set_up(Client::Rockstar, json!({}));
        let exe = rig.install(Client::Rockstar);
        let command = command_with(&rig.paths, Client::Rockstar, "/home/u", &rig.probe(&exe), &FakeProcs::new()).unwrap();
        assert_eq!(command.args, [exe.to_string_lossy().as_ref()]);
    }

    #[test]
    fn while_it_runs_the_request_joins_the_open_client() {
        let rig = Rig::new();
        rig.set_up(Client::Ubisoft, json!({}));
        let exe = rig.install(Client::Ubisoft);
        let procs = FakeProcs::new();
        procs.start(20, "wine", None);
        procs.set_prefix_programs(&[(20, "upc.exe")]);
        let command = command_with(&rig.paths, Client::Ubisoft, "/home/u", &rig.probe(&exe), &procs).unwrap();
        assert_eq!(command.args, [exe.to_string_lossy().as_ref()]);
        assert_eq!(command.env["PROTON_VERB"], "runinprefix");
        assert_eq!(command.env[UMU_NSENTER], "1");
    }

    #[test]
    fn a_client_in_the_sandbox_is_refused() {
        let rig = Rig::new();
        rig.set_up(Client::Ea, json!({"heroniteSandbox": {"enabled": true}}));
        let exe = rig.install(Client::Ea);
        let err = command_with(&rig.paths, Client::Ea, "/home/u", &rig.probe(&exe), &FakeProcs::new()).unwrap_err();
        assert_eq!(err, "The sandbox does not run EA app. Turn it off in Stores → EA app.");
    }

    /// A Proton that records how it was called and what it was asked, so
    /// the client can be opened and closed for real.
    fn set_up_with_fake_proton(rig: &Rig, client: Client) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = rig.dir.path().join("proton");
        let body =
            "#!/bin/sh\necho \"$@\" >> \"$WINEPREFIX/calls.txt\"\necho \"verb=$PROTON_VERB nsenter=$UMU_CONTAINER_NSENTER\"\nsleep 1\n";
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let app = client.config_app();
        let wine = json!({"bin": script, "name": "Fake", "type": "proton"});
        let section = json!({"winePrefix": rig.prefix(), "wineVersion": wine, "disableUMU": true});
        std::fs::create_dir_all(rig.paths.games_config_dir()).unwrap();
        std::fs::write(rig.paths.game_config(app), json!({ app: section, "version": "v0" }).to_string()).unwrap();
        std::fs::create_dir_all(rig.prefix()).unwrap();
        rig.install(client)
    }

    #[test]
    fn opens_the_client_in_a_group_of_its_own_with_its_output_in_a_log() {
        let rig = Rig::new();
        let exe = set_up_with_fake_proton(&rig, Client::BattleNet);
        let mut child = open(&rig.paths, Client::BattleNet, "/home/u").unwrap();
        // The app's process group is not the client's, so quitting the app leaves it.
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        let group: u32 = stat.rsplit(')').next().unwrap().split_whitespace().nth(2).unwrap().parse().unwrap();
        assert_eq!(group, child.id());
        child.wait().unwrap();
        let log = std::fs::read_to_string(rig.dir.path().join("logs/clients/battlenet.log")).unwrap();
        assert!(log.starts_with("$ "), "{log}");
        assert!(log.contains("verb=run nsenter=1"), "{log}");
        let calls = std::fs::read_to_string(rig.prefix().join("calls.txt")).unwrap();
        assert_eq!(calls.trim(), format!("run {} --in-process-gpu", exe.display()));

        // Each start begins a log of its own.
        open(&rig.paths, Client::BattleNet, "/home/u").unwrap().wait().unwrap();
        let again = std::fs::read_to_string(rig.dir.path().join("logs/clients/battlenet.log")).unwrap();
        assert_eq!(again.matches("verb=run").count(), 1);
    }

    #[test]
    fn closing_asks_the_prefix_to_end_its_programs() {
        let rig = Rig::new();
        set_up_with_fake_proton(&rig, Client::Ea);
        close(&rig.paths, Client::Ea, "/home/u").unwrap();
        let calls = std::fs::read_to_string(rig.prefix().join("calls.txt")).unwrap();
        assert_eq!(calls.trim(), "run wineboot -k");
    }

    #[test]
    fn tells_the_client_from_its_games_by_program() {
        let prefix = Path::new("/p");
        let procs = FakeProcs::new();
        assert_eq!(running_in(&procs, Client::BattleNet, prefix), (false, false));
        for pid in [1, 2, 3] {
            procs.start(pid, "wine", None);
        }
        // Helpers of the client are neither the client nor a game.
        procs.set_prefix_programs(&[(1, "battle.net helper.exe"), (2, "agent.exe")]);
        assert_eq!(running_in(&procs, Client::BattleNet, prefix), (false, false));
        procs.set_prefix_programs(&[(1, "battle.net.exe"), (2, "agent.exe")]);
        assert_eq!(running_in(&procs, Client::BattleNet, prefix), (true, false));
        procs.set_prefix_programs(&[(1, "battle.net.exe"), (3, "wow.exe")]);
        assert_eq!(running_in(&procs, Client::BattleNet, prefix), (true, true));
        procs.set_prefix_programs(&[(3, "wow.exe")]);
        assert_eq!(running_in(&procs, Client::BattleNet, prefix), (false, true));
        // Nothing runs under a prefix no process uses.
        assert_eq!(running(Client::BattleNet, Path::new("/nowhere/at/all")), (false, false));
        assert!(!game_running(Client::BattleNet, Path::new("/nowhere/at/all")));
    }
}
