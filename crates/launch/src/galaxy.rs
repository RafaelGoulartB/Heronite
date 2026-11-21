//! GOG Galaxy's communication service for GOG games. The GOG Galaxy SDK in a
//! game talks to it on `127.0.0.1:9977` for achievements, stats and
//! leaderboards; without GOG Galaxy, Comet (`imLinguin/comet`, an open-source
//! stand-in, [`crate::store_tools::COMET`]) plays that part while the game
//! runs, signed in with the user's GOG account, as Heroic does (`launch` in
//! its `storeManagers/gog/games.ts`). The runtime starts and stops it.
//!
//! Some SDK versions look for the Windows service before they connect. Wine
//! prefixes get Comet's dummy `GalaxyCommunication` service the first time a
//! GOG game runs in them (Heroic's `prepareWineLaunch`); on Windows,
//! registering it needs the administrator, so Settings asks for it
//! ([`install_windows_service`]).
//!
//! Comet uploads what the game unlocked every few seconds while the game
//! talks to it, and once more when it is stopped with Ctrl+C, so it gets
//! Ctrl+C and a moment after the game ends.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use heronite_data::DataPaths;
use heronite_i18n::tr;

use crate::net::{Cancel, Net};
use crate::plan::{Command, PlanContext};
use crate::prepare::{run_wait, Notice, Prep};
use crate::settings::GameSettings;
use crate::store_tools::COMET;
use crate::wine_tools::wine_command;

/// The port the SDK connects to.
pub const PORT: u16 = 9977;
/// The Windows service the SDK looks for.
pub const SERVICE_NAME: &str = "GalaxyCommunication";
/// Where GOG Galaxy keeps the service, in a Wine prefix (on Windows, under
/// `%ProgramData%`).
pub const SERVICE_EXE: &str = r"C:\ProgramData\GOG.com\Galaxy\redists\GalaxyCommunication.exe";
/// The files taken from Comet's dummy service release asset
/// (`dummy-service.zip`): the service, and the program that lets a user
/// (not only the administrator) start it on Windows.
const DUMMY_FILES: [&str; 2] = ["GalaxyCommunication.exe", "update-permissions.exe"];
/// What registers the service on Windows, run as administrator. Comet's own
/// `install-dummy-service.bat` does the same, but leaves its paths unquoted
/// (a user name with a space breaks it), writes `binpath=` the way only
/// Wine's `sc` takes it, and runs the service as SYSTEM from a folder of
/// `ProgramData` where users may add files (a DLL put there would run as
/// SYSTEM). Here the administrators own the folder (a user who made it
/// before keeps no say), only SYSTEM and the administrators may change it
/// (users read it), and the service runs as LocalService.
const REGISTER_SCRIPT: &str = "@echo off\r
set \"target=%ProgramData%\\GOG.com\\Galaxy\\redists\"\r
if not exist \"%target%\" mkdir \"%target%\" || exit /b 1\r
icacls \"%target%\" /setowner *S-1-5-32-544 /T >nul || exit /b 1\r
icacls \"%target%\" /reset /T >nul\r
icacls \"%target%\" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX >nul || exit /b 1\r
copy /y \"%~dp0GalaxyCommunication.exe\" \"%target%\\GalaxyCommunication.exe\" >nul || exit /b 1\r
sc create GalaxyCommunication binPath= \"\\\"%target%\\GalaxyCommunication.exe\\\"\" obj= \"NT AUTHORITY\\LocalService\" password= \"\" >nul\r
\"%~dp0update-permissions.exe\"\r
";
const REGISTER_SCRIPT_NAME: &str = "register-galaxy-service.bat";
const DUMMY_ASSET: &str = "dummy-service.zip";
/// How long one Wine command of the prefix setup may take (they take a
/// few seconds; a hung Wine is ended, and the setup runs again next time).
const WINE_TIMEOUT: Duration = Duration::from_secs(45);
/// How long a launch waits for [`GalaxyAccount::login`] (refreshing the
/// tokens takes a second or two; a bad network may hang it).
pub const LOGIN_LIMIT: Duration = Duration::from_secs(30);
/// How long a launch waits for Comet's first download (about 15 MB); past
/// it the game starts without Comet, and the download goes on for the next
/// launch.
pub const DOWNLOAD_LIMIT: Duration = Duration::from_secs(60);

