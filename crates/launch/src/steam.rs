//! Steam helpers: `steam://` URIs, installed-app detection from
//! `libraryfolders.vdf` + `appmanifest_*.acf`, and Steam's log files
//! (port of `steam.ts`).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use heronite_i18n::tr;
use regex::Regex;

use crate::probe::Probe;
use crate::procs::Procs;
use crate::runtime::Spawner;
use crate::settings::GlobalSettings;
use crate::vdf::Vdf;

/// `StateFlags` bit meaning "fully installed".
pub const STEAM_FULLY_INSTALLED: u64 = 4;

/// Proton, runtimes and tools that appear as Steam apps but are not games.
pub const SKIP_STEAM_APP_IDS: &[&str] = &[
    "228980", "858280", "961940", "1070560", "1113280", "1161040", "1245040", "1391110", "1420170", "1493710", "1580130", "1628350",
    "1826330", "1887720", "2180100", "2230260", "2348590", "2805730", "3658110",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamInstalledGame {
    pub app_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamManifest {
    pub app_id: String,
    pub name: String,
    pub fully_installed: bool,
}

/// `steam://<action>/<id>`.
pub fn steam_uri(action: &str, app_id: &str) -> String {
    format!("steam://{action}/{app_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamAction {
    Install,
    Uninstall,
}

impl SteamAction {
    pub fn as_str(self) -> &'static str {
        match self {
            SteamAction::Install => "install",
            SteamAction::Uninstall => "uninstall",
        }
    }
}

/// `steam` (or `steam-runtime`) on `$PATH`; `steam.exe` of the Steam
/// folder on Windows.
pub fn find_steam_binary(probe: &dyn Probe) -> Option<String> {
    if cfg!(windows) {
        return platform_roots(Path::new(probe.home().as_str()))
            .into_iter()
            .map(|root| root.join("steam.exe").to_string_lossy().into_owned())
            .find(|exe| probe.exists(exe));
    }
    ["steam", "steam-runtime", FLATPAK_STEAM_ID].iter().find_map(|n| probe.which(n)).or_else(|| {
        // Flatpak's launchers, which are not always on `$PATH`.
        let user = format!("{}/.local/share/flatpak/exports/bin/{FLATPAK_STEAM_ID}", probe.home().trim_end_matches('/'));
        [user, format!("/var/lib/flatpak/exports/bin/{FLATPAK_STEAM_ID}")].into_iter().find(|p| probe.exists(p))
    })
}

/// Flatpak id of the Steam client (Fedora, Bazzite and other immutable setups).
pub const FLATPAK_STEAM_ID: &str = "com.valvesoftware.Steam";

/// Steam's folder inside the Flatpak sandbox data.
pub fn flatpak_steam_root(home: &Path) -> PathBuf {
    home.join(".var/app").join(FLATPAK_STEAM_ID).join(".local/share/Steam")
}

/// Steam's folder for the Snap (Ubuntu's App Center installs this one):
/// the Snap's home is `~/snap/steam/common`.
pub fn snap_steam_root(home: &Path) -> PathBuf {
    home.join("snap/steam/common/.local/share/Steam")
}

/// Where Steam installs itself on this system, the usual place first: on
/// Linux the native folders, the Flatpak's and the Snap's; on Windows the
/// folder the Steam client wrote into the registry, then the default one.
pub fn platform_roots(home: &Path) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let _ = home;
        windows_roots()
    }
    #[cfg(not(windows))]
    {
        vec![home.join(".steam/steam"), home.join(".local/share/Steam"), flatpak_steam_root(home), snap_steam_root(home)]
    }
}

