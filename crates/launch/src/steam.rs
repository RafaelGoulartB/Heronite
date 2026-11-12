//! Steam helpers: `steam://` URIs, installed-app detection from
//! `libraryfolders.vdf` + `appmanifest_*.acf`, and Steam's log files
//! (port of `steam.ts`).

use std::collections::{HashMap, HashSet};
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
        return probe
            .steam_roots()
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
    /// Ordered installation candidates captured at detection. Later scans
    /// never rediscover folders from the host's registry or environment.
    pub platform_roots: Vec<PathBuf>,
}

impl SteamLocator {
    /// The configured Steam folder when it exists, else the first existing
    /// platform candidate. With none installed, retain the configured path
    /// or the first candidate (an empty probe falls back under its home).
    pub fn detect(global: &GlobalSettings, probe: &dyn Probe) -> Self {
        let home = PathBuf::from(probe.home());
        let configured = global.default_steam_path().filter(|p| !p.is_empty()).map(PathBuf::from);
        let platform_roots = probe.steam_roots();
        let native = platform_roots.first().cloned().unwrap_or_else(|| home.join(".steam/steam"));
        let candidates = configured.iter().cloned().chain(platform_roots.iter().cloned());
        let found = candidates.into_iter().find(|p| probe.exists(&p.to_string_lossy()));
        let steam_path = found.or(configured).unwrap_or(native);
        let system_roots = probe.steam_system_roots();
        Self { home, steam_path, system_roots, platform_roots }
    }

