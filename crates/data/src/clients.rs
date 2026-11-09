//! The Windows store clients Heronite starts games through: Battle.net, the
//! EA app, Ubisoft Connect and the Rockstar Games Launcher
//! (`heronite_clients`).
//!
//! On Linux each client lives in a Wine prefix of its own, set in the
//! client's `GamesConfig/<config app>.json` ([`Client::config_app`]) with its
//! Wine build, so the game settings panel edits it like a game's. On Windows
//! it is installed on the PC as it is ([`machine_drive`], [`machine_user`],
//! [`Client::install_keys`]) and has no settings file. The user signs
//! in inside the client, which installs, updates and starts its games; they
//! come into the Collection as sideload entries the client starts
//! ([`LAUNCH_KIND`]), whose `source` is the client's [`Client::id`].
//!
//! The client's prefix (and the games inside it) belong to the client:
//! uninstalling a game added here, Wine Manager's trash and the Storage page
//! never delete it ([`crate::launchers::managed_paths`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json_store;
use crate::models::Extra;
use crate::paths::DataPaths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Client {
    BattleNet,
    Ea,
    Ubisoft,
    Rockstar,
}

/// `launchKind` of the games a client starts (`launcherArgs` holds what the
/// client is asked to do: `--exec="launch WoW"`, `uplay://launch/635/0`).
pub const LAUNCH_KIND: &str = "client";

impl Client {
    pub const ALL: [Client; 4] = [Client::BattleNet, Client::Ea, Client::Ubisoft, Client::Rockstar];