#[cfg(windows)]
fn windows_roots() -> Vec<PathBuf> {
    use heronite_platform::registry::{string, Hive};
    let mut roots: Vec<PathBuf> = [
        (Hive::CurrentUser, r"Software\Valve\Steam", "SteamPath"),
        (Hive::LocalMachine, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (Hive::LocalMachine, r"SOFTWARE\Valve\Steam", "InstallPath"),
    ]
    .into_iter()
    .filter_map(|(hive, key, name)| string(hive, key, name))
    .filter(|p| !p.trim().is_empty())
    // `SteamPath` is written with forward slashes (`c:/program files (x86)/steam`).
    .map(|p| PathBuf::from(p.trim().replace('/', "\\")))
    .collect();
    let program_files =
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    roots.push(program_files.join("Steam"));
    let mut seen = HashSet::new();
    roots.retain(|r| seen.insert(r.to_string_lossy().to_lowercase()));
    roots
}

/// Whether the Steam client runs on Windows: the pid it wrote into the
/// registry (`ActiveProcess`, 0 once it closed) lives, or a `steam.exe`
/// runs (the registry is not always kept up to date).
fn windows_client_running(procs: &dyn Procs) -> bool {
    #[cfg(windows)]
    {
        use heronite_platform::registry::{dword, Hive};
        dword(Hive::CurrentUser, r"Software\Valve\Steam\ActiveProcess", "pid").is_some_and(|pid| pid != 0 && procs.is_alive(pid))
            || heronite_platform::process::snapshot().iter().any(|p| p.name.eq_ignore_ascii_case("steam.exe"))
    }
    #[cfg(not(windows))]
    {
        let _ = procs;
        false
    }
}

/// Opens `steam://install/<id>` or `steam://uninstall/<id>` in the Steam client.
pub fn open_steam_client_uri(spawner: &dyn Spawner, probe: &dyn Probe, action: SteamAction, app_id: &str) -> Result<(), String> {
    if find_steam_binary(probe).is_none() {
        return Err(tr!("Steam client was not found").into());
    }
    spawner.open_uri(&steam_uri(action.as_str(), app_id)).map_err(|e| e.to_string())
}

const ENV_BLOCK_EXACT: &[&str] = &[
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "LD_AUDIT",
    "APPIMAGE",
    "APPDIR",
    "OWD",
    "WINEDLLOVERRIDES",
    "WINEPREFIX",
    "GI_TYPELIB_PATH",
    "GTK_PATH",
    "QT_PLUGIN_PATH",
    "PYTHONPATH",
];

const ENV_BLOCK_PREFIX: &[&str] = &[
    "ELECTRON_",
    "CHROME_",
    "GIO_LAUNCHED_DESKTOP_FILE",
    "MANGOHUD",
    "ENABLE_VKBASALT",
    "OBS_VKCAPTURE",
    "STEAM_RUNTIME",
    "STEAM_COMPAT_",
    "PRESSURE_VESSEL_",
    "PROTON_",
    "DXVK_",
    "VKD3D_",
    "GST_PLUGIN_",
    "NODE_",
];

/// Whether `key` must not leak into the desktop's protocol handler (it would
/// break Steam's own runtime). Mirrors `STEAM_PROTOCOL_ENV_BLOCKLIST`.
pub fn is_blocked_protocol_env(key: &str) -> bool {
    ENV_BLOCK_EXACT.contains(&key) || ENV_BLOCK_PREFIX.iter().any(|p| key.starts_with(p))
}

/// The cleaned environment used to run `xdg-open steam://...`.
pub fn desktop_protocol_env(env: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    env.into_iter().filter(|(k, _)| !is_blocked_protocol_env(k)).collect()
}

/// Where Steam lives on this machine.
#[derive(Debug, Clone)]
pub struct SteamLocator {
    pub home: PathBuf,
    /// Heroic's `defaultSteamPath` setting.
    pub steam_path: PathBuf,
    /// Always-checked system roots (`/usr/share/steam`).
    pub system_roots: Vec<PathBuf>,
}

impl SteamLocator {
    /// The configured Steam folder when it exists, else the first existing
    /// of the native folders and the Flatpak one.
    pub fn detect(global: &GlobalSettings, probe: &dyn Probe) -> Self {
        let home = PathBuf::from(probe.home());
        let configured = global.default_steam_path().filter(|p| !p.is_empty()).map(PathBuf::from);
        let roots = platform_roots(&home);
        let native = roots[0].clone();
        let candidates = configured.iter().cloned().chain(roots);
        let found = candidates.into_iter().find(|p| probe.exists(&p.to_string_lossy()));
        let steam_path = found.or(configured).unwrap_or(native);
        let system_roots = if cfg!(windows) { Vec::new() } else { vec![PathBuf::from("/usr/share/steam")] };
        Self { home, steam_path, system_roots }
    }

    /// `steamLibraryRoots`: system root, libraries from `libraryfolders.vdf`,
    /// and the two usual install dirs, deduplicated and existing.
    pub fn library_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self.system_roots.clone();
        let vdf_file = self.steam_path.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(&vdf_file) {
            match Vdf::parse(&text) {
                Ok(doc) => {
                    if let Some(folders) = doc.get("libraryfolders") {
                        for (_, folder) in folders.entries() {
                            // Modern files nest `path`; very old ones map index -> path.
                            let path = match folder {
                                Vdf::Obj(_) => folder.str_of("path"),
                                Vdf::Str(s) => Some(s.as_str()),
                            };
                            if let Some(p) = path.filter(|p| Path::new(p).is_absolute()) {
                                roots.push(PathBuf::from(p));
                            }
                        }
                    }
                }
                Err(e) => log::warn!("{}: {e}", vdf_file.display()),
            }
        }
        roots.extend(platform_roots(&self.home));
        dedup_existing(roots)
    }

    /// Every Steam install on this machine: the chosen folder, then the
    /// native, Flatpak and Snap ones that exist (each once, symlinks
    /// resolved).
    pub fn installs(&self) -> Vec<PathBuf> {
        let all = std::iter::once(self.steam_path.clone()).chain(platform_roots(&self.home));
        let mut seen = Vec::new();
        all.filter(|p| p.join("config").is_dir() || p.join("steam.sh").is_file())
            .filter(|p| {
                let real = fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                let new = !seen.contains(&real);
                seen.push(real);
                new
            })
            .collect()
    }

    /// Folder of an installed Steam game: `steamapps/common/<installdir>` of
    /// the first library whose `appmanifest_<id>.acf` lists it.
    pub fn install_dir(&self, app_id: &str) -> Option<PathBuf> {
        if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        self.library_roots().into_iter().find_map(|root| {
            let apps = root.join("steamapps");
            let text = fs::read_to_string(apps.join(format!("appmanifest_{app_id}.acf"))).ok()?;
            let doc = Vdf::parse(&text).ok()?;
            let dir = doc.get("AppState")?.str_of("installdir").filter(|d| !d.is_empty())?;
            Some(apps.join("common").join(dir)).filter(|p| p.is_dir())
        })
    }

    /// `SizeOnDisk` of an installed Steam game's `appmanifest_<id>.acf`, in bytes.
    pub fn size_on_disk(&self, app_id: &str) -> Option<u64> {
        if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        self.library_roots().into_iter().find_map(|root| {
            let text = fs::read_to_string(root.join("steamapps").join(format!("appmanifest_{app_id}.acf"))).ok()?;
            let doc = Vdf::parse(&text).ok()?;
            doc.get("AppState")?.str_of("SizeOnDisk")?.parse::<u64>().ok().filter(|n| *n > 0)
        })
    }

    /// Steam console logs that mention game processes.
    pub fn log_files(&self) -> Vec<PathBuf> {
        const NAMES: [&str; 3] = ["console_log.txt", "console-linux.txt", "console-windows.txt"];
        let mut dirs: Vec<PathBuf> = self.library_roots().into_iter().map(|r| r.join("logs")).collect();
        dirs.extend(platform_roots(&self.home).into_iter().map(|r| r.join("logs")));
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for dir in dirs {
            for name in NAMES {
                let file = dir.join(name);
                if file.exists() {
                    let key = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
                    if seen.insert(key) {
                        out.push(file);
                    }
                }
            }
        }
        out
    }

    /// Whether the Steam client runs: a `steam.pid` it writes (native or
    /// Flatpak) names a live process. In Steam Deck game mode Steam is the
    /// session, so it always counts as running there.
    pub fn client_running(&self, procs: &dyn Procs) -> bool {
        if cfg!(windows) {
            return windows_client_running(procs);
        }
        if std::env::var("XDG_CURRENT_DESKTOP").as_deref() == Ok("gamescope") {
            return true;
        }
        let mut files = vec![self.steam_path.join("steam.pid"), self.home.join(".steam/steam.pid")];
        files.extend(self.steam_path.parent().map(|p| p.join("steam.pid")));
        files.push(self.home.join(".var/app").join(FLATPAK_STEAM_ID).join(".steam/steam.pid"));
        files.iter().any(|f| fs::read_to_string(f).ok().and_then(|s| s.trim().parse::<u32>().ok()).is_some_and(|pid| procs.is_alive(pid)))
    }
}

