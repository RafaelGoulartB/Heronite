//! Fake `EpicGamesLauncher.exe` wrapper for Epic games that verify ownership
//! through their parent process (Rockstar Games Launcher titles such as GTA V,
//! GTA V Enhanced and Red Dead Redemption 2, Returnal, etc.), ported from
//! Heroic's `USE_FAKE_EPIC_EXE` handling in `storeManagers/legendary/games.ts`.
//!
//! When `USE_FAKE_EPIC_EXE` is enabled (through `known_fixes.json`, the game's
//! environment options or the inherited environment) and `LEGENDARY_WRAPPER_EXE`
//! is not already set:
//! * Heronite downloads the pinned `EpicGamesLauncher.exe` from
//!   `BananaWorks07/heroic-epic-integration` into `<data>/tools/epic/`;
//! * under Wine/Proton, copies it to `C:\windows\command\EpicGamesLauncher.exe`
//!   inside the prefix and sets `LEGENDARY_WRAPPER_EXE` to that Windows path;
//! * on Windows, sets `LEGENDARY_WRAPPER_EXE` directly to the cached file.
//!
//! The wrapper also clears `SteamAppId` before spawning the game, preventing
//! Rockstar Games Launcher from entering Steam mode when umu sets `SteamAppId`
//! from `GAMEID`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use heronite_data::DataPaths;
use heronite_i18n::tr;

use crate::env::Env;
use crate::legendary::known_fixes_env;
use crate::legendary_install::sha256_of;
use crate::net::{Cancel, Net, CANCELLED};
use crate::plan::{PlanContext, PrepSpec};
use crate::prepare::{Notice, Prep};
use crate::probe::Probe;
use crate::settings::GameSettings;

/// Environment switch set by Heroic's `known_fixes.json` or the user.
pub const ENV_KEY: &str = "USE_FAKE_EPIC_EXE";
/// Variable `legendary` reads to insert a wrapper executable before the game.
pub const WRAPPER_ENV_KEY: &str = "LEGENDARY_WRAPPER_EXE";
/// Executable name expected by third-party launchers as parent process.
pub const EXE_NAME: &str = "EpicGamesLauncher.exe";
/// Windows path used inside Wine/Proton prefixes (matches Heroic).
pub const PREFIX_WIN_PATH: &str = r"C:\windows\command\EpicGamesLauncher.exe";

const REPO: &str = "BananaWorks07/heroic-epic-integration";
/// The release Heroic pins (`RELEASE_TAGS['epic-integration']`).
pub const VERSION: &str = "v0.4";
/// SHA-256 of `EpicGamesLauncher.exe` in [`VERSION`]: a download that does not
/// match is never kept or copied into a prefix.
pub const SHA256: &str = "1feb21e21e19cbb34e881791c8f2557769f59231e46850915d49a6d0c1ce4583";
/// Global timeout for downloading the ~150 KB wrapper so a stalled connection
/// cannot hang an optional pre-launch step indefinitely.
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(30);

/// Result of preparing `EpicGamesLauncher.exe` before a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeEpicOutcome {
    /// Pass this value as `LEGENDARY_WRAPPER_EXE`.
    Use(String),
    /// Not requested, or setup failed with a warning (launch without wrapper).
    Skip,
    /// The user stopped the launch while downloading.
    Cancelled,
}

pub fn download_url() -> String {
    format!("https://github.com/{REPO}/releases/download/{VERSION}/{EXE_NAME}")
}

/// `<data>/tools/epic/EpicGamesLauncher.exe`.
pub fn cached_exe(paths: &DataPaths) -> PathBuf {
    paths.tools_dir().join("epic").join(EXE_NAME)
}

/// Whether a `USE_FAKE_EPIC_EXE` value turns the wrapper on: anything other
/// than absent, empty, `"0"` or `"false"` (case-insensitive), matching Heroic.
pub fn is_enabled(value: Option<&str>) -> bool {
    match value.map(str::trim) {
        None | Some("") | Some("0") => false,
        Some(v) if v.eq_ignore_ascii_case("false") => false,
        Some(_) => true,
    }
}