    /// Also the `source` of its Collection entries and the start of their
    /// app names (`battlenet_wow`).
    pub fn id(self) -> &'static str {
        match self {
            Client::BattleNet => "battlenet",
            Client::Ea => "ea",
            Client::Ubisoft => "ubisoft",
            Client::Rockstar => "rockstar",
        }
    }

    /// The brand (not translated).
    pub fn name(self) -> &'static str {
        match self {
            Client::BattleNet => "Battle.net",
            Client::Ea => "EA app",
            Client::Ubisoft => "Ubisoft Connect",
            Client::Rockstar => "Rockstar Games Launcher",
        }
    }

    /// The store's short name: the "Library" of a game's details.
    pub fn store(self) -> &'static str {
        match self {
            Client::BattleNet => "Battle.net",
            Client::Ea => "EA",
            Client::Ubisoft => "Ubisoft",
            Client::Rockstar => "Rockstar",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    /// The app name of the client's `GamesConfig` file: its prefix, Wine
    /// build and the rest of its Wine settings.
    pub fn config_app(self) -> &'static str {
        match self {
            Client::BattleNet => "heronite-client-battlenet",
            Client::Ea => "heronite-client-ea",
            Client::Ubisoft => "heronite-client-ubisoft",
            Client::Rockstar => "heronite-client-rockstar",
        }
    }

    /// The client that `app_name` is the [`config_app`](Self::config_app) of.
    pub fn from_config_app(app_name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.config_app() == app_name)
    }

    /// The prefix folder a new setup gets inside `defaultWinePrefixDir`.
    pub fn prefix_folder(self) -> &'static str {
        match self {
            Client::BattleNet => "Battle.net",
            Client::Ea => "EA app",
            Client::Ubisoft => "Ubisoft Connect",
            Client::Rockstar => "Rockstar Games Launcher",
        }
    }

    /// Where the client's program sits on the C: drive (a prefix's
    /// `drive_c`, or this PC's), the one to start first. Forward slashes,
    /// relative to the drive.
    pub fn exe_candidates(self) -> &'static [&'static str] {
        match self {
            Client::BattleNet => {
                &["Program Files (x86)/Battle.net/Battle.net.exe", "Program Files (x86)/Battle.net/Battle.net Launcher.exe"]
            }
            Client::Ea => &[
                "Program Files/Electronic Arts/EA Desktop/EA Desktop/EALauncher.exe",
                "Program Files/Electronic Arts/EA Desktop/EA Desktop/EADesktop.exe",
            ],
            Client::Ubisoft => &[
                "Program Files (x86)/Ubisoft/Ubisoft Game Launcher/upc.exe",
                "Program Files (x86)/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe",
            ],
            Client::Rockstar => &["Program Files/Rockstar Games/Launcher/Launcher.exe"],
        }
    }

    /// The client's program in `prefix`, when it is installed there.
    pub fn exe_in(self, prefix: &Path) -> Option<PathBuf> {
        let c = drive_c(prefix);
        self.exe_candidates().iter().map(|rel| c.join(rel)).find(|p| p.is_file())
    }

    /// Where the client's installer writes, in the machine's registry
    /// (`HKLM`), the folder it put the client in or its program: `(key,
    /// value)`, in the order to try. What finds a client installed
    /// elsewhere than [`exe_candidates`](Self::exe_candidates) say (another
    /// drive).
    pub fn install_keys(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Client::BattleNet => &[("Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Battle.net", "InstallLocation")],
            Client::Ea => &[
                ("Software\\Electronic Arts\\EA Desktop", "LauncherAppPath"),
                ("Software\\Electronic Arts\\EA Desktop", "ClientPath"),
                ("Software\\Wow6432Node\\Electronic Arts\\EA Desktop", "LauncherAppPath"),
                ("Software\\Wow6432Node\\Electronic Arts\\EA Desktop", "ClientPath"),
            ],
            Client::Ubisoft => &[("Software\\Wow6432Node\\Ubisoft\\Launcher", "InstallDir")],
            Client::Rockstar => &[("Software\\Wow6432Node\\Rockstar Games\\Launcher", "InstallFolder")],
        }
    }

    /// The client's program in `named`, a folder or a program an install
    /// key names ([`install_keys`](Self::install_keys)): the first of the
    /// file names of [`exe_candidates`](Self::exe_candidates) there that
    /// `exists`.
    pub fn exe_in_folder(self, named: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
        let is_program = named.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
        let folder = if is_program { named.parent()? } else { named };
        self.exe_candidates().iter().filter_map(|rel| rel.rsplit('/').next()).map(|file| folder.join(file)).find(|p| exists(p))
    }

    /// The programs of the client and its helpers, as Wine and Windows name
    /// their processes (lowercase file names): they run beside the games and
    /// are never one of them.
    pub fn programs(self) -> &'static [&'static str] {
        match self {
            Client::BattleNet => &[
                "battle.net.exe",
                "battle.net launcher.exe",
                "battle.net helper.exe",
                "agent.exe",
                "blizzarderror.exe",
                "blizzard update agent.exe",
                "blizzard uninstaller.exe",
                "battle.net-setup.exe",
            ],
            Client::Ea => &[
                "ealauncher.exe",
                "eadesktop.exe",
                "ealaunchhelper.exe",
                "eadestager.exe",
                "eabackgroundservice.exe",
                "ealocalhostsvc.exe",
                "eacefsubprocess.exe",
                "eaconnect_microsoft.exe",
                "eacrashreporter.exe",
                "eaupdater.exe",
                "qtwebengineprocess.exe",
                "link2ea.exe",
                "eaappinstaller.exe",
                "eaappinstallerbootstrapper.exe",
            ],
            Client::Ubisoft => &[
                "upc.exe",
                "ubisoftconnect.exe",
                "uplaywebcore.exe",
                "uplaycrashreporter.exe",
                "uplayservice.exe",
                "ubisoftgamelauncher.exe",
                "ubisoftgamelauncher64.exe",
                "ubisoftextension.exe",
                "upcelevationservice.exe",
                "ubisoftconnectinstaller.exe",
            ],
            Client::Rockstar => &[
                "launcher.exe",
                "launcherpatcher.exe",
                "rockstarservice.exe",
                "rockstarsteamhelper.exe",
                "rockstarerrorhandler.exe",
                "socialclubhelper.exe",
                "rockstar-games-launcher.exe",
            ],
        }
    }

    /// The client's main program: while it runs, a game request goes to it.
    pub fn main_program(self) -> &'static str {
        match self {
            Client::BattleNet => "battle.net.exe",
            Client::Ea => "eadesktop.exe",
            Client::Ubisoft => "upc.exe",
            Client::Rockstar => "launcher.exe",
        }
    }
}