/// Global setting: close Steam once the game that made Heronite open it ends.
pub const KEY_CLOSE_AFTER_GAME: &str = "heroniteCloseSteamAfterGame";

fn dedup_existing(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in paths {
        if !p.exists() {
            continue;
        }
        let key = fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            out.push(p);
        }
    }
    out
}

fn non_game_regexes() -> &'static [Regex; 3] {
    static RE: OnceLock<[Regex; 3]> = OnceLock::new();
    RE.get_or_init(|| {
        [
            Regex::new(r"(?i)^(proton(\s|$|\d)|steam linux runtime|steamworks common redistributables|steam runtime)").unwrap(),
            Regex::new(r"(?i)\b(proton experimental|proton hotfix|easyanticheat runtime|battleye runtime)\b").unwrap(),
            Regex::new(r"(?i)\b(dedicated server|sdk|soundtrack|ost)\b").unwrap(),
        ]
    })
}

/// Proton builds, runtimes, SDKs, soundtracks and servers are not games.
pub fn is_non_game_steam_app(name: &str, app_id: &str) -> bool {
    SKIP_STEAM_APP_IDS.contains(&app_id) || non_game_regexes().iter().any(|r| r.is_match(name))
}

/// Parses an `appmanifest_<id>.acf`. A file that does not parse counts as an
/// installed app without a name (Heroic behaves the same way).
pub fn parse_app_manifest(text: &str, fallback_app_id: &str) -> SteamManifest {
    let Ok(doc) = Vdf::parse(text) else {
        return SteamManifest { app_id: fallback_app_id.into(), name: String::new(), fully_installed: true };
    };
    let state = doc.get("AppState");
    let app_id = state.and_then(|s| s.str_of("appid")).filter(|s| !s.is_empty()).unwrap_or(fallback_app_id).to_string();
    let flags = match state.and_then(|s| s.str_of("StateFlags")) {
        Some(f) => f.trim().parse::<u64>().ok(),
        None => Some(STEAM_FULLY_INSTALLED),
    };
    SteamManifest {
        app_id,
        name: state.and_then(|s| s.str_of("name")).unwrap_or("").trim().to_string(),
        fully_installed: flags.map(|f| f & STEAM_FULLY_INSTALLED == STEAM_FULLY_INSTALLED).unwrap_or(false),
    }
}

