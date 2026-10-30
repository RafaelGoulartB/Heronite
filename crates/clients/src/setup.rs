//! Installing a client into a Wine prefix of its own: download its
//! installer, create the prefix with the default Wine, write the settings
//! that make the client behave under Wine, run the installer silently. Or
//! use a client already installed in a prefix of the user's.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::clients::{drive_c, prefix_of, wine_prefix};
use heronite_data::game_config::{self, Settings};
use heronite_data::proc::Cancel;
use heronite_data::DataPaths;
use heronite_epic::third_party::{self, Provider};
use heronite_i18n::tr;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::settings::{load_game_settings, GameSettings, GlobalSettings};
use heronite_launch::wine::{valid_wine, CREATING_MARKER};
use heronite_launch::wine_tools::{create_prefix_commands, run_exe_command, UMU_NSENTER};
use heronite_launch::{PlanContext, SystemProbe};
use serde_json::{json, Value};

use crate::spawn::{self, Installer, Waits};
use crate::winpath::join_ci;
use crate::{find_exe, user_dirs, Client};

/// Where a setup is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Downloading the installer; `0..=100`, `None` while the size is unknown.
    Downloading(Option<f64>),
    CreatingPrefix,
    /// The client's installer runs (it may download the client itself).
    Installing,
}

const BATTLENET_URL: &str = "https://downloader.battle.net/download/getInstaller?os=win&installer=Battle.net-Setup.exe";
const ROCKSTAR_URL: &str = "https://gamedownloads.rockstargames.com/public/installer/Rockstar-Games-Launcher.exe";

/// The installers' programs, as Wine names their processes. While one runs
/// in the prefix the setup is not done.
const INSTALLER_PROGRAMS: [&str; 5] =
    ["battle.net-setup.exe", "eaappinstaller.exe", "msiexec.exe", "ubisoftconnectinstaller.exe", "rockstar-games-launcher.exe"];

const PREFIX_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Battle.net's settings under Wine: its game window stays out of the way,
/// and no hardware acceleration or streaming (they leave a white window).
const BATTLENET_CONFIG: &str =
    r#"{"Client": {"GameLaunchWindowBehavior": "2", "HardwareAcceleration": "false", "Streaming": {"StreamingEnabled": "false"}}}"#;

/// Ubisoft Connect's: no overlay (it hooks into the games and crashes
/// them under Wine), and closing the window closes the launcher.
const UBISOFT_SETTINGS: &str =
    "overlay:\n  enabled: false\n  forceunhookgame: false\n  fps_enabled: false\n  warning_enabled: false\nuser:\n  closebehavior: CloseBehavior_Close\n";