/// The C: drive of a prefix: `<prefix>/pfx/drive_c` for Proton's layout,
/// else `<prefix>/drive_c`.
pub fn drive_c(prefix: &Path) -> PathBuf {
    let proton = prefix.join("pfx").join("drive_c");
    if proton.is_dir() {
        proton
    } else {
        prefix.join("drive_c")
    }
}

/// The Wine prefix proper (where `system.reg` is): `<prefix>/pfx` for
/// Proton's layout, else `prefix`.
pub fn wine_prefix(prefix: &Path) -> PathBuf {
    let proton = prefix.join("pfx");
    if proton.join("system.reg").is_file() || proton.join("drive_c").is_dir() {
        proton
    } else {
        prefix.to_path_buf()
    }
}

/// Folders as the clients' paths name them (`Program Files/...`,
/// `AppData/Roaming/...`, parts separated by `/`): below `root`, but for
/// the folders Windows may keep elsewhere (`moved`: `Program Files` on
/// another drive, `Documents` in OneDrive).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folders {
    pub root: PathBuf,
    /// `(the path below root, parts separated by '/', where it is)`.
    pub moved: Vec<(String, PathBuf)>,
}

impl Folders {
    /// `rel` in the moved folder it starts in (any case), else below `root`.
    pub fn path(&self, rel: &str) -> PathBuf {
        let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
        for (under, dir) in &self.moved {
            let head: Vec<&str> = under.split('/').filter(|p| !p.is_empty()).collect();
            if parts.len() >= head.len() && parts.iter().zip(&head).all(|(a, b)| a.eq_ignore_ascii_case(b)) {
                return parts[head.len()..].iter().fold(dir.clone(), |p, part| p.join(part));
            }
        }
        parts.iter().fold(self.root.clone(), |p, part| p.join(part))
    }
}

/// This PC's C: drive as [`Client::exe_candidates`] and the clients' files
/// name it (Windows): `Program Files`, `Program Files (x86)` and
/// `ProgramData` where Windows keeps them (`%ProgramW6432%`,
/// `%ProgramFiles(x86)%`, `%ProgramData%`), the rest on the system drive
/// (`%SystemDrive%`). `var` reads the environment.
pub fn machine_drive(var: impl Fn(&str) -> Option<String>) -> Folders {
    let get = |names: &[&str], default: &str| {
        names.iter().filter_map(|n| var(n)).map(|v| v.trim().to_string()).find(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
    };
    let drive = get(&["SystemDrive"], "C:");
    let moved = [
        ("Program Files (x86)", get(&["ProgramFiles(x86)"], r"C:\Program Files (x86)")),
        // 64-bit programs' folder, whatever the process asking.
        ("Program Files", get(&["ProgramW6432", "ProgramFiles"], r"C:\Program Files")),
        ("ProgramData", get(&["ProgramData"], r"C:\ProgramData")),
    ];
    Folders {
        root: PathBuf::from(format!("{}\\", drive.trim_end_matches(['\\', '/']))),
        moved: moved.into_iter().map(|(rel, dir)| (rel.to_string(), PathBuf::from(dir))).collect(),
    }
}

/// This user's folder as the clients' files name it (`AppData/Roaming/...`,
/// `Documents/...`), with the folders Windows may keep elsewhere where they
/// are (a redirected `AppData`, `Documents` in OneDrive). Windows; `None`
/// without a home folder.
pub fn machine_user() -> Option<Folders> {
    let root = dirs::home_dir()?;
    let moved = [("AppData/Roaming", dirs::data_dir()), ("AppData/Local", dirs::data_local_dir()), ("Documents", dirs::document_dir())];
    Some(Folders { root, moved: moved.into_iter().filter_map(|(rel, dir)| Some((rel.to_string(), dir?))).collect() })
}

/// The prefix the client is set up in (`winePrefix` of its own settings
/// file); `None` while it was never set up.
pub fn prefix_of(paths: &DataPaths, client: Client, home: &str) -> Option<PathBuf> {
    let own = crate::game_config::game_overrides(paths, client.config_app());
    let prefix = own.get("winePrefix").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty())?;
    Some(PathBuf::from(crate::game_config::expand_tilde(prefix, home)))
}