/// Every `appmanifest_*.acf` in every library, first occurrence of an id wins.
pub fn read_manifests(locator: &SteamLocator) -> Vec<SteamManifest> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in locator.library_roots() {
        let steamapps = root.join("steamapps");
        let Ok(rd) = fs::read_dir(&steamapps) else {
            continue;
        };
        let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names {
            let lower = name.to_lowercase();
            let Some(id) = lower.strip_prefix("appmanifest_").and_then(|r| r.strip_suffix(".acf")) else {
                continue;
            };
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let text = fs::read_to_string(steamapps.join(&name)).unwrap_or_default();
            let manifest = parse_app_manifest(&text, id);
            if seen.insert(manifest.app_id.clone()) {
                out.push(manifest);
            }
        }
    }
    out
}

/// Ids of fully installed Steam apps (games, tools and runtimes alike).
pub fn installed_app_ids(locator: &SteamLocator) -> HashSet<String> {
    read_manifests(locator).into_iter().filter(|m| m.fully_installed).map(|m| m.app_id).collect()
}

/// Installed Steam *games*, without Proton/runtimes/SDKs/soundtracks.
pub fn list_installed_games(locator: &SteamLocator) -> Vec<SteamInstalledGame> {
    read_manifests(locator)
        .into_iter()
        .filter(|m| m.fully_installed && !m.name.is_empty() && !is_non_game_steam_app(&m.name, &m.app_id))
        .map(|m| SteamInstalledGame { app_id: m.app_id, name: m.name })
        .collect()
}