/// Whether this Epic launch should run under the fake `EpicGamesLauncher.exe`.
/// User game settings override `known_fixes.json`, which overrides the
/// inherited environment. Skipped for third-party store games (EA / Ubisoft)
/// and when `LEGENDARY_WRAPPER_EXE` is already configured.
pub(crate) fn is_requested(
    ctx: &PlanContext,
    probe: &dyn Probe,
    app_name: &str,
    settings: &GameSettings,
    env: &Env,
    store_managed: bool,
) -> bool {
    if store_managed {
        return false;
    }
    // Plan/user env shadows an inherited host value (so setting "" clears it).
    let effective_wrapper = env.get(WRAPPER_ENV_KEY).cloned().or_else(|| probe.env_var(WRAPPER_ENV_KEY));
    if effective_wrapper.is_some_and(|v| !v.trim().is_empty()) {
        return false;
    }
    if let Some((_, v)) = settings.env_options.iter().rev().find(|(k, _)| k == ENV_KEY) {
        return is_enabled(Some(v));
    }
    let fixes = known_fixes_env(ctx, "epic", app_name);
    if let Some(v) = fixes.get(ENV_KEY) {
        return is_enabled(Some(v));
    }
    let inherited = probe.env_var(ENV_KEY);
    is_enabled(inherited.as_deref())
}

/// Ensures the pinned `EpicGamesLauncher.exe` is in `<data>/tools/epic/`.
/// Blocking, network.
pub fn ensure(paths: &DataPaths, net: &dyn Net, cancel: Cancel) -> Result<PathBuf, String> {
    ensure_pinned(paths, net, cancel, SHA256, || {})
}

fn ensure_pinned(paths: &DataPaths, net: &dyn Net, cancel: Cancel, pin: &str, on_download: impl FnOnce()) -> Result<PathBuf, String> {
    static BUSY: Mutex<()> = Mutex::new(());
    let _guard = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let target = cached_exe(paths);
    if fs::symlink_metadata(&target).is_ok_and(|m| m.is_file()) && sha256_of(&target).is_ok_and(|sum| sum == pin) {
        return Ok(target);
    }
    if !net.is_online() {
        if target.exists() {
            return Err(format!(
                "{EXE_NAME} in Heronite's data folder is not the release Heronite expects, and there is no internet connection to download it again"
            ));
        }
        return Err(format!("{EXE_NAME} is not downloaded and there is no internet connection"));
    }
    if cancel() {
        return Err(CANCELLED.into());
    }
    on_download();
    let dir = target.parent().ok_or_else(|| format!("{EXE_NAME} has no folder"))?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let part = dir.join(format!("{EXE_NAME}.part"));
    let _ = fs::remove_file(&part);
    let fail = |msg: String| {
        let _ = fs::remove_file(&part);
        msg
    };
    net.download_within(&download_url(), &part, cancel, DOWNLOAD_LIMIT)
        .map_err(|e| fail(if e == CANCELLED { e } else { format!("downloading {EXE_NAME} failed: {e}") }))?;
    let sum = sha256_of(&part).map_err(|e| fail(format!("reading the downloaded {EXE_NAME}: {e}")))?;
    if sum != pin {
        return Err(fail(format!("the downloaded {EXE_NAME} is not the expected file (SHA-256 {sum})")));
    }
    heronite_platform::fs::rename_replacing(&part, &target).map_err(|e| fail(format!("installing {EXE_NAME}: {e}")))?;
    log::info!("{EXE_NAME} {VERSION} downloaded to {}", target.display());
    Ok(target)
}

/// Checks that `path` is a real directory (not a symlink) whose canonical path
/// stays inside `prefix_canon`: sandboxed programs can write to the prefix,
/// while Heronite copies files into it from the host.
fn check_dir_inside(path: &Path, prefix_canon: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("checking {}: {e}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!("refusing to follow non-directory or link in prefix: {}", path.display()));
    }
    let canon = fs::canonicalize(path).map_err(|e| format!("resolving {}: {e}", path.display()))?;
    if !canon.starts_with(prefix_canon) {
        return Err(format!("prefix path {} escapes {}", path.display(), prefix_canon.display()));
    }
    Ok(canon)
}