/// When a client's games were last brought in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    /// RFC 3339.
    #[serde(default)]
    pub at: String,
    /// Games the client had installed.
    #[serde(default)]
    pub games: usize,
}

/// `local_library/clients.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientPrefs {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub last: BTreeMap<Client, LastSync>,
    /// Games of each client the user took out of the Collection (their app
    /// names): the sync leaves them out until they are installed again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub removed: BTreeMap<Client, BTreeSet<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl ClientPrefs {
    /// A missing or unreadable file: nothing synced yet.
    pub fn load(paths: &DataPaths) -> Self {
        json_store::read(&paths.clients()).unwrap_or_default()
    }

    pub fn save(&self, paths: &DataPaths) -> Result<()> {
        json_store::write(&paths.clients(), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_name_the_config_files() {
        for c in Client::ALL {
            assert_eq!(Client::from_id(c.id()), Some(c));
            assert_eq!(Client::from_config_app(c.config_app()), Some(c));
            assert!(c.programs().contains(&c.main_program()), "{c:?}");
            assert_eq!(serde_json::to_value(c).unwrap(), Value::String(c.id().into()));
        }
        assert_eq!(Client::from_id("lutris"), None);
    }

    #[test]
    fn finds_the_client_in_either_prefix_layout() {
        let dir = tempfile::tempdir().unwrap();
        let wine = dir.path().join("wine");
        let exe = wine.join("drive_c/Program Files (x86)/Battle.net/Battle.net.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "MZ").unwrap();
        assert_eq!(Client::BattleNet.exe_in(&wine), Some(exe));
        assert_eq!(wine_prefix(&wine), wine);

        let proton = dir.path().join("proton");
        let exe = proton.join("pfx/drive_c/Program Files/Rockstar Games/Launcher/Launcher.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "MZ").unwrap();
        assert_eq!(Client::Rockstar.exe_in(&proton), Some(exe));
        assert_eq!(wine_prefix(&proton), proton.join("pfx"));
        assert_eq!(Client::Ea.exe_in(&proton), None);
    }

    #[test]
    fn the_pcs_folders_are_where_windows_keeps_them() {
        let env = |k: &str| match k {
            "SystemDrive" => Some("D:".to_string()),
            "ProgramFiles(x86)" => Some(r"E:\Apps (x86)".to_string()),
            "ProgramW6432" => Some(r"E:\Apps".to_string()),
            // Only when there is no `ProgramW6432`.
            "ProgramFiles" => Some(r"E:\Other".to_string()),
            _ => None,
        };
        let drive = machine_drive(env);
        let at = |root: &str, parts: &[&str]| parts.iter().fold(PathBuf::from(root), |p, part| p.join(part));
        assert_eq!(drive.path("Program Files (x86)/Battle.net/Battle.net.exe"), at(r"E:\Apps (x86)", &["Battle.net", "Battle.net.exe"]));
        assert_eq!(drive.path("program files/EA Games"), at(r"E:\Apps", &["EA Games"]));
        // Not set: Windows' default.
        assert_eq!(drive.path("ProgramData/Battle.net/Agent/product.db"), at(r"C:\ProgramData", &["Battle.net", "Agent", "product.db"]));
        assert_eq!(drive.path("Games/EA"), at("D:\\", &["Games", "EA"]));
        assert_eq!(drive.path("Program Files Extra/x"), at("D:\\", &["Program Files Extra", "x"]));

        let defaults = machine_drive(|_| None);
        assert_eq!(defaults.root, PathBuf::from("C:\\"));
        assert_eq!(defaults.path("Program Files/Rockstar Games"), at(r"C:\Program Files", &["Rockstar Games"]));

        let user = Folders {
            root: PathBuf::from("/home/u"),
            moved: vec![("AppData/Roaming".into(), PathBuf::from("/roaming")), ("Documents".into(), PathBuf::from("/onedrive/docs"))],
        };
        assert_eq!(user.path("AppData/Roaming/Battle.net/Battle.net.config"), at("/roaming", &["Battle.net", "Battle.net.config"]));
        assert_eq!(user.path("Documents/Rockstar Games"), at("/onedrive/docs", &["Rockstar Games"]));
        assert_eq!(user.path("AppData/Local/Ubisoft Game Launcher"), at("/home/u", &["AppData", "Local", "Ubisoft Game Launcher"]));
    }

    #[test]
    fn an_install_key_names_the_folder_or_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Battle.net");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Battle.net Launcher.exe"), "MZ").unwrap();
        let exists = |p: &Path| p.is_file();
        assert_eq!(Client::BattleNet.exe_in_folder(&folder, exists), Some(folder.join("Battle.net Launcher.exe")));
        std::fs::write(folder.join("Battle.net.exe"), "MZ").unwrap();
        assert_eq!(Client::BattleNet.exe_in_folder(&folder, exists), Some(folder.join("Battle.net.exe")), "the first candidate wins");
        // EA's `ClientPath` names `EADesktop.exe`; its launcher comes first.
        let ea = dir.path().join("EA Desktop");
        std::fs::create_dir_all(&ea).unwrap();
        std::fs::write(ea.join("EADesktop.exe"), "MZ").unwrap();
        assert_eq!(Client::Ea.exe_in_folder(&ea.join("EADesktop.exe"), exists), Some(ea.join("EADesktop.exe")));
        std::fs::write(ea.join("EALauncher.exe"), "MZ").unwrap();
        assert_eq!(Client::Ea.exe_in_folder(&ea.join("EADesktop.EXE"), exists), Some(ea.join("EALauncher.exe")));
        assert_eq!(Client::Ubisoft.exe_in_folder(&folder, exists), None);
        for c in Client::ALL {
            assert!(!c.install_keys().is_empty(), "{c:?}");
        }
    }

    #[test]
    fn the_prefix_is_only_the_one_set_for_the_client() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(prefix_of(&paths, Client::Ubisoft, "/home/u"), None);
        crate::game_config::set_game_setting(&paths, Client::Ubisoft.config_app(), "/home/u", "winePrefix", "~/Prefixes/Ubisoft".into())
            .unwrap();
        assert_eq!(prefix_of(&paths, Client::Ubisoft, "/home/u"), Some(PathBuf::from("/home/u/Prefixes/Ubisoft")));
    }

    #[test]
    fn prefs_round_trip_and_keep_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(ClientPrefs::load(&paths), ClientPrefs::default());
        std::fs::create_dir_all(paths.clients().parent().unwrap()).unwrap();
        std::fs::write(paths.clients(), r#"{"removed":{"battlenet":["battlenet_wow"]},"future":1}"#).unwrap();
        let mut prefs = ClientPrefs::load(&paths);
        assert!(prefs.removed[&Client::BattleNet].contains("battlenet_wow"));
        prefs.last.insert(Client::Ea, LastSync { at: "2026-10-26T10:00:00Z".into(), games: 2 });
        prefs.save(&paths).unwrap();
        let again = ClientPrefs::load(&paths);
        assert_eq!(again.last[&Client::Ea].games, 2);
        assert_eq!(again.extra.get("future"), Some(&serde_json::json!(1)));
    }
}