/// The GOG account Comet signs in with.
#[derive(Clone, PartialEq, Eq)]
pub struct GalaxyLogin {
    pub access_token: String,
    pub refresh_token: String,
    /// The Galaxy user id (`galaxyUserId`).
    pub user_id: String,
    pub username: String,
}

impl std::fmt::Debug for GalaxyLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GalaxyLogin").field("user_id", &self.user_id).field("username", &self.username).finish_non_exhaustive()
    }
}

/// What Comet needs from the GOG side, implemented by the app because the
/// store crates depend on this one.
pub trait GalaxyAccount: Send + Sync {
    /// The signed-in account with fresh tokens; `Ok(None)` when Comet is
    /// turned off or nobody is signed in to GOG. Blocking: the tokens may
    /// be refreshed, and should be given up on after [`LOGIN_LIMIT`].
    fn login(&self) -> Result<Option<GalaxyLogin>, String>;
}

/// Comet serving `login`'s account until it is stopped. The tokens are in
/// the arguments: log [`log_line`] instead of the command.
pub fn comet_command(bin: &str, login: &GalaxyLogin) -> Command {
    let args = [
        "--access-token",
        &login.access_token,
        "--refresh-token",
        &login.refresh_token,
        "--user-id",
        &login.user_id,
        "--username",
        &login.username,
    ];
    Command { program: bin.to_string(), args: args.map(String::from).to_vec(), ..Default::default() }
}

/// The command of [`comet_command`] for the logs, without the tokens.
pub fn log_line(bin: &str, login: &GalaxyLogin) -> String {
    format!("{bin} --access-token <hidden> --refresh-token <hidden> --user-id {} --username {}", login.user_id, login.username)
}

/// Whether something already listens on [`PORT`] (GOG Galaxy, a Comet of
/// another launcher, or ours still finishing).
pub fn port_taken() -> bool {
    std::net::TcpListener::bind(("127.0.0.1", PORT)).is_err()
}