/// The prefix a new setup gets: `<defaultWinePrefixDir>/<Client::prefix_folder>`.
pub fn default_prefix(paths: &DataPaths, client: Client, home: &str) -> PathBuf {
    let global = game_config::load_global(paths, home);
    let dir = global
        .get("defaultWinePrefixDir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| game_config::expand_tilde(d, home))
        .unwrap_or_else(|| game_config::default_wine_prefix_dir(home));
    PathBuf::from(dir).join(client.prefix_folder())
}

/// The default Wine (`wineVersion` of the global settings) when it can run
/// programs: the setup needs one. The app gets Proton when there is none.
pub fn has_wine(paths: &DataPaths, home: &str) -> bool {
    default_wine(paths, home).is_some()
}

/// The global `wineVersion` as saved, when that Wine is usable.
fn default_wine(paths: &DataPaths, home: &str) -> Option<Value> {
    let global: Settings = game_config::load_global(paths, home);
    let wine = global.get("wineVersion")?.clone();
    valid_wine(&GlobalSettings::from_defaults(global).wine_version(), &SystemProbe).then_some(wine)
}

/// Installs `client` into [`default_prefix`] with the default Wine
/// (blocking; minutes). Its settings file is written first, so a setup cut
/// short is picked up again by the next one. Fails when no Wine or Proton
/// is installed, when cancelled, or when the client's program is not there
/// once the installer is done.
pub fn install(paths: &DataPaths, client: Client, home: &str, cancel: &Cancel, on_step: &mut dyn FnMut(Step)) -> Result<()> {
    let wine = default_wine(paths, home).ok_or_else(|| anyhow!(tr!("No Wine or Proton is installed. Get one in the Wine Manager.")))?;
    let prefix = match prefix_of(paths, client, home) {
        Some(prefix) => prefix,
        None => {
            let prefix = default_prefix(paths, client, home);
            let changes = [("winePrefix".to_string(), json!(prefix.to_string_lossy())), ("wineVersion".to_string(), wine)];
            game_config::set_game_settings(paths, client.config_app(), home, &changes)?;
            prefix
        }
    };
    let log_file = spawn::log_path(paths, &format!("{}-setup.log", client.id()));
    let mut log = spawn::create_log(&log_file).with_context(|| log_file.display().to_string())?;

    let installer = get_installer(paths, client, cancel, on_step)?;
    check_cancel(cancel)?;

    let ctx = PlanContext::load(paths);
    let settings = load_game_settings(paths, client.config_app(), &ctx.global, &SystemProbe);
    if prefix_missing(&prefix) {
        on_step(Step::CreatingPrefix);
        create_prefix(&ctx, &settings, &prefix, cancel, &mut log, &log_file)?;
        if prefix_missing(&prefix) {
            bail!(tr!("The Wine prefix could not be created. See {log}", log = log_file.display()));
        }
    }
    check_cancel(cancel)?;

    write_client_settings(client, &prefix);
    on_step(Step::Installing);
    let mut command = run_exe_command(&ctx, &SystemProbe, &settings, &installer.to_string_lossy(), &installer_args(client))
        .map_err(|e| anyhow!(e))?
        .command;
    // A client the installer starts takes the requests of its games later.
    command.env.insert(UMU_NSENTER.into(), "1".into());
    let _ = writeln!(log, "$ {}", command.display());
    let mut process = spawn::std_command(&command);
    heronite_platform::process::new_group(process.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log.try_clone()?));
    // Not tied to the app: the installer may become the client.
    let child = heronite_data::proc::spawn(&mut process)
        .map_err(|e| anyhow!(tr!("Could not start {program}: {error}", program = command.program, error = e)))?;
    let prefix_name = prefix.to_string_lossy().into_owned();
    let installed = || find_exe(client, &prefix).is_some();
    let installer_state =
        Installer { running: &|| installer_running(&SystemProcs, &prefix_name, !installed()), client_installed: &installed };
    spawn::wait_for_installer(child, &SystemProcs, &installer_state, cancel, &Waits::default(), client.name())?;

    if find_exe(client, &prefix).is_none() {
        bail!(tr!("{client} was not installed. The installer's log is in {log}", client = client.name(), log = log_file.display()));
    }
    Ok(())
}

fn check_cancel(cancel: &Cancel) -> Result<()> {
    if cancel.is_requested() {
        bail!(tr!("Cancelled"));
    }
    Ok(())
}

/// The client's installer, downloaded once and kept.
fn get_installer(paths: &DataPaths, client: Client, cancel: &Cancel, on_step: &mut dyn FnMut(Step)) -> Result<PathBuf> {
    let mut progress = |percent: f64| on_step(Step::Downloading((percent > 0.0).then_some(percent)));
    let failed = |e: anyhow::Error| {
        if cancel.is_requested() {
            anyhow!(tr!("Cancelled"))
        } else {
            anyhow!(tr!("Could not download the {client} installer: {error}", client = client.name(), error = e))
        }
    };
    let (file, url) = match client {
        Client::Ea | Client::Ubisoft => {
            let provider = if client == Client::Ea { Provider::Ea } else { Provider::Ubisoft };
            if !third_party::redist_dir(paths).join(provider.installer_file()).is_file() {
                progress(0.0);
            }
            return third_party::ensure_installer(paths, provider, cancel, &mut |p| progress(p.percent)).map_err(failed);
        }
        Client::BattleNet => ("Battle.net-Setup.exe", BATTLENET_URL),
        Client::Rockstar => ("Rockstar-Games-Launcher.exe", ROCKSTAR_URL),
    };
    let dir = paths.tools_dir().join("redist").join("clients");
    let dest = dir.join(file);
    if dest.is_file() {
        return Ok(dest);
    }
    std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    progress(0.0);
    third_party::download_to(url, &dest, cancel, &mut |p| progress(p.percent)).map_err(failed)?;
    Ok(dest)
}