    /// `steamLibraryRoots`: system roots, the libraries listed in the
    /// `libraryfolders.vdf` of every Steam install (the chosen one first),
    /// then the captured installation candidates, deduplicated and existing.
    ///
    /// Every install's list counts, not only the chosen folder's: the chosen
    /// one can be a stale install (`~/.steam/steam` left from the native
    /// package) while Steam runs from its Snap or Flatpak, whose own folder is
    /// a candidate but whose extra libraries (a second drive) are only
    /// listed in its `libraryfolders.vdf`.
    pub fn library_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self.system_roots.clone();
        // The chosen folder's list as before, then those of the other Steam
        // installs (folders with Steam's `config` or `steam.sh`, not any
        // leftover folder).
        let installs = std::iter::once(self.steam_path.clone()).chain(self.installs()).collect();
        for install in dedup_existing(installs) {
            roots.extend(library_folders(&install.join("steamapps").join("libraryfolders.vdf")));
        }
        roots.extend(self.platform_roots.iter().cloned());
        dedup_existing(roots)
    }

    /// Every Steam install on this machine: the chosen folder, then the
    /// native, Flatpak and Snap ones that exist (each once, symlinks
    /// resolved).
    pub fn installs(&self) -> Vec<PathBuf> {
        let all = std::iter::once(self.steam_path.clone()).chain(self.platform_roots.iter().cloned());
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

    /// Steam's `userdata` folder that keeps `app_id`'s saves. Of the Steam
    /// installs with a `userdata` folder (the chosen one first): one whose
    /// libraries hold the game (the one with a folder for the game in an
    /// account if several do), else one with such a folder, else the chosen
    /// one. The chosen folder can be a stale
    /// install while Steam runs from its Snap or Flatpak.
    pub fn userdata_dir(&self, app_id: &str) -> Option<PathBuf> {
        let installs: Vec<PathBuf> = dedup_existing(std::iter::once(self.steam_path.clone()).chain(self.installs()).collect())
            .into_iter()
            .filter(|i| i.join("userdata").is_dir())
            .collect();
        let valid_id = !app_id.is_empty() && app_id.bytes().all(|b| b.is_ascii_digit());
        let has_game_folder = |install: &&PathBuf| {
            fs::read_dir(install.join("userdata")).is_ok_and(|rd| rd.flatten().any(|account| account.path().join(app_id).is_dir()))
        };
        let holds_game = |install: &&PathBuf| {
            let manifest = format!("appmanifest_{app_id}.acf");
            std::iter::once(install.to_path_buf())
                .chain(library_folders(&install.join("steamapps").join("libraryfolders.vdf")))
                .any(|lib| lib.join("steamapps").join(&manifest).is_file())
        };
        let found = valid_id.then(|| {
            let holders: Vec<&PathBuf> = installs.iter().filter(|i| holds_game(i)).collect();
            if holders.is_empty() {
                installs.iter().find(has_game_folder)
            } else {
                holders.iter().copied().find(has_game_folder).or(holders.first().copied())
            }
        });
        found.flatten().or(installs.first()).map(|i| i.join("userdata"))
    }

    /// Every library's `appmanifest_<id>.acf` as `(steamapps, text)`, in
    /// library order but fully installed ones first: a manifest left by a move
    /// to another drive that stopped half way does not stand for the game.
    fn app_manifests(&self, app_id: &str) -> Vec<(PathBuf, String)> {
        if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
            return Vec::new();
        }
        let mut found: Vec<(PathBuf, String)> = self
            .library_roots()
            .into_iter()
            .filter_map(|root| {
                let apps = root.join("steamapps");
                let text = fs::read_to_string(apps.join(format!("appmanifest_{app_id}.acf"))).ok()?;
                Some((apps, text))
            })
            .collect();
        found.sort_by_key(|(_, text)| !parse_app_manifest(text, app_id).fully_installed);
        found
    }

    /// Folder of an installed Steam game: `steamapps/common/<installdir>` of
    /// the first library whose `appmanifest_<id>.acf` lists it (a fully
    /// installed one first).
    pub fn install_dir(&self, app_id: &str) -> Option<PathBuf> {
        self.app_manifests(app_id).into_iter().find_map(|(apps, text)| {
            let doc = Vdf::parse(&text).ok()?;
            let dir = doc.get("AppState")?.str_of("installdir").filter(|d| !d.is_empty())?;
            Some(apps.join("common").join(dir)).filter(|p| p.is_dir())
        })
    }

    /// `SizeOnDisk` of an installed Steam game's `appmanifest_<id>.acf`, in bytes.
    pub fn size_on_disk(&self, app_id: &str) -> Option<u64> {
        self.app_manifests(app_id).into_iter().find_map(|(_, text)| {
            let doc = Vdf::parse(&text).ok()?;
            doc.get("AppState")?.str_of("SizeOnDisk")?.parse::<u64>().ok().filter(|n| *n > 0)
        })
    }

    /// Steam console logs that mention game processes.
    pub fn log_files(&self) -> Vec<PathBuf> {
        const NAMES: [&str; 3] = ["console_log.txt", "console-linux.txt", "console-windows.txt"];
        let mut dirs: Vec<PathBuf> = self.library_roots().into_iter().map(|r| r.join("logs")).collect();
        dirs.extend(self.platform_roots.iter().map(|r| r.join("logs")));
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

    /// Whether the Steam client runs: the platform's client check, or on
    /// Linux a native/Flatpak `steam.pid` naming a live process.
    pub fn client_running(&self, procs: &dyn Procs) -> bool {
        if cfg!(windows) {
            return procs.steam_client_running();
        }
        if procs.steam_client_running() {
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

/// The absolute library paths a `libraryfolders.vdf` lists (none when the
/// file is missing or broken).
fn library_folders(vdf_file: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(vdf_file) else {
        return Vec::new();
    };
    let doc = match Vdf::parse(&text) {
        Ok(doc) => doc,
        Err(e) => {
            log::warn!("{}: {e}", vdf_file.display());
            return Vec::new();
        }
    };
    let Some(folders) = doc.get("libraryfolders") else {
        return Vec::new();
    };
    folders
        .entries()
        .iter()
        .filter_map(|(_, folder)| match folder {
            // Modern files nest `path`; very old ones map index -> path.
            Vdf::Obj(_) => folder.str_of("path"),
            Vdf::Str(s) => Some(s.as_str()),
        })
        .filter(|p| Path::new(p).is_absolute())
        .map(PathBuf::from)
        .collect()
}

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

/// Every `appmanifest_*.acf` in every library, one per id: the first fully
/// installed one, else the first one. A leftover manifest (an interrupted
/// move to another drive) must not hide the game installed in a later library.
pub fn read_manifests(locator: &SteamLocator) -> Vec<SteamManifest> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<SteamManifest> = Vec::new();
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
            match seen.get(&manifest.app_id) {
                None => {
                    seen.insert(manifest.app_id.clone(), out.len());
                    out.push(manifest);
                }
                Some(&i) if !out[i].fully_installed && manifest.fully_installed => out[i] = manifest,
                Some(_) => {}
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

    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn vdf_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"")
    }

    #[test]
    fn detection_preserves_candidate_order_and_missing_path_fallbacks() {
        use crate::probe::FakeProbe;
        let dir = tempfile::tempdir().unwrap();
        let (first, second, configured) = (dir.path().join("first"), dir.path().join("second"), dir.path().join("configured"));
        let roots = vec![first.clone(), second.clone()];
        let probe = FakeProbe::new(dir.path().to_str().unwrap()).with_steam_roots(roots.clone());
        let empty = GlobalSettings::default();
        assert_eq!(SteamLocator::detect(&empty, &probe).steam_path, first);
        let mut global = GlobalSettings::default();
        global.defaults.insert("defaultSteamPath".into(), configured.to_string_lossy().into_owned().into());
        assert_eq!(SteamLocator::detect(&global, &probe).steam_path, configured, "missing configured folder is the fallback");
        let probe = probe.with_path(second.to_str().unwrap());
        assert_eq!(SteamLocator::detect(&global, &probe).steam_path, second, "an existing candidate beats a missing configured folder");
        let probe = probe.with_path(first.to_str().unwrap());
        assert_eq!(SteamLocator::detect(&empty, &probe).steam_path, first, "platform ordering is kept");
        let probe = probe.with_path(configured.to_str().unwrap());
        let locator = SteamLocator::detect(&global, &probe);
        assert_eq!(locator.steam_path, configured, "existing configured folder wins");
        assert_eq!(locator.platform_roots, roots);
        assert!(locator.system_roots.is_empty());
    }

    #[test]
    fn later_scans_use_only_the_captured_roots() {
        use crate::probe::FakeProbe;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let (steam, extra, system) = (home.join(".steam/steam"), dir.path().join("extra"), dir.path().join("system"));
        for root in [&steam, &extra, &system] {
            fs::create_dir_all(root.join("config")).unwrap();
            write(&root.join("logs/console-windows.txt"), "");
            write(&root.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal", "4"));
        }
        let mut probe = FakeProbe::new(home.to_str().unwrap()).with_steam_roots(vec![steam.clone(), extra.clone(), steam.clone()]);
        probe.steam_system_roots = vec![system.clone()];
        let locator = SteamLocator::detect(&GlobalSettings::default(), &probe);
        probe.steam_roots.clear();
        probe.steam_system_roots.clear();
        assert_eq!(locator.library_roots(), [system.clone(), steam.clone(), extra.clone()]);
        assert_eq!(locator.installs(), [steam.clone(), extra.clone()]);
        assert_eq!(
            locator.log_files(),
            [system.join("logs/console-windows.txt"), steam.join("logs/console-windows.txt"), extra.join("logs/console-windows.txt")]
        );
        let empty = SteamLocator { home: home.clone(), steam_path: home.join("missing"), system_roots: vec![], platform_roots: vec![] };
        assert!(empty.library_roots().is_empty());
        assert!(empty.installs().is_empty());
        assert!(empty.log_files().is_empty());
        assert!(installed_app_ids(&empty).is_empty());
        let empty = SteamLocator::detect(&GlobalSettings::default(), &FakeProbe::new(dir.path().to_str().unwrap()));
        assert!(empty.platform_roots.is_empty());
        assert!(empty.system_roots.is_empty());
        assert!(empty.library_roots().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn binary_detection_checks_only_injected_roots_in_order() {
        use crate::probe::FakeProbe;
        let roots = vec![PathBuf::from(r"C:\fixture\missing"), PathBuf::from(r"D:\fixture\Steam"), PathBuf::from(r"E:\fixture\Steam")];
        let second = roots[1].join("steam.exe");
        let third = roots[2].join("steam.exe");
        let probe = FakeProbe::new("/h").with_steam_roots(roots).with_path(third.to_str().unwrap()).with_path(second.to_str().unwrap());
        assert_eq!(find_steam_binary(&probe), Some(second.to_string_lossy().into_owned()));
        assert!(find_steam_binary(&FakeProbe::new("/h")).is_none());
    }

    #[test]
    fn client_running_uses_only_fake_processes_and_fixture_pid_files() {
        use crate::procs::FakeProcs;
        let dir = tempfile::tempdir().unwrap();
        let locator = SteamLocator {
            home: dir.path().to_path_buf(),
            steam_path: dir.path().join("Steam"),
            system_roots: vec![],
            platform_roots: vec![],
        };
        let procs = FakeProcs::new();
        assert!(!locator.client_running(&procs));
        if cfg!(windows) {
            procs.start(100, "steamwebhelper.exe", None);
            assert!(!locator.client_running(&procs));
            procs.start(101, "StEaM.ExE", None);
        } else {
            write(&locator.steam_path.join("steam.pid"), "101\n");
            assert!(!locator.client_running(&procs), "stale pid");
            procs.start(101, "steam", None);
        }
        assert!(locator.client_running(&procs));
        procs.exit(101);
        assert!(!locator.client_running(&procs));
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
        let roots = platform_roots(Path::new("/h"));
        let probe = FakeProbe::new("/h").with_steam_roots(roots.clone()).with_path(flatpak);
        let global = GlobalSettings::default();
        assert_eq!(SteamLocator::detect(&global, &probe).steam_path, PathBuf::from(flatpak));
        // A native install wins.
        let probe = probe.with_path("/h/.steam/steam");
        assert_eq!(SteamLocator::detect(&global, &probe).steam_path, PathBuf::from("/h/.steam/steam"));
        // Nothing installed: the usual default.
        assert_eq!(
            SteamLocator::detect(&global, &FakeProbe::new("/h").with_steam_roots(roots.clone())).steam_path,
            PathBuf::from("/h/.steam/steam")
        );
        // The Snap is found too.
        let snap = "/h/snap/steam/common/.local/share/Steam";
        assert_eq!(
            SteamLocator::detect(&global, &FakeProbe::new("/h").with_steam_roots(roots).with_path(snap)).steam_path,
            PathBuf::from(snap)
        );
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
        let loc =
            SteamLocator { home: home.clone(), steam_path: home.join(".steam/steam"), system_roots: vec![], platform_roots: vec![flatpak] };
        assert!(installed_app_ids(&loc).contains("400"));
    }

    #[test]
    fn install_dir_follows_the_manifest_of_any_library() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let steam = home.join(".steam").join("steam");
        let extra = dir.path().join("SteamLibrary");
        write(
            &steam.join("steamapps/libraryfolders.vdf"),
            &format!("\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} }}", vdf_path(&extra)),
        );
        write(
            &extra.join("steamapps/appmanifest_1245620.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\"1245620\"\n\t\"installdir\"\t\"ELDEN RING\"\n}\n",
        );
        fs::create_dir_all(extra.join("steamapps/common/ELDEN RING")).unwrap();
        write(&steam.join("steamapps/appmanifest_400.acf"), "\"AppState\" { \"installdir\" \"Portal\" }");
        let loc = SteamLocator { home, steam_path: steam.clone(), system_roots: vec![], platform_roots: vec![steam] };
        assert_eq!(loc.install_dir("1245620"), Some(extra.join("steamapps/common/ELDEN RING")));
        // Listed but the folder is gone; unknown id; not an id.
        assert_eq!(loc.install_dir("400"), None);
        assert_eq!(loc.install_dir("9"), None);
        assert_eq!(loc.install_dir("../x"), None);
    }

    #[test]
    fn size_on_disk_comes_from_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("home/.steam/steam");
        write(&steam.join("steamapps/appmanifest_400.acf"), "\"AppState\" { \"installdir\" \"Portal\" \"SizeOnDisk\" \"4518092416\" }");
        write(&steam.join("steamapps/appmanifest_620.acf"), "\"AppState\" { \"installdir\" \"Portal 2\" \"SizeOnDisk\" \"0\" }");
        let loc =
            SteamLocator { home: dir.path().join("home"), steam_path: steam.clone(), system_roots: vec![], platform_roots: vec![steam] };
        assert_eq!(loc.size_on_disk("400"), Some(4_518_092_416));
        assert_eq!(loc.size_on_disk("620"), None, "not downloaded yet");
        assert_eq!(loc.size_on_disk("9"), None);
    }

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
                vdf_path(&steam),
                vdf_path(&extra)
            ),
        );
        write(&steam.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal", "4"));
        write(&steam.join("steamapps/appmanifest_1493710.acf"), &acf("1493710", "Proton Experimental", "4"));
        write(&steam.join("steamapps/appmanifest_620.acf"), &acf("620", "Portal 2", "1026"));
        write(&extra.join("steamapps/appmanifest_730.acf"), &acf("730", "Counter-Strike 2", "4"));
        write(&extra.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal (dup)", "4"));
        write(&extra.join("steamapps/appmanifest_abc.acf"), &acf("1", "ignored", "4"));
        write(&extra.join("steamapps/readme.txt"), "x");

        let loc = SteamLocator { home: home.clone(), steam_path: steam.clone(), system_roots: vec![], platform_roots: vec![steam.clone()] };
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

    /// Issue #318: the chosen folder is a stale native install while Steam
    /// runs from its Snap, whose `libraryfolders.vdf` lists a second drive.
    #[test]
    fn libraries_listed_by_any_install_are_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let stale = home.join(".steam/steam");
        let snap = snap_steam_root(&home);
        let hdd = dir.path().join("media/HDD/SteamLibrary");
        write(
            &stale.join("steamapps/libraryfolders.vdf"),
            &format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}", vdf_path(&stale)),
        );
        write(
            &snap.join("steamapps/libraryfolders.vdf"),
            &format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}", vdf_path(&snap), vdf_path(&hdd)),
        );
        write(&snap.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal", "4"));
        write(&hdd.join("steamapps/appmanifest_730.acf"), &acf("730", "Counter-Strike 2", "4"));
        write(
            &hdd.join("steamapps/appmanifest_1245620.acf"),
            "\"AppState\" { \"appid\" \"1245620\" \"installdir\" \"ELDEN RING\" \"StateFlags\" \"4\" }",
        );
        fs::create_dir_all(hdd.join("steamapps/common/ELDEN RING")).unwrap();
        // Both are Steam installs; a Flatpak folder without `config` is a
        // leftover whose list (an old drive still on disk) is not read.
        fs::create_dir_all(stale.join("config")).unwrap();
        fs::create_dir_all(snap.join("config")).unwrap();
        let leftover = flatpak_steam_root(&home);
        let old_drive = dir.path().join("old/SteamLibrary");
        write(
            &leftover.join("steamapps/libraryfolders.vdf"),
            &format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}", vdf_path(&old_drive)),
        );
        write(&old_drive.join("steamapps/appmanifest_9.acf"), &acf("9", "Gone", "4"));

        let loc = SteamLocator {
            home: home.clone(),
            steam_path: stale.clone(),
            system_roots: vec![],
            platform_roots: vec![stale.clone(), snap.clone(), leftover.clone()],
        };
        assert_eq!(loc.library_roots(), [stale, snap, hdd.clone(), leftover]);
        assert!(is_steam_app_installed(&loc, "400"));
        assert!(is_steam_app_installed(&loc, "730"));
        assert!(!is_steam_app_installed(&loc, "9"));
        assert_eq!(loc.install_dir("1245620"), Some(hdd.join("steamapps/common/ELDEN RING")));
    }

    /// Restoring a Steam game's saves goes to the `userdata` of the Steam that
    /// has the game, not to a stale chosen folder.
    #[test]
    fn userdata_of_the_install_that_has_the_game() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let stale = home.join(".steam/steam");
        let snap = snap_steam_root(&home);
        let hdd = dir.path().join("media/HDD/SteamLibrary");
        for install in [&stale, &snap] {
            fs::create_dir_all(install.join("config")).unwrap();
            fs::create_dir_all(install.join("userdata/111")).unwrap();
        }
        write(
            &snap.join("steamapps/libraryfolders.vdf"),
            &format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}", vdf_path(&snap), vdf_path(&hdd)),
        );
        write(&hdd.join("steamapps/appmanifest_1245620.acf"), &acf("1245620", "ELDEN RING", "4"));
        write(&stale.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal", "4"));
        fs::create_dir_all(snap.join("userdata/111/620/remote")).unwrap();
        let loc = SteamLocator {
            home: home.clone(),
            steam_path: stale.clone(),
            system_roots: vec![],
            platform_roots: vec![stale.clone(), snap.clone()],
        };
        // The stale folder keeps saves from when it was the Steam in use.
        fs::create_dir_all(stale.join("userdata/111/1245620/remote")).unwrap();
        assert_eq!(loc.userdata_dir("1245620"), Some(snap.join("userdata")), "its library is listed by the Snap");
        assert_eq!(loc.userdata_dir("620"), Some(snap.join("userdata")), "the Snap has its cloud saves (not installed)");
        assert_eq!(loc.userdata_dir("400"), Some(stale.join("userdata")), "installed in the chosen folder");
        assert_eq!(loc.userdata_dir("9"), Some(stale.join("userdata")), "unknown game: the chosen folder");
        assert_eq!(loc.userdata_dir("../x"), Some(stale.join("userdata")), "not an id: the chosen folder");
        // The chosen folder without `userdata`: the other install.
        fs::remove_dir_all(stale.join("userdata")).unwrap();
        assert_eq!(loc.userdata_dir("400"), Some(snap.join("userdata")));
        let none = SteamLocator { home: home.clone(), steam_path: home.join("missing"), system_roots: vec![], platform_roots: vec![] };
        assert_eq!(none.userdata_dir("400"), None);
    }

    /// A manifest left in the main library (a move to another drive that
    /// stopped half way) does not hide the game installed on the other drive.
    #[test]
    fn a_fully_installed_manifest_beats_an_earlier_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("home/.steam/steam");
        let hdd = dir.path().join("hdd/SteamLibrary");
        write(
            &steam.join("steamapps/libraryfolders.vdf"),
            &format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}", vdf_path(&steam), vdf_path(&hdd)),
        );
        write(&steam.join("steamapps/appmanifest_730.acf"), &acf("730", "Counter-Strike 2", "1026"));
        write(&hdd.join("steamapps/appmanifest_730.acf"), &acf("730", "Counter-Strike 2", "4"));
        write(&steam.join("steamapps/appmanifest_620.acf"), &acf("620", "Portal 2", "1026"));
        write(&steam.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal", "4"));
        write(&hdd.join("steamapps/appmanifest_400.acf"), &acf("400", "Portal (old copy)", "1026"));
        let loc = SteamLocator {
            home: dir.path().join("home"),
            steam_path: steam.clone(),
            system_roots: vec![],
            platform_roots: vec![steam.clone()],
        };
        let manifests = read_manifests(&loc);
        let get = |id: &str| manifests.iter().filter(|m| m.app_id == id).collect::<Vec<_>>();
        assert!(get("730").len() == 1 && get("730")[0].fully_installed, "the HDD copy replaces the leftover");
        assert_eq!(get("400")[0].name, "Portal", "an installed one is kept over a later leftover");
        assert!(get("620").len() == 1 && !get("620")[0].fully_installed, "a lone leftover still counts as not installed");
        assert!(is_steam_app_installed(&loc, "730"));
        // Both folders exist (the copy stopped half way): the installed one.
        fs::create_dir_all(steam.join("steamapps/common/Counter-Strike 2")).unwrap();
        fs::create_dir_all(hdd.join("steamapps/common/Counter-Strike 2")).unwrap();
        let with_dir = |text: String| text.replace("}\n", "\t\"installdir\"\t\t\"Counter-Strike 2\"\n\t\"SizeOnDisk\"\t\t\"SIZE\"\n}\n");
        write(&steam.join("steamapps/appmanifest_730.acf"), &with_dir(acf("730", "Counter-Strike 2", "1026")).replace("SIZE", "10"));
        write(&hdd.join("steamapps/appmanifest_730.acf"), &with_dir(acf("730", "Counter-Strike 2", "4")).replace("SIZE", "99"));
        assert_eq!(loc.install_dir("730"), Some(hdd.join("steamapps/common/Counter-Strike 2")));
        assert_eq!(loc.size_on_disk("730"), Some(99));
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
            let root = PathBuf::from(r"C:\fixture\Steam");
            FakeProbe::new("/h").with_path(&root.join("steam.exe").to_string_lossy()).with_steam_roots(vec![root])
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
        let loc = SteamLocator {
            home: home.to_path_buf(),
            steam_path: home.join(".steam/steam"),
            system_roots: vec![],
            platform_roots: vec![home.join(".steam/steam"), home.join(".local/share/Steam")],
        };
        let files = loc.log_files();
        assert_eq!(files.len(), 1, "{files:?}");
    }
}