/// The `drive_c` directory of the prefix (`pfx/drive_c` first for Proton,
/// `drive_c` first for plain Wine) and the canonical prefix root, rejecting
/// symlinked `pfx` or `drive_c` directories.
fn prefix_drive_c(settings: &GameSettings) -> Result<(PathBuf, PathBuf), String> {
    let prefix = Path::new(settings.wine_prefix.trim());
    if prefix.as_os_str().is_empty() {
        return Err("Wine prefix is empty".into());
    }
    let prefix_canon = fs::canonicalize(prefix).map_err(|e| format!("Wine prefix {}: {e}", prefix.display()))?;
    let candidates: [(Option<&str>, PathBuf); 2] = if settings.wine_version.kind == "proton" {
        [(Some("pfx"), prefix.join("pfx")), (None, prefix.to_path_buf())]
    } else {
        [(None, prefix.to_path_buf()), (Some("pfx"), prefix.join("pfx"))]
    };
    let mut rejected: Option<String> = None;
    for (sub, root) in candidates {
        if let Some(sub_name) = sub {
            let sub_path = prefix.join(sub_name);
            if fs::symlink_metadata(&sub_path).is_err() {
                continue;
            }
            if let Err(e) = check_dir_inside(&sub_path, &prefix_canon) {
                rejected = Some(e);
                continue;
            }
        }
        let drive_c = root.join("drive_c");
        if fs::symlink_metadata(&drive_c).is_err() {
            continue;
        }
        match check_dir_inside(&drive_c, &prefix_canon) {
            Ok(_) => return Ok((drive_c, prefix_canon)),
            Err(e) => rejected = Some(e),
        }
    }
    Err(rejected.unwrap_or_else(|| format!("Wine prefix has no drive_c: {}", settings.wine_prefix)))
}

/// Finds an existing child directory matching `name` case-insensitively (for
/// case-sensitive Linux filesystems) or creates `parent/name`, rejecting
/// symlinks or paths that escape `prefix_canon`.
fn open_or_create_subdir(parent: &Path, name: &str, prefix_canon: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(parent).map_err(|e| format!("reading {}: {e}", parent.display()))?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            let path = entry.path();
            check_dir_inside(&path, prefix_canon)?;
            return Ok(path);
        }
    }
    let target = parent.join(name);
    if let Err(e) = fs::create_dir(&target) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(format!("creating {}: {e}", target.display()));
        }
    }
    check_dir_inside(&target, prefix_canon)?;
    Ok(target)
}

/// Copies `exe` into `<prefix>/.../drive_c/windows/command/EpicGamesLauncher.exe`
/// atomically, refusing symlinked ancestor directories and replacing any
/// symlink or special file at the destination without opening it.
fn install_in_prefix(exe: &Path, settings: &GameSettings, pin: &str) -> Result<PathBuf, String> {
    let (drive_c, prefix_canon) = prefix_drive_c(settings)?;
    let windows = open_or_create_subdir(&drive_c, "windows", &prefix_canon)?;
    let command = open_or_create_subdir(&windows, "command", &prefix_canon)?;
    let dest = command.join(EXE_NAME);
    if let Ok(meta) = fs::symlink_metadata(&dest) {
        if meta.is_dir() {
            return Err(format!("{} is a directory", dest.display()));
        }
        // Only hash a regular file: avoids hanging on FIFOs/devices or
        // accepting a symlink pointing outside the prefix.
        if meta.is_file() && sha256_of(&dest).is_ok_and(|sum| sum == pin) {
            return Ok(dest);
        }
    }
    let bytes = fs::read(exe).map_err(|e| format!("reading {}: {e}", exe.display()))?;
    heronite_data::temps::write_atomic(&dest, &bytes, None).map_err(|e| format!("writing {}: {e}", dest.display()))?;
    Ok(dest)
}