/// Whether the prefix is not there, or a creation was cut short.
fn prefix_missing(prefix: &Path) -> bool {
    let wine = wine_prefix(prefix);
    prefix.join(CREATING_MARKER).exists() || !drive_c(prefix).is_dir() || ["system.reg", "user.reg"].iter().any(|f| !wine.join(f).is_file())
}

/// Creates the prefix with the client's Wine. A marker stays in it while
/// that lasts, so a creation that was cut short is made again.
fn create_prefix(
    ctx: &PlanContext,
    settings: &GameSettings,
    prefix: &Path,
    cancel: &Cancel,
    log: &mut std::fs::File,
    log_file: &Path,
) -> Result<()> {
    std::fs::create_dir_all(prefix).with_context(|| prefix.display().to_string())?;
    let marker = prefix.join(CREATING_MARKER);
    let _ = std::fs::remove_file(&marker);
    if let Err(e) = std::fs::OpenOptions::new().write(true).create_new(true).open(&marker) {
        log::debug!("{}: {e}", marker.display());
    }
    for command in create_prefix_commands(ctx, &SystemProbe, settings).map_err(|e| anyhow!(e))? {
        if !spawn::run_to_completion(&command, cancel, PREFIX_TIMEOUT, log)? {
            bail!(tr!("The Wine prefix could not be created. See {log}", log = log_file.display()));
        }
    }
    let _ = std::fs::remove_file(&marker);
    Ok(())
}

/// What the client's installer is given.
fn installer_args(client: Client) -> Vec<String> {
    let args: &[&str] = match client {
        Client::BattleNet => &["--lang=enUS", "--installpath=C:\\Program Files (x86)\\Battle.net"],
        Client::Ea => &["/quiet", "EAX_LAUNCH_CLIENT=0", "IGNORE_INSTALLED=1"],
        Client::Ubisoft => &["/S"],
        Client::Rockstar => &["/s", "/f"],
    };
    args.iter().map(|a| a.to_string()).collect()
}

/// Whether the setup still runs in the prefix: a program of an installer,
/// or, while the client's program is not there yet, any program (Battle.net's
/// setup hands over to its Agent, which downloads the client).
fn installer_running(procs: &dyn Procs, prefix: &str, client_missing: bool) -> bool {
    procs.prefix_programs(prefix, None).iter().any(|p| client_missing || INSTALLER_PROGRAMS.contains(&p.name.as_str()))
}

/// Writes the settings that make the client work under Wine into each
/// user's folder of the prefix, where the user has none yet.
fn write_client_settings(client: Client, prefix: &Path) {
    let (file, text) = match client {
        Client::BattleNet => ("AppData/Roaming/Battle.net/Battle.net.config", BATTLENET_CONFIG),
        Client::Ubisoft => ("AppData/Local/Ubisoft Game Launcher/settings.yaml", UBISOFT_SETTINGS),
        Client::Ea | Client::Rockstar => return,
    };
    for user in user_dirs(prefix) {
        let path = join_ci(&user, file);
        if path.exists() {
            continue;
        }
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, text));
        if let Err(e) = written {
            log::warn!("{}: {e}", path.display());
        }
    }
}

/// Uses `client` already installed in `prefix` (a prefix of Lutris, Bottles
/// or the user's): its settings point there, with the default Wine. Fails
/// when the client's program is not in that prefix.
pub fn adopt(paths: &DataPaths, client: Client, home: &str, prefix: &Path) -> Result<()> {
    if find_exe(client, prefix).is_none() {
        bail!(tr!("{client} is not installed in {prefix}", client = client.name(), prefix = prefix.display()));
    }
    let mut changes = vec![("winePrefix".to_string(), json!(prefix.to_string_lossy()))];
    if !game_config::game_overrides(paths, client.config_app()).contains_key("wineVersion") {
        if let Some(wine) = game_config::load_global(paths, home).get("wineVersion") {
            changes.push(("wineVersion".to_string(), wine.clone()));
        }
    }
    game_config::set_game_settings(paths, client.config_app(), home, &changes)
}