pub fn is_steam_app_installed(locator: &SteamLocator, app_id: &str) -> bool {
    !app_id.is_empty() && installed_app_ids(locator).contains(app_id)
}

/// Steam CDN covers used for games detected from manifests.
pub fn steam_cdn_covers(app_id: &str) -> (String, String) {
    (
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app_id}/header.jpg"),
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app_id}/library_600x900_2x.jpg"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[cfg(unix)]
    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn acf(id: &str, name: &str, flags: &str) -> String {
        format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n}}\n")
    }

    #[test]
    fn manifest_flags_and_fallbacks() {
        assert!(parse_app_manifest(&acf("400", "Portal", "4"), "400").fully_installed);
        assert!(parse_app_manifest(&acf("400", "Portal", "6"), "400").fully_installed);
        assert!(!parse_app_manifest(&acf("400", "Portal", "1026"), "400").fully_installed);
        assert!(!parse_app_manifest(&acf("400", "Portal", "nope"), "400").fully_installed);
        // Missing StateFlags defaults to installed; broken files too.
        assert!(parse_app_manifest("\"AppState\" { \"appid\" \"7\" }", "7").fully_installed);
        let broken = parse_app_manifest("\"AppState\" {", "9");
        assert_eq!((broken.app_id.as_str(), broken.name.as_str(), broken.fully_installed), ("9", "", true));
        assert_eq!(parse_app_manifest(&acf("12", "  Spaced  ", "4"), "1").name, "Spaced");
    }

    #[test]
    fn non_game_filters() {
        assert!(is_non_game_steam_app("Proton 9.0", "2805730"));
        assert!(is_non_game_steam_app("Proton Experimental", "1493710"));
        assert!(is_non_game_steam_app("Proton", "1"));
        assert!(is_non_game_steam_app("Steam Linux Runtime 3.0 (sniper)", "1628350"));
        assert!(is_non_game_steam_app("Steamworks Common Redistributables", "228980"));
        assert!(is_non_game_steam_app("Portal Dedicated Server", "5"));
        assert!(is_non_game_steam_app("Some Game Soundtrack", "6"));
        assert!(is_non_game_steam_app("Source SDK Base 2013", "243730"));
        assert!(is_non_game_steam_app("Anything", "1493710"));
        assert!(!is_non_game_steam_app("Portal", "400"));
        assert!(!is_non_game_steam_app("Protonmail Simulator", "9")); // "proton" followed by a letter
        assert!(!is_non_game_steam_app("Frostpunk", "323190"));
    }

    // Linux Steam folders (native, Flatpak, Snap).
    #[cfg(unix)]
    #[test]
    fn flatpak_steam_is_found_when_the_native_one_is_missing() {
        use crate::probe::FakeProbe;
        let flatpak = "/h/.var/app/com.valvesoftware.Steam/.local/share/Steam";
        let probe = FakeProbe::new("/h").with_path(flatpak);
        let global = GlobalSettings::default();
        assert_eq!(SteamLocator::detect(&global, &probe).steam_path, PathBuf::from(flatpak));
        // A native install wins.
        let probe = probe.with_path("/h/.steam/steam");
        assert_eq!(SteamLocator::detect(&global, &probe).steam_path, PathBuf::from("/h/.steam/steam"));
        // Nothing installed: the usual default.
        assert_eq!(SteamLocator::detect(&global, &FakeProbe::new("/h")).steam_path, PathBuf::from("/h/.steam/steam"));
        // The Snap is found too.
        let snap = "/h/snap/steam/common/.local/share/Steam";
        assert_eq!(SteamLocator::detect(&global, &FakeProbe::new("/h").with_path(snap)).steam_path, PathBuf::from(snap));
    }

    // Linux Steam folders (native, Flatpak, Snap).
    #[cfg(unix)]
    #[test]
    fn the_flatpak_steam_launcher_counts_as_a_steam_binary() {
        use crate::probe::FakeProbe;
        assert_eq!(find_steam_binary(&FakeProbe::new("/h")), None);
        let user = "/h/.local/share/flatpak/exports/bin/com.valvesoftware.Steam";
        assert_eq!(find_steam_binary(&FakeProbe::new("/h").with_path(user)).as_deref(), Some(user));
        let system = "/var/lib/flatpak/exports/bin/com.valvesoftware.Steam";
        assert_eq!(find_steam_binary(&FakeProbe::new("/h").with_path(system)).as_deref(), Some(system));
        assert_eq!(find_steam_binary(&FakeProbe::new("/h").with_bin("steam", "/usr/bin/steam")).as_deref(), Some("/usr/bin/steam"));
    }

    // Linux Steam folders (native, Flatpak, Snap).
    #[cfg(unix)]
    #[test]
    fn flatpak_libraries_are_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let flatpak = flatpak_steam_root(&home);
        write(&flatpak.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal", "4"));
        let loc = SteamLocator { home: home.clone(), steam_path: home.join(".steam/steam"), system_roots: vec![] };
        assert!(installed_app_ids(&loc).contains("400"));
    }

    // TODO(windows): not isolated there: `library_roots` adds the Steam folder of
    // the registry (this machine's real Steam) and not the test's `home`.
    #[cfg(unix)]
    #[test]
    fn install_dir_follows_the_manifest_of_any_library() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let steam = home.join(".steam").join("steam");
        let extra = dir.path().join("SteamLibrary");
        write(
            &steam.join("steamapps/libraryfolders.vdf"),
            &format!("\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} }}", extra.display()),
        );
        write(
            &extra.join("steamapps/appmanifest_1245620.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\"1245620\"\n\t\"installdir\"\t\"ELDEN RING\"\n}\n",
        );
        fs::create_dir_all(extra.join("steamapps/common/ELDEN RING")).unwrap();
        write(&steam.join("steamapps/appmanifest_400.acf"), "\"AppState\" { \"installdir\" \"Portal\" }");
        let loc = SteamLocator { home, steam_path: steam, system_roots: vec![] };
        assert_eq!(loc.install_dir("1245620"), Some(extra.join("steamapps/common/ELDEN RING")));
        // Listed but the folder is gone; unknown id; not an id.
        assert_eq!(loc.install_dir("400"), None);
        assert_eq!(loc.install_dir("9"), None);
        assert_eq!(loc.install_dir("../x"), None);
    }

    // TODO(windows): not isolated there: `library_roots` adds the Steam folder of
    // the registry (this machine's real Steam) and not the test's `home`.
    #[cfg(unix)]
    #[test]
    fn size_on_disk_comes_from_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("home/.steam/steam");
        write(&steam.join("steamapps/appmanifest_400.acf"), "\"AppState\" { \"installdir\" \"Portal\" \"SizeOnDisk\" \"4518092416\" }");
        write(&steam.join("steamapps/appmanifest_620.acf"), "\"AppState\" { \"installdir\" \"Portal 2\" \"SizeOnDisk\" \"0\" }");
        let loc = SteamLocator { home: dir.path().join("home"), steam_path: steam, system_roots: vec![] };
        assert_eq!(loc.size_on_disk("400"), Some(4_518_092_416));
        assert_eq!(loc.size_on_disk("620"), None, "not downloaded yet");
        assert_eq!(loc.size_on_disk("9"), None);
    }

    // TODO(windows): not isolated there: `library_roots` adds the Steam folder of
    // the registry (this machine's real Steam) and not the test's `home`.
    #[cfg(unix)]
    #[test]
    fn scans_libraries_from_libraryfolders() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let steam = home.join(".steam").join("steam");
        let extra = dir.path().join("games").join("SteamLibrary");
        write(
            &steam.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} \"2\" {{ \"path\" \"/does/not/exist\" }} }}",
                steam.display(),
                extra.display()
            ),
        );
        write(&steam.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal", "4"));
        write(&steam.join("steamapps/appmanifest_1493710.acf"), &acf("1493710", "Proton Experimental", "4"));
        write(&steam.join("steamapps/appmanifest_620.acf"), &acf("620", "Portal 2", "1026"));
        write(&extra.join("steamapps/appmanifest_730.acf"), &acf("730", "Counter-Strike 2", "4"));
        write(&extra.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal (dup)", "4"));
        write(&extra.join("steamapps/appmanifest_abc.acf"), &acf("1", "ignored", "4"));
        write(&extra.join("steamapps/readme.txt"), "x");

        let loc = SteamLocator { home: home.clone(), steam_path: steam.clone(), system_roots: vec![] };
        let roots = loc.library_roots();
        assert!(roots.contains(&steam) && roots.contains(&extra));
        assert!(!roots.iter().any(|r| r.ends_with("exist")));

        let ids = installed_app_ids(&loc);
        assert_eq!(ids, ["400", "1493710", "730"].iter().map(|s| s.to_string()).collect());
        let games = list_installed_games(&loc);
        let names: Vec<_> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Portal", "Counter-Strike 2"]);
        assert!(is_steam_app_installed(&loc, "730"));
        assert!(!is_steam_app_installed(&loc, "620"));
        assert!(!is_steam_app_installed(&loc, ""));
    }

    #[test]
    fn protocol_env_drops_runtime_variables() {
        let kept = desktop_protocol_env(
            [
                ("HOME", "/h"),
                ("PATH", "/bin"),
                ("LD_LIBRARY_PATH", "/x"),
                ("LD_PRELOAD", ""),
                ("ELECTRON_RUN_AS_NODE", "1"),
                ("STEAM_COMPAT_DATA_PATH", "/p"),
                ("PROTON_LOG", "1"),
                ("DISPLAY", ":0"),
                ("WINEPREFIX", "/w"),
                ("XDG_SESSION_TYPE", "wayland"),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let keys: Vec<_> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["HOME", "PATH", "DISPLAY", "XDG_SESSION_TYPE"]);
    }

    #[test]
    fn install_and_uninstall_go_through_the_desktop_handler() {
        use crate::probe::FakeProbe;
        use crate::runtime::FakeSpawner;
        let spawner = FakeSpawner::new();
        // Linux finds `steam` on the PATH, Windows `steam.exe` in Steam's folder.
        let with_steam = if cfg!(windows) {
            FakeProbe::new("/h").with_path(&platform_roots(Path::new("/h"))[0].join("steam.exe").to_string_lossy())
        } else {
            FakeProbe::new("/h").with_bin("steam", "/usr/bin/steam")
        };
        open_steam_client_uri(&spawner, &with_steam, SteamAction::Install, "400").unwrap();
        open_steam_client_uri(&spawner, &with_steam, SteamAction::Uninstall, "400").unwrap();
        assert_eq!(spawner.uris(), ["steam://install/400", "steam://uninstall/400"]);
        let err = open_steam_client_uri(&spawner, &FakeProbe::new("/h"), SteamAction::Install, "1").unwrap_err();
        assert_eq!(err, "Steam client was not found");
        assert_eq!(spawner.uris().len(), 2, "nothing is opened without a Steam client");
        spawner.fail_next_uri("xdg-open: not found");
        assert!(open_steam_client_uri(&spawner, &with_steam, SteamAction::Install, "1").unwrap_err().contains("xdg-open"));
    }

    #[test]
    fn uri_building() {
        assert_eq!(steam_uri("rungameid", "400"), "steam://rungameid/400");
        assert_eq!(steam_uri(SteamAction::Uninstall.as_str(), "7"), "steam://uninstall/7");
    }

    // Windows makes symlinks only in Developer Mode or as an administrator.
    #[cfg(unix)]
    #[test]
    fn log_files_found_and_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write(&home.join(".local/share/Steam/logs/console_log.txt"), "x");
        std::os::unix::fs::symlink(home.join(".local/share/Steam"), home.join(".steam_link")).unwrap();
        fs::create_dir_all(home.join(".steam")).unwrap();
        std::os::unix::fs::symlink(home.join(".local/share/Steam"), home.join(".steam/steam")).unwrap();
        let loc = SteamLocator { home: home.to_path_buf(), steam_path: home.join(".steam/steam"), system_roots: vec![] };
        let files = loc.log_files();
        assert_eq!(files.len(), 1, "{files:?}");
    }
}