/// Waits up to `wait` for the port to be free. `false`: still taken.
pub fn wait_for_port(wait: Duration, cancel: Cancel) -> bool {
    let until = Instant::now() + wait;
    loop {
        if !port_taken() {
            return true;
        }
        if Instant::now() >= until || cancel() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// `<data>/tools/comet/dummy-service`: the files of [`DUMMY_FILES`].
pub fn dummy_service_dir(paths: &DataPaths) -> PathBuf {
    paths.tools_dir().join("comet").join("dummy-service")
}

/// The folder with Comet's dummy service, downloaded the first time.
/// Blocking, network.
pub fn ensure_dummy_service(paths: &DataPaths, net: &dyn Net, cancel: Cancel) -> Result<PathBuf, String> {
    static BUSY: Mutex<()> = Mutex::new(());
    let _guard = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = dummy_service_dir(paths);
    if DUMMY_FILES.iter().all(|f| dir.join(f).is_file()) {
        return Ok(dir);
    }
    if !net.is_online() {
        return Err("Comet's dummy service is not downloaded and there is no internet connection".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let zip = dir.join(format!("{DUMMY_ASSET}.part"));
    let result = net.download(&COMET.download_url(DUMMY_ASSET), &zip, cancel).and_then(|()| extract(&zip, &dir));
    let _ = std::fs::remove_file(&zip);
    result.map(|()| dir).map_err(|e| format!("getting Comet's dummy service: {e}"))
}

/// Writes the [`DUMMY_FILES`] of the archive `zip` into `dir`.
fn extract(zip: &Path, dir: &Path) -> Result<(), String> {
    use std::io::Read;
    let file = std::fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", zip.display()))?;
    for name in DUMMY_FILES {
        let mut entry = archive.by_name(name).map_err(|e| format!("{name} in {}: {e}", zip.display()))?;
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| format!("{name}: {e}"))?;
        let dest = dir.join(name);
        heronite_data::temps::write_atomic(&dest, &bytes, None).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    Ok(())
}

/// The folder of the prefix that holds `drive_c` (`pfx` inside a Proton
/// prefix); `None` before the prefix exists.
fn prefix_root(settings: &GameSettings) -> Option<PathBuf> {
    let prefix = Path::new(settings.wine_prefix.trim());
    if prefix.as_os_str().is_empty() {
        return None;
    }
    [prefix.join("pfx"), prefix.to_path_buf()].into_iter().find(|p| p.join("drive_c").is_dir())
}

/// [`SERVICE_EXE`] inside the prefix folder `root`.
fn service_in_prefix(root: &Path) -> PathBuf {
    root.join("drive_c/ProgramData/GOG.com/Galaxy/redists/GalaxyCommunication.exe")
}

impl Prep<'_> {
    /// Puts Comet's dummy service (`dir`, from [`ensure_dummy_service`]) into
    /// the game's prefix and registers it, once per prefix
    /// (`prepareWineLaunch` with `cometSupport`). A failure is only logged:
    /// most games connect without the service.
    pub(crate) fn galaxy_service_in_prefix(&self, settings: &GameSettings, dir: &Path) {
        let Some(root) = prefix_root(settings) else { return };
        let target = service_in_prefix(&root);
        if target.is_file() {
            return;
        }
        self.notice(Notice::info(
            "Setting up GOG Galaxy's service in the prefix...",
            tr!("Setting up GOG Galaxy's service in the prefix..."),
        ));
        let ctx = PlanContext::load(self.paths);
        let binpath = format!("binpath={SERVICE_EXE}");
        let commands: [Vec<&str>; 2] = [
            vec!["sc", "create", SERVICE_NAME, &binpath],
            // Some SDK versions read where GOG Galaxy is installed.
            vec![
                "reg",
                "add",
                r"HKLM\SOFTWARE\WOW6432Node\GOG.com\GalaxyClient\paths",
                "/v",
                "client",
                "/t",
                "REG_SZ",
                "/d",
                r"C:\Program Files\GOG Galaxy",
                "/f",
            ],
        ];
        let mut done = true;
        for (i, parts) in commands.into_iter().enumerate() {
            let parts: Vec<String> = parts.into_iter().map(String::from).collect();
            let ok = match wine_command(&ctx, self.probe, settings, parts.clone()) {
                Ok(cmd) => run_wait(self.spawner, &cmd, WINE_TIMEOUT, self.cancel),
                Err(e) => {
                    log::warn!("{e}");
                    false
                }
            };
            if !ok {
                log::warn!("`{}` in the prefix did not succeed", parts.join(" "));
                // `sc create` also fails for a service the prefix already
                // has: `reg add` (which overwrites) decides.
                done &= i == 0;
            }
        }
        if !done || (self.cancel)() {
            return;
        }
        // The service's file marks the prefix as set up: copied last, so a
        // setup cut short runs again at the next launch.
        let copied = target
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::copy(dir.join(DUMMY_FILES[0]), &target).map(drop));
        if let Err(e) = copied {
            log::warn!("copying GOG Galaxy's dummy service to {}: {e}", target.display());
        }
    }
}

/// Whether the service is registered on this Windows (GOG Galaxy's, or
/// Comet's dummy one). Always `true` elsewhere: nothing to set up.
pub fn windows_service_registered() -> bool {
    #[cfg(windows)]
    {
        use heronite_platform::registry::{values, Hive};
        values(Hive::LocalMachine, &format!(r"SYSTEM\CurrentControlSet\Services\{SERVICE_NAME}")).is_some()
    }
    #[cfg(not(windows))]
    true
}

/// Registers Comet's dummy service on Windows ([`REGISTER_SCRIPT`]), run as
/// administrator (Windows asks the user). Blocking, network.
pub fn install_windows_service(paths: &DataPaths, net: &dyn Net) -> Result<(), String> {
    let dir = ensure_dummy_service(paths, net, &|| false)?;
    let script = dir.join(REGISTER_SCRIPT_NAME);
    heronite_data::temps::write_atomic(&script, REGISTER_SCRIPT.as_bytes(), None).map_err(|e| format!("{}: {e}", script.display()))?;
    match heronite_platform::shell::run_elevated(&script, &[] as &[&str], Some(&dir)) {
        Ok(_) if windows_service_registered() => Ok(()),
        Ok(code) => Err(tr!("The service was not registered (the script ended with {code})", code)),
        Err(heronite_platform::shell::ElevatedError::Declined) => Err(tr!("Windows did not get the administrator's permission").into()),
        Err(heronite_platform::shell::ElevatedError::Failed(e)) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;

    fn login() -> GalaxyLogin {
        GalaxyLogin { access_token: "acc".into(), refresh_token: "ref".into(), user_id: "4242".into(), username: "player".into() }
    }

    #[test]
    fn comet_signs_in_with_the_account_and_the_log_hides_the_tokens() {
        let cmd = comet_command("/hr/tools/bin/comet", &login());
        assert_eq!(cmd.program, "/hr/tools/bin/comet");
        assert_eq!(cmd.args, ["--access-token", "acc", "--refresh-token", "ref", "--user-id", "4242", "--username", "player"]);
        let line = log_line(&cmd.program, &login());
        assert!(!line.contains("acc ") && !line.contains("ref ") && line.contains("player"), "{line}");
        let debug = format!("{:?}", login());
        assert!(!debug.contains("acc") && !debug.contains("ref"), "{debug}");
    }

    fn dummy_zip() -> Vec<u8> {
        use std::io::Write;
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut out);
        for f in DUMMY_FILES {
            w.start_file(f, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(f.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn the_dummy_service_is_downloaded_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let url = COMET.download_url(DUMMY_ASSET);
        assert_eq!(url, "https://github.com/imLinguin/comet/releases/download/v0.4.1/dummy-service.zip");
        let net = FakeNet::online().with_file(&url, dummy_zip());
        let got = ensure_dummy_service(&paths, &net, &|| false).unwrap();
        assert_eq!(got, dummy_service_dir(&paths));
        assert_eq!(std::fs::read(got.join("GalaxyCommunication.exe")).unwrap(), b"GalaxyCommunication.exe");
        assert!(!got.join("dummy-service.zip.part").exists());
        ensure_dummy_service(&paths, &FakeNet::offline(), &|| false).unwrap();
        assert_eq!(net.requests().len(), 1);

        let other = DataPaths::at(dir.path().join("other"));
        assert!(ensure_dummy_service(&other, &FakeNet::offline(), &|| false).unwrap_err().contains("no internet"));
    }

    #[test]
    fn the_windows_script_quotes_its_paths() {
        assert!(REGISTER_SCRIPT.contains(r#"copy /y "%~dp0GalaxyCommunication.exe" "%target%\GalaxyCommunication.exe""#));
        assert!(
            REGISTER_SCRIPT.contains(r#"binPath= "\"%target%\GalaxyCommunication.exe\"""#),
            "sc.exe wants a space after binPath=, and the path quoted inside"
        );
        assert!(REGISTER_SCRIPT.contains(r#"obj= "NT AUTHORITY\LocalService""#), "not SYSTEM");
        assert!(REGISTER_SCRIPT.contains("/inheritance:r"), "users may not add files next to it");
        assert!(REGISTER_SCRIPT.contains("/setowner *S-1-5-32-544"), "nor own a folder they made before");
        assert!(REGISTER_SCRIPT.split_inclusive('\n').all(|l| l.ends_with("\r\n")), "a batch file with CRLF line ends");
    }

    #[test]
    fn the_service_goes_where_the_prefix_keeps_drive_c() {
        let dir = tempfile::tempdir().unwrap();
        let settings = GameSettings { wine_prefix: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        assert_eq!(prefix_root(&settings), None, "not a prefix yet");
        std::fs::create_dir_all(dir.path().join("pfx/drive_c")).unwrap();
        assert_eq!(prefix_root(&settings), Some(dir.path().join("pfx")));
        assert_eq!(
            service_in_prefix(&dir.path().join("pfx")),
            dir.path().join("pfx/drive_c/ProgramData/GOG.com/Galaxy/redists/GalaxyCommunication.exe")
        );
        let wine = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(wine.path().join("drive_c")).unwrap();
        let settings = GameSettings { wine_prefix: wine.path().to_string_lossy().into_owned(), ..Default::default() };
        assert_eq!(prefix_root(&settings), Some(wine.path().to_path_buf()));
        assert_eq!(prefix_root(&GameSettings::default()), None);
    }

    #[test]
    fn a_free_port_is_found_and_a_taken_one_waited_for() {
        // Another test, or a real Comet, may hold the port: only check the
        // answer agrees with a bind made right after.
        let taken = port_taken();
        assert_eq!(wait_for_port(Duration::ZERO, &|| false), !taken);
    }
}