/// Forgets `client`: its settings file goes, its prefix (and the games in
/// it) stays on disk.
pub fn forget(paths: &DataPaths, client: Client) -> Result<()> {
    let file = paths.game_config(client.config_app());
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| file.display().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_launch::procs::FakeProcs;

    const HOME: &str = "/home/tester";

    fn paths() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths)
    }

    /// A Proton that makes a prefix and puts the client in it, for the test
    /// to run a whole setup without Wine. Unix only: it needs `sh`.
    #[cfg(unix)]
    fn fake_proton(dir: &Path) -> PathBuf {
        let script = dir.join("proton");
        let body = r#"#!/bin/sh
echo "$@" >> "$WINEPREFIX/calls.txt"
[ "$1" = run ] && shift
case "$1" in
  wineboot)
    mkdir -p "$WINEPREFIX/pfx/drive_c/users/steamuser" "$WINEPREFIX/pfx/dosdevices"
    : > "$WINEPREFIX/pfx/system.reg"; : > "$WINEPREFIX/pfx/user.reg" ;;
  *.exe)
    mkdir -p "$WINEPREFIX/pfx/drive_c/Program Files (x86)/Battle.net"
    : > "$WINEPREFIX/pfx/drive_c/Program Files (x86)/Battle.net/Battle.net.exe" ;;