#[cfg(test)]
static TEST_PIN: Mutex<Option<String>> = Mutex::new(None);
#[cfg(test)]
static TEST_PIN_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) struct TestPinGuard {
    prev: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for TestPinGuard {
    fn drop(&mut self) {
        *TEST_PIN.lock().unwrap_or_else(|e| e.into_inner()) = self.prev.take();
    }
}

#[cfg(test)]
pub(crate) fn override_pin_for_test(pin: impl Into<String>) -> TestPinGuard {
    let lock = TEST_PIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = TEST_PIN.lock().unwrap_or_else(|e| e.into_inner()).replace(pin.into());
    TestPinGuard { prev, _lock: lock }
}

impl Prep<'_> {
    /// Prepares `EpicGamesLauncher.exe` when `spec.fake_epic_exe` is set and
    /// returns the `LEGENDARY_WRAPPER_EXE` value to pass to `legendary`.
    /// Failures warn and return [`FakeEpicOutcome::Skip`] so the launch
    /// carries on without pointing legendary at a missing file.
    pub fn fake_epic_exe(&self, spec: &PrepSpec) -> FakeEpicOutcome {
        #[cfg(test)]
        let pin = TEST_PIN.lock().unwrap_or_else(|e| e.into_inner()).clone();
        #[cfg(test)]
        let pin = pin.as_deref().unwrap_or(SHA256);
        #[cfg(not(test))]
        let pin = SHA256;
        self.fake_epic_exe_pinned(spec, pin)
    }

    pub(crate) fn fake_epic_exe_pinned(&self, spec: &PrepSpec, pin: &str) -> FakeEpicOutcome {
        if !spec.fake_epic_exe {
            return FakeEpicOutcome::Skip;
        }
        if (self.cancel)() {
            return FakeEpicOutcome::Cancelled;
        }
        let exe = match ensure_pinned(self.paths, self.net, self.cancel, pin, || {
            self.notice(Notice::info("Downloading the Epic launcher wrapper...", tr!("Downloading the Epic launcher wrapper...")));
        }) {
            Ok(p) => p,
            Err(e) if e == CANCELLED => return FakeEpicOutcome::Cancelled,
            Err(e) => {
                self.notice(Notice::warn(
                    format!("Could not set up the Epic launcher wrapper: {e}"),
                    tr!("Could not set up the Epic launcher wrapper: {error}", error = e),
                ));
                return FakeEpicOutcome::Skip;
            }
        };
        if let Some(settings) = &spec.wine {
            if let Err(e) = install_in_prefix(&exe, settings, pin) {
                self.notice(Notice::warn(
                    format!("Could not set up the Epic launcher wrapper: {e}"),
                    tr!("Could not set up the Epic launcher wrapper: {error}", error = e),
                ));
                return FakeEpicOutcome::Skip;
            }
            return FakeEpicOutcome::Use(PREFIX_WIN_PATH.to_string());
        }
        FakeEpicOutcome::Use(exe.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::prepare::tests::InstantSpawner;
    use crate::probe::FakeProbe;
    use crate::settings::WineVersion;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex as StdMutex;

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn wine_settings(prefix: &Path, kind: &str) -> GameSettings {
        GameSettings {
            wine_prefix: prefix.to_string_lossy().into_owned(),
            wine_version: WineVersion { kind: kind.into(), ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn enabled_values_match_heroics_rule() {
        for off in [None, Some(""), Some("  "), Some("0"), Some("false"), Some("FALSE"), Some(" False ")] {
            assert!(!is_enabled(off), "expected off for {off:?}");
        }
        for on in [Some("true"), Some("1"), Some("TRUE"), Some("yes")] {
            assert!(is_enabled(on), "expected on for {on:?}");
        }
    }

    #[test]
    fn request_precedence_respects_user_override_and_explicit_wrapper() {
        let mut ctx = PlanContext::load(&DataPaths::at("/hr"));
        ctx.known_fixes = Some(json!({
            "epic": {
                "8769e24080ea413b8ebca3f1b8c50951": {"envVariables": {"USE_FAKE_EPIC_EXE": true}},
                "Fortnite": {"envVariables": {"USE_FAKE_EPIC_EXE": 0}},
            }
        }));
        let probe = FakeProbe::new("/h");
        let empty = GameSettings::default();
        let env = Env::new();

        // Enabled via known_fixes.json.
        assert!(is_requested(&ctx, &probe, "8769e24080ea413b8ebca3f1b8c50951", &empty, &env, false));
        // Disabled via known_fixes.json (0).
        assert!(!is_requested(&ctx, &probe, "Fortnite", &empty, &env, false));
        // Store-managed (EA / Ubisoft) never uses the wrapper.
        assert!(!is_requested(&ctx, &probe, "8769e24080ea413b8ebca3f1b8c50951", &empty, &env, true));

        // User can turn off a known fix.
        let mut user_off = GameSettings::default();
        user_off.env_options.push((ENV_KEY.into(), "0".into()));
        assert!(!is_requested(&ctx, &probe, "8769e24080ea413b8ebca3f1b8c50951", &user_off, &env, false));

        // User can turn it on for a game not in known_fixes.
        let mut user_on = GameSettings::default();
        user_on.env_options.push((ENV_KEY.into(), "true".into()));
        assert!(is_requested(&ctx, &probe, "OtherGame", &user_on, &env, false));

        // Inherited host env enables it when neither user nor known_fixes sets it.
        let host_on = FakeProbe::new("/h").with_env(ENV_KEY, "1");
        assert!(is_requested(&ctx, &host_on, "OtherGame", &empty, &env, false));
        // Known fix `0` overrides inherited env.
        assert!(!is_requested(&ctx, &host_on, "Fortnite", &empty, &env, false));

        // Explicit LEGENDARY_WRAPPER_EXE in env or host is preserved.
        let mut with_wrapper = Env::new();
        with_wrapper.insert(WRAPPER_ENV_KEY.into(), r"C:\custom\wrap.exe".into());
        assert!(!is_requested(&ctx, &probe, "8769e24080ea413b8ebca3f1b8c50951", &empty, &with_wrapper, false));
        let host_wrapper = FakeProbe::new("/h").with_env(WRAPPER_ENV_KEY, r"C:\custom\wrap.exe");
        assert!(!is_requested(&ctx, &host_wrapper, "8769e24080ea413b8ebca3f1b8c50951", &empty, &env, false));

        // Explicitly clearing LEGENDARY_WRAPPER_EXE in game env shadows host env.
        let mut cleared_wrapper = Env::new();
        cleared_wrapper.insert(WRAPPER_ENV_KEY.into(), String::new());
        assert!(is_requested(&ctx, &host_wrapper, "8769e24080ea413b8ebca3f1b8c50951", &empty, &cleared_wrapper, false));
    }

    #[test]
    fn ensure_downloads_verifies_and_caches() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let body = b"MZ-fake-epic-v0.4";
        let pin = sha(body);
        let net = FakeNet::online().with_file(&download_url(), body.to_vec());

        let notified = AtomicBool::new(false);
        let got = ensure_pinned(&paths, &net, &|| false, &pin, || notified.store(true, Ordering::SeqCst)).unwrap();
        assert!(notified.load(Ordering::SeqCst));
        assert_eq!(got, cached_exe(&paths));
        assert_eq!(fs::read(&got).unwrap(), body);
        assert!(!got.with_file_name(format!("{EXE_NAME}.part")).exists());

        // Cached and matching: no second request, no notice, even offline.
        let offline = FakeNet::offline();
        let notified_again = AtomicBool::new(false);
        assert_eq!(ensure_pinned(&paths, &offline, &|| false, &pin, || notified_again.store(true, Ordering::SeqCst)).unwrap(), got);
        assert!(!notified_again.load(Ordering::SeqCst));
        assert_eq!(net.requests().len(), 1);
    }

    #[test]
    fn ensure_rejects_mismatched_hash_offline_and_mid_download_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let net = FakeNet::online().with_file(&download_url(), b"tampered".to_vec());
        let err = ensure(&paths, &net, &|| false).unwrap_err();
        assert!(err.contains("not the expected file"), "{err}");
        assert!(!cached_exe(&paths).exists());
        assert!(!cached_exe(&paths).with_file_name(format!("{EXE_NAME}.part")).exists());

        // Cancel firing during download cleans up .part and returns CANCELLED.
        struct CancelDuringDownload;
        impl Net for CancelDuringDownload {
            fn is_online(&self) -> bool {
                true
            }
            fn get_json(&self, _: &str) -> Result<serde_json::Value, String> {
                unreachable!()
            }
            fn download(&self, _: &str, dest: &Path, cancel: Cancel) -> Result<(), String> {
                fs::write(dest, b"partial").unwrap();
                assert!(cancel());
                Err(CANCELLED.into())
            }
        }
        let cancelled = AtomicBool::new(false);
        let err = ensure_pinned(&paths, &CancelDuringDownload, &|| cancelled.load(Ordering::SeqCst), SHA256, || {
            cancelled.store(true, Ordering::SeqCst)
        })
        .unwrap_err();
        assert_eq!(err, CANCELLED);
        assert!(!cached_exe(&paths).exists());
        assert!(!cached_exe(&paths).with_file_name(format!("{EXE_NAME}.part")).exists());

        let offline = FakeNet::offline();
        let err = ensure(&paths, &offline, &|| false).unwrap_err();
        assert!(err.contains("no internet"), "{err}");

        // Stale file offline reports mismatch rather than using it.
        fs::create_dir_all(cached_exe(&paths).parent().unwrap()).unwrap();
        fs::write(cached_exe(&paths), b"stale").unwrap();
        let err = ensure(&paths, &offline, &|| false).unwrap_err();
        assert!(err.contains("not the release Heronite expects"), "{err}");
    }

    #[test]
    fn install_in_prefix_handles_proton_wine_casing_and_updates() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("EpicGamesLauncher.exe");
        let body = b"MZ-wrapper-bytes";
        let pin = sha(body);
        fs::write(&src, body).unwrap();

        // Proton prefix (`pfx/drive_c/windows/command`).
        let proton = dir.path().join("proton-pfx");
        fs::create_dir_all(proton.join("pfx/drive_c/windows")).unwrap();
        let dest = install_in_prefix(&src, &wine_settings(&proton, "proton"), &pin).unwrap();
        assert_eq!(dest, proton.join("pfx/drive_c/windows/command/EpicGamesLauncher.exe"));
        assert_eq!(fs::read(&dest).unwrap(), body);

        // Plain Wine prefix with arbitrary uppercase `WINDOWS/COMMAND`.
        let wine = dir.path().join("wine-pfx");
        fs::create_dir_all(wine.join("drive_c/WINDOWS/COMMAND")).unwrap();
        fs::write(wine.join("drive_c/WINDOWS/COMMAND/EpicGamesLauncher.exe"), b"old").unwrap();
        let dest = install_in_prefix(&src, &wine_settings(&wine, "wine"), &pin).unwrap();
        assert_eq!(dest, wine.join("drive_c/WINDOWS/COMMAND/EpicGamesLauncher.exe"));
        assert_eq!(fs::read(&dest).unwrap(), body, "stale copy in prefix is replaced");

        // Missing prefix reports an error.
        let missing = dir.path().join("missing-pfx");
        assert!(install_in_prefix(&src, &wine_settings(&missing, "proton"), &pin).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn install_in_prefix_blocks_symlink_escapes_and_replaces_linked_dest() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("EpicGamesLauncher.exe");
        let body = b"MZ-wrapper-bytes";
        let pin = sha(body);
        fs::write(&src, body).unwrap();

        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();

        // Symlinked `command` directory pointing outside the prefix is refused.
        let pfx1 = dir.path().join("pfx-linked-command");
        fs::create_dir_all(pfx1.join("pfx/drive_c/windows")).unwrap();
        symlink(&outside, pfx1.join("pfx/drive_c/windows/command")).unwrap();
        assert!(install_in_prefix(&src, &wine_settings(&pfx1, "proton"), &pin).is_err());
        assert!(!outside.join(EXE_NAME).exists());

        // Symlinked `windows` directory pointing outside the prefix is refused.
        let pfx2 = dir.path().join("pfx-linked-windows");
        fs::create_dir_all(pfx2.join("pfx/drive_c")).unwrap();
        symlink(&outside, pfx2.join("pfx/drive_c/windows")).unwrap();
        assert!(install_in_prefix(&src, &wine_settings(&pfx2, "proton"), &pin).is_err());
        assert!(!outside.join("command").exists());

        // Symlinked `pfx` directory pointing outside the prefix is refused.
        let pfx3 = dir.path().join("pfx-linked-root");
        fs::create_dir_all(&pfx3).unwrap();
        fs::create_dir_all(outside.join("drive_c")).unwrap();
        symlink(&outside, pfx3.join("pfx")).unwrap();
        assert!(install_in_prefix(&src, &wine_settings(&pfx3, "proton"), &pin).is_err());

        // Symlink at the destination file (even pointing at matching bytes) is
        // replaced atomically with a regular file rather than followed.
        let pfx4 = dir.path().join("pfx-linked-dest");
        fs::create_dir_all(pfx4.join("pfx/drive_c/windows/command")).unwrap();
        let linked_target = outside.join("victim.bin");
        fs::write(&linked_target, body).unwrap();
        let dest_link = pfx4.join("pfx/drive_c/windows/command/EpicGamesLauncher.exe");
        symlink(&linked_target, &dest_link).unwrap();
        let dest = install_in_prefix(&src, &wine_settings(&pfx4, "proton"), &pin).unwrap();
        let meta = fs::symlink_metadata(&dest).unwrap();
        assert!(!meta.file_type().is_symlink() && meta.is_file());
        assert_eq!(fs::read(&dest).unwrap(), body);

        // umu Proton layout: `pfx` is a symlink back to `<prefix>`, so it
        // falls back to `<prefix>/drive_c` and installs cleanly there.
        let pfx5 = dir.path().join("pfx-umu-layout");
        fs::create_dir_all(pfx5.join("drive_c/windows")).unwrap();
        symlink(&pfx5, pfx5.join("pfx")).unwrap();
        let dest = install_in_prefix(&src, &wine_settings(&pfx5, "proton"), &pin).unwrap();
        assert_eq!(dest, pfx5.join("drive_c/windows/command/EpicGamesLauncher.exe"));
        assert_eq!(fs::read(&dest).unwrap(), body);
    }

    #[test]
    fn prep_returns_wrapper_for_wine_and_native_and_warns_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let body = b"MZ-prep-test";
        let pin = sha(body);
        let net = FakeNet::online().with_file(&download_url(), body.to_vec());
        let probe = FakeProbe::new("/h");
        let spawner = InstantSpawner::default();
        let notices = StdMutex::new(Vec::new());
        let sink = |n: Notice| notices.lock().unwrap().push(n);

        let prep = Prep { paths: &paths, probe: &probe, net: &net, spawner: &spawner, cloud: None, cancel: &|| false, notify: &sink };

        // Proton prefix: downloads, copies into prefix, returns Windows path.
        let pfx = dir.path().join("pfx");
        fs::create_dir_all(pfx.join("pfx/drive_c")).unwrap();
        let spec = PrepSpec { fake_epic_exe: true, wine: Some(wine_settings(&pfx, "proton")), ..Default::default() };
        assert_eq!(prep.fake_epic_exe_pinned(&spec, &pin), FakeEpicOutcome::Use(PREFIX_WIN_PATH.to_string()));
        assert_eq!(fs::read(pfx.join("pfx/drive_c/windows/command/EpicGamesLauncher.exe")).unwrap(), body);
        assert_eq!(notices.lock().unwrap().len(), 1);
        assert!(!notices.lock().unwrap()[0].warning);

        // Native Windows (`wine: None`): reuses cached file and returns its path.
        let native_spec = PrepSpec { fake_epic_exe: true, wine: None, ..Default::default() };
        let FakeEpicOutcome::Use(got) = prep.fake_epic_exe_pinned(&native_spec, &pin) else { panic!() };
        assert_eq!(PathBuf::from(got), cached_exe(&paths));
        assert_eq!(notices.lock().unwrap().len(), 1, "no download notice when cached");

        // Offline and uncached: warns and skips so launch continues.
        let paths2 = DataPaths::at(dir.path().join("heronite2"));
        let offline = FakeNet::offline();
        let prep2 = Prep { paths: &paths2, probe: &probe, net: &offline, spawner: &spawner, cloud: None, cancel: &|| false, notify: &sink };
        assert_eq!(prep2.fake_epic_exe_pinned(&native_spec, &pin), FakeEpicOutcome::Skip);
        assert!(notices.lock().unwrap().last().unwrap().warning);

        // Cancelled: returns Cancelled.
        let prep_cancel =
            Prep { paths: &paths2, probe: &probe, net: &net, spawner: &spawner, cloud: None, cancel: &|| true, notify: &sink };
        assert_eq!(prep_cancel.fake_epic_exe_pinned(&native_spec, &pin), FakeEpicOutcome::Cancelled);
    }

    /// The real pinned release, downloaded from GitHub and verified against
    /// [`SHA256`]: `cargo test -p heronite-launch --lib fake_epic::tests::real_release -- --ignored`.
    #[test]
    #[ignore = "downloads ~150 KB from GitHub"]
    fn real_release_downloads_and_installs_into_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let exe = ensure(&paths, &crate::net::SystemNet, &|| false).unwrap();
        assert_eq!(exe, cached_exe(&paths));
        assert_eq!(sha256_of(&exe).unwrap(), SHA256);

        let pfx = dir.path().join("pfx");
        fs::create_dir_all(pfx.join("pfx/drive_c/windows")).unwrap();
        let installed = install_in_prefix(&exe, &wine_settings(&pfx, "proton"), SHA256).unwrap();
        assert_eq!(sha256_of(&installed).unwrap(), SHA256);
    }
}