esac
"#;
        std::fs::write(&script, body).unwrap();
        heronite_platform::fs::make_executable(&script).unwrap();
        script
    }

    fn set_default_wine(paths: &DataPaths, bin: &Path) {
        let wine = json!({"bin": bin, "name": "Fake Proton", "type": "proton"});
        game_config::set_global_setting(paths, HOME, "wineVersion", wine).unwrap();
    }

    /// The client's settings file with only Proton's own launcher asked for
    /// (a file made by a setting change would also name the shared prefix).
    #[cfg(unix)]
    fn run_proton_directly(paths: &DataPaths, client: Client) {
        let app = client.config_app();
        std::fs::create_dir_all(paths.games_config_dir()).unwrap();
        std::fs::write(paths.game_config(app), json!({ app: {"disableUMU": true}, "version": "v0", "explicit": true }).to_string())
            .unwrap();
    }

    #[test]
    fn a_new_prefix_goes_in_the_default_prefix_folder() {
        let (_dir, paths) = paths();
        assert_eq!(default_prefix(&paths, Client::Ea, HOME), PathBuf::from("/home/tester/Games/Heronite/Prefixes/EA app"));
        game_config::set_global_setting(&paths, HOME, "defaultWinePrefixDir", json!("~/Wine")).unwrap();
        assert_eq!(default_prefix(&paths, Client::BattleNet, HOME), PathBuf::from("/home/tester/Wine/Battle.net"));
        game_config::set_global_setting(&paths, HOME, "defaultWinePrefixDir", json!("/data/prefixes")).unwrap();
        assert_eq!(default_prefix(&paths, Client::Rockstar, HOME), PathBuf::from("/data/prefixes/Rockstar Games Launcher"));
    }

    #[cfg(unix)]
    #[test]
    fn the_default_wine_must_be_there_to_count() {
        let (dir, paths) = paths();
        assert!(!has_wine(&paths, HOME));
        set_default_wine(&paths, &dir.path().join("missing/proton"));
        assert!(!has_wine(&paths, HOME));
        set_default_wine(&paths, &fake_proton(dir.path()));
        assert!(has_wine(&paths, HOME));
    }

    #[test]
    fn adopting_points_the_client_at_a_prefix_that_has_it() {
        let (dir, paths) = paths();
        let prefix = dir.path().join("lutris/battlenet");
        assert!(adopt(&paths, Client::BattleNet, HOME, &prefix).unwrap_err().to_string().contains("Battle.net is not installed in"));
        assert_eq!(prefix_of(&paths, Client::BattleNet, HOME), None);

        let exe = prefix.join("drive_c/Program Files (x86)/Battle.net/Battle.net.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "MZ").unwrap();
        set_default_wine(&paths, &dir.path().join("proton"));
        adopt(&paths, Client::BattleNet, HOME, &prefix).unwrap();
        assert_eq!(prefix_of(&paths, Client::BattleNet, HOME), Some(prefix.clone()));
        let own = game_config::game_overrides(&paths, Client::BattleNet.config_app());
        assert_eq!(own["wineVersion"]["name"], "Fake Proton");

        // The Wine the client has chosen stays.
        game_config::set_game_setting(&paths, Client::BattleNet.config_app(), HOME, "wineVersion", json!({"name": "Mine"})).unwrap();
        adopt(&paths, Client::BattleNet, HOME, &prefix).unwrap();
        assert_eq!(game_config::game_overrides(&paths, Client::BattleNet.config_app())["wineVersion"]["name"], "Mine");
    }

    #[test]
    fn forgetting_removes_the_settings_and_leaves_the_prefix() {
        let (dir, paths) = paths();
        forget(&paths, Client::Ea).unwrap();
        let prefix = dir.path().join("ea");
        std::fs::create_dir_all(&prefix).unwrap();
        game_config::set_game_setting(&paths, Client::Ea.config_app(), HOME, "winePrefix", json!(prefix)).unwrap();
        assert!(paths.game_config(Client::Ea.config_app()).is_file());
        forget(&paths, Client::Ea).unwrap();
        assert!(!paths.game_config(Client::Ea.config_app()).exists());
        assert_eq!(prefix_of(&paths, Client::Ea, HOME), None);
        assert!(prefix.is_dir());
    }

    #[test]
    fn the_settings_that_help_a_client_are_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("drive_c/users/steamuser");
        let other = dir.path().join("drive_c/users/me");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(dir.path().join("drive_c/users/Public")).unwrap();
        let config = user.join("AppData/Roaming/Battle.net/Battle.net.config");
        std::fs::create_dir_all(other.join("AppData/Roaming/Battle.net")).unwrap();
        std::fs::write(other.join("AppData/Roaming/Battle.net/Battle.net.config"), "mine").unwrap();

        write_client_settings(Client::BattleNet, dir.path());
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(written["Client"]["Streaming"]["StreamingEnabled"], "false");
        assert_eq!(written["Client"]["GameLaunchWindowBehavior"], "2");
        assert_eq!(std::fs::read_to_string(other.join("AppData/Roaming/Battle.net/Battle.net.config")).unwrap(), "mine");
        assert!(!dir.path().join("drive_c/users/Public/AppData").exists());

        write_client_settings(Client::Ubisoft, dir.path());
        let yaml = std::fs::read_to_string(user.join("AppData/Local/Ubisoft Game Launcher/settings.yaml")).unwrap();
        assert!(yaml.starts_with("overlay:\n  enabled: false\n") && yaml.ends_with("closebehavior: CloseBehavior_Close\n"));
        write_client_settings(Client::Ea, dir.path());
        assert!(!user.join("AppData/Local/Electronic Arts").exists());
    }

    #[test]
    fn an_installer_is_told_by_its_programs() {
        let procs = FakeProcs::new();
        procs.start(10, "wine", None);
        assert!(!installer_running(&procs, "/p", true));
        procs.set_prefix_programs(&[(10, "agent.exe")]);
        assert!(!installer_running(&procs, "/p", false));
        // Until the client is there, whatever runs is still the setup.
        assert!(installer_running(&procs, "/p", true));
        procs.start(11, "wine", None);
        procs.set_prefix_programs(&[(10, "agent.exe"), (11, "msiexec.exe")]);
        assert!(installer_running(&procs, "/p", false));
    }

    #[test]
    fn the_installers_get_their_silent_arguments() {
        assert_eq!(installer_args(Client::Ea), ["/quiet", "EAX_LAUNCH_CLIENT=0", "IGNORE_INSTALLED=1"]);
        assert_eq!(installer_args(Client::Ubisoft), ["/S"]);
        assert_eq!(installer_args(Client::Rockstar), ["/s", "/f"]);
        assert_eq!(installer_args(Client::BattleNet), ["--lang=enUS", r"--installpath=C:\Program Files (x86)\Battle.net"]);
    }

    #[test]
    fn without_a_wine_nothing_starts() {
        let (_dir, paths) = paths();
        let err = install(&paths, Client::BattleNet, HOME, &Cancel::new(), &mut |_| panic!("no step")).unwrap_err();
        assert_eq!(err.to_string(), "No Wine or Proton is installed. Get one in the Wine Manager.");
        assert_eq!(prefix_of(&paths, Client::BattleNet, HOME), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_whole_setup_with_a_fake_proton() {
        let (dir, paths) = paths();
        let home = dir.path().to_string_lossy().into_owned();
        set_default_wine(&paths, &fake_proton(dir.path()));
        game_config::set_global_setting(&paths, &home, "defaultWinePrefixDir", json!(dir.path().join("prefixes"))).unwrap();
        run_proton_directly(&paths, Client::BattleNet);
        // The installer is already downloaded.
        let installer = paths.tools_dir().join("redist/clients/Battle.net-Setup.exe");
        std::fs::create_dir_all(installer.parent().unwrap()).unwrap();
        std::fs::write(&installer, "MZ").unwrap();

        let mut steps = Vec::new();
        install(&paths, Client::BattleNet, &home, &Cancel::new(), &mut |s| steps.push(s)).unwrap();
        assert_eq!(steps, [Step::CreatingPrefix, Step::Installing]);

        let prefix = dir.path().join("prefixes/Battle.net");
        assert_eq!(prefix_of(&paths, Client::BattleNet, &home), Some(prefix.clone()));
        assert!(find_exe(Client::BattleNet, &prefix).is_some());
        assert!(!prefix.join(CREATING_MARKER).exists());
        assert!(prefix.join("pfx/drive_c/users/steamuser/AppData/Roaming/Battle.net/Battle.net.config").is_file());
        let calls = std::fs::read_to_string(prefix.join("calls.txt")).unwrap();
        let calls: Vec<&str> = calls.lines().collect();
        assert_eq!(calls[0], "run wineboot --init");
        assert_eq!(calls[1], format!("run {} --lang=enUS --installpath=C:\\Program Files (x86)\\Battle.net", installer.display()));
        let log = std::fs::read_to_string(dir.path().join("logs/clients/battlenet-setup.log")).unwrap();
        assert!(log.contains("wineboot --init") && log.contains("Battle.net-Setup.exe"), "{log}");

        // Again: the prefix and the client are there, the installer runs once more.
        let mut again = Vec::new();
        install(&paths, Client::BattleNet, &home, &Cancel::new(), &mut |s| again.push(s)).unwrap();
        assert_eq!(again, [Step::Installing]);
    }

    // The fake Proton is a shell script.
    #[cfg(unix)]
    #[test]
    fn a_setup_that_leaves_no_client_says_where_to_look() {
        let (dir, paths) = paths();
        let home = dir.path().to_string_lossy().into_owned();
        let script = dir.path().join("proton");
        std::fs::write(&script, "#!/bin/sh\nmkdir -p \"$WINEPREFIX/pfx/drive_c\" \"$WINEPREFIX/pfx/dosdevices\"\n: > \"$WINEPREFIX/pfx/system.reg\"\n: > \"$WINEPREFIX/pfx/user.reg\"\n").unwrap();
        heronite_platform::fs::make_executable(&script).unwrap();
        set_default_wine(&paths, &script);
        game_config::set_global_setting(&paths, &home, "defaultWinePrefixDir", json!(dir.path().join("prefixes"))).unwrap();
        run_proton_directly(&paths, Client::Rockstar);
        let installer = paths.tools_dir().join("redist/clients/Rockstar-Games-Launcher.exe");
        std::fs::create_dir_all(installer.parent().unwrap()).unwrap();
        std::fs::write(&installer, "MZ").unwrap();

        let err = install(&paths, Client::Rockstar, &home, &Cancel::new(), &mut |_| {}).unwrap_err();
        assert!(err.to_string().starts_with("Rockstar Games Launcher was not installed. The installer's log is in "), "{err}");
        assert!(err.to_string().ends_with("rockstar-setup.log"), "{err}");

        let cancel = Cancel::new();
        cancel.request();
        let err = install(&paths, Client::Rockstar, &home, &cancel, &mut |_| panic!("cancelled before any step")).unwrap_err();
        assert_eq!(err.to_string(), "Cancelled");
    }
}
