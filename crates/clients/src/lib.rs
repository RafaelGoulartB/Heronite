//! The Windows store clients Heronite starts games through: Battle.net, the
//! EA app, Ubisoft Connect and the Rockstar Games Launcher (the clients
//! themselves are [`heronite_data::clients::Client`]).
//!
//! On Linux each client is installed into a Wine prefix of its own
//! ([`setup`]); on Windows it is installed on the PC as it is ([`native`]),
//! and one already there is used. The user signs in and installs games
//! inside the client. Heronite reads, without running anything, what the
//! client installed ([`games`]) and whether it is signed in ([`account`]),
//! brings those games into the Collection ([`collection`]) and opens or
//! closes the client ([`run`]). Its games are started through it by
//! `heronite_launch` (`launchKind` `client`).
//!
//! * [`place`]: where a client lives (a prefix, or this PC) and where its
//!   files are there.
//! * [`reg`]: Wine's registry files, read without Wine.
//! * [`winpath`]: the prefix's Windows paths as Linux paths.
//! * [`battlenet`], [`ea`], [`ubisoft`], [`rockstar`]: where each client
//!   keeps its games and its sign-in.

pub mod battlenet;
pub mod collection;
pub mod ea;
pub mod native;
pub mod place;
pub mod reg;
pub mod rockstar;
pub mod run;
pub mod setup;
pub mod ubisoft;
pub mod winpath;

mod files;
mod pbuf;
mod spawn;
mod xml;

use std::path::{Path, PathBuf};

pub use heronite_data::clients::Client;
use heronite_data::DataPaths;
use heronite_launch::host::{run_mode, Os, RunMode};
pub use place::Place;

/// Whether the client is signed in, as far as its files tell.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Account {
    /// Its files say nothing either way (never opened, or unreadable).
    #[default]
    Unknown,
    SignedOut,
    /// Signed in; the account's name or email when the client keeps it.
    SignedIn(Option<String>),
}

/// A game the client has installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientGame {
    /// The client's id for the game (Battle.net's product uid, Ubisoft's
    /// install id, EA's first content id, Rockstar's title id). With the
    /// client it names the Collection entry ([`collection::app_name`]).
    pub id: String,
    pub title: String,
    /// The game's folder (a Linux path in a prefix), when known.
    pub folder: Option<PathBuf>,
    /// What the client is asked to start it: the entry's `launcherArgs`
    /// (`--exec="launch WoW"`, `uplay://launch/635/0`).
    pub launch: String,
}

/// What Heronite knows about a client on this PC (a few file reads).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// The prefix the client is set up in; `None` while it never was, and
    /// on Windows, where it has none.
    pub prefix: Option<PathBuf>,
    /// The client's program, when it is installed (in the prefix, or on
    /// this PC).
    pub exe: Option<PathBuf>,
    pub account: Account,
    /// The games it has installed, by title.
    pub games: Vec<ClientGame>,
}

impl Status {
    pub fn installed(&self) -> bool {
        self.exe.is_some()
    }
}

/// Whether the clients run as they are on this system (Windows), not
/// under Wine: no prefix, no Wine settings, and a client installed on the
/// PC counts as set up.
pub fn native() -> bool {
    run_mode(Os::Windows) == RunMode::Native
}

/// Where `client` lives: this PC where the clients run as they are, else
/// the prefix it was set up in (`None` while it never was).
pub fn place_of(paths: &DataPaths, client: Client, home: &str) -> Option<Place> {
    place_for(paths, client, home, native())
}

fn place_for(paths: &DataPaths, client: Client, home: &str, native: bool) -> Option<Place> {
    if native {
        return Some(Place::machine());
    }
    heronite_data::clients::prefix_of(paths, client, home).map(Place::Prefix)
}

/// Looks at `client` on this PC: its prefix, program, sign-in and games.
/// Reads files (and on Windows the registry) only; never starts Wine.
pub fn status(paths: &DataPaths, client: Client, home: &str) -> Status {
    place_of(paths, client, home).map(|place| status_in(client, &place)).unwrap_or_default()
}

/// [`status`] of `client` in `place`.
pub fn status_in(client: Client, place: &Place) -> Status {
    let exe = find_exe(client, place);
    let (account, mut games) = match exe {
        Some(_) => (account(client, place), self::games(client, place)),
        None => (Account::Unknown, Vec::new()),
    };
    games.sort_by_key(|g| g.title.to_lowercase());
    Status { prefix: place.prefix().map(Path::to_path_buf), exe, account, games }
}

/// The client's program in `place` (`Client::exe_candidates`, plus where a
/// client that moves its program between versions keeps it). On this PC
/// also where the registry says the user installed it
/// (`Client::install_keys`).
pub fn find_exe(client: Client, place: &Place) -> Option<PathBuf> {
    let usual = match place {
        Place::Prefix(prefix) => client.exe_in(prefix),
        Place::Machine(_) => client.exe_candidates().iter().map(|rel| place.drive(rel)).find(|p| p.is_file()),
    };
    usual
        .or_else(|| match client {
            Client::Ea => ea::versioned_exe(place),
            _ => None,
        })
        .or_else(|| place.prefix().is_none().then(|| registered_exe(client, place)).flatten())
}

/// The client's program in the folder its installer wrote down.
fn registered_exe(client: Client, place: &Place) -> Option<PathBuf> {
    client.install_keys().iter().find_map(|(key, value)| {
        let keys = place.machine_keys(&[key]);
        let named = keys.iter().find(|k| k.path.eq_ignore_ascii_case(key))?.str(value)?;
        client.exe_in_folder(&place.local_path(named)?, |p| p.is_file())
    })
}

/// The games `client` has installed in `place`.
pub fn games(client: Client, place: &Place) -> Vec<ClientGame> {
    match client {
        Client::BattleNet => battlenet::games(place),
        Client::Ea => ea::games(place),
        Client::Ubisoft => ubisoft::games(place),
        Client::Rockstar => rockstar::games(place),
    }
}

/// Whether `client` is signed in, from its files in `place`.
pub fn account(client: Client, place: &Place) -> Account {
    match client {
        Client::BattleNet => battlenet::account(place),
        Client::Ea => ea::account(place),
        Client::Ubisoft => ubisoft::account(place),
        Client::Rockstar => rockstar::account(place),
    }
}

/// The folders of the prefix's Windows users (`drive_c/users/<name>`, not
/// `Public`): `steamuser` under Proton, the login name under Wine.
pub fn user_dirs(prefix: &Path) -> Vec<PathBuf> {
    let users = heronite_data::clients::drive_c(prefix).join("users");
    let Ok(rd) = std::fs::read_dir(&users) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().eq_ignore_ascii_case("public"))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REG: &str = r#"WINE REGISTRY Version 2

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install 635] 1
"DisplayName"="Far Cry 3"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install 720] 1
"DisplayName"="Anno 1800"

[Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\635] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Far Cry 3/"

[Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\720] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Anno 1800/"
"#;

    /// [`status`] under Wine, whatever this system is.
    fn status_under_wine(paths: &DataPaths, client: Client, home: &str) -> Status {
        place_for(paths, client, home, false).map(|place| status_in(client, &place)).unwrap_or_default()
    }

    #[test]
    fn a_client_that_was_never_set_up_has_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let status = status_under_wine(&DataPaths::at(dir.path()), Client::Ubisoft, "/home/u");
        assert_eq!(status, Status::default());
        assert!(!status.installed());
    }

    #[test]
    fn natively_the_client_is_this_pcs_with_no_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(matches!(place_for(&paths, Client::Ea, "/home/u", true), Some(Place::Machine(_))));
        assert_eq!(place_for(&paths, Client::Ea, "/home/u", false), None);
        assert_eq!(native(), cfg!(windows));
    }

    #[test]
    fn follows_the_settings_to_the_prefix_the_program_and_the_games() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let prefix = dir.path().join("prefixes/Ubisoft Connect");
        heronite_data::game_config::set_game_setting(&paths, Client::Ubisoft.config_app(), "/home/u", "winePrefix", json!(prefix)).unwrap();

        // Set up, but the client is not installed yet.
        let status_now = || status_under_wine(&paths, Client::Ubisoft, "/home/u");
        assert_eq!(status_now(), Status { prefix: Some(prefix.clone()), ..Default::default() });

        // Proton's layout: the Wine prefix is in `pfx`.
        let wine = prefix.join("pfx");
        let drive = wine.join("drive_c");
        let games = drive.join("Program Files (x86)/Ubisoft/Ubisoft Game Launcher");
        std::fs::create_dir_all(games.join("games/Far Cry 3")).unwrap();
        std::fs::create_dir_all(games.join("games/Anno 1800")).unwrap();
        std::fs::write(games.join("upc.exe"), "MZ").unwrap();
        std::fs::write(wine.join("system.reg"), REG).unwrap();
        let storage = drive.join("users/steamuser/AppData/Local/Ubisoft Game Launcher");
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(storage.join("ConnectSecureStorage.dat"), b"..RememberMeTicket..").unwrap();

        let status = status_now();
        assert!(status.installed());
        assert_eq!(status.exe, Some(games.join("upc.exe")));
        assert_eq!(status.account, Account::SignedIn(None));
        // By title, whatever the registry's order.
        let listed: Vec<_> = status.games.iter().map(|g| (g.id.as_str(), g.title.as_str())).collect();
        assert_eq!(listed, [("720", "Anno 1800"), ("635", "Far Cry 3")]);
        assert_eq!(status.games[1].folder, Some(games.join("games/Far Cry 3")));
    }

    #[test]
    fn the_ea_app_in_a_versioned_folder_is_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("drive_c/Program Files/Electronic Arts/EA Desktop/13.1.0.1/EA Desktop/EALauncher.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "MZ").unwrap();
        assert_eq!(find_exe(Client::Ea, &dir.path().into()), Some(exe));
        assert_eq!(find_exe(Client::Ubisoft, &dir.path().into()), None);
    }

    /// Ubisoft Connect on a PC: its program where the installer puts it,
    /// its games in the registry, its sign-in in the user's folder.
    #[cfg(windows)]
    #[test]
    fn a_client_on_this_pc_is_read_like_one_in_a_prefix() {
        use heronite_platform::registry::{delete_tree, set_string, Hive};
        let base = format!(r"Software\HeroniteClientsTest\{}-status", std::process::id());
        let dir = tempfile::tempdir().unwrap();
        let (pc, drive, user) = place::tests::machine_in(dir.path(), &base);
        assert_eq!(status_in(Client::Ubisoft, &pc), Status::default(), "not installed");

        let launcher = drive.join("Program_Files_(x86)/Ubisoft/Ubisoft Game Launcher");
        std::fs::create_dir_all(launcher.join("games/Far Cry 3")).unwrap();
        std::fs::write(launcher.join("upc.exe"), "MZ").unwrap();
        let installs = format!(r"{base}\Software\Wow6432Node\Ubisoft\Launcher\Installs\635");
        let folder = launcher.join("games/Far Cry 3");
        set_string(Hive::CurrentUser, &installs, "InstallDir", &format!("{}/", folder.display())).unwrap();
        let uninstall = format!(r"{base}\Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Uplay Install 635");
        set_string(Hive::CurrentUser, &uninstall, "DisplayName", "Far Cry® 3").unwrap();
        let storage = user.join("AppData/Local/Ubisoft Game Launcher");
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(storage.join("ConnectSecureStorage.dat"), b"..RememberMeTicket..").unwrap();

        let status = status_in(Client::Ubisoft, &pc);
        delete_tree(Hive::CurrentUser, &base).unwrap();
        assert_eq!(status.prefix, None);
        assert_eq!(status.exe, Some(launcher.join("upc.exe")));
        assert_eq!(status.account, Account::SignedIn(None));
        let listed: Vec<_> = status.games.iter().map(|g| (g.id.as_str(), g.title.as_str(), g.launch.as_str())).collect();
        assert_eq!(listed, [("635", "Far Cry® 3", "uplay://launch/635/0")]);
        assert_eq!(status.games[0].folder, Some(folder));
    }

    /// A client the user installed on another drive is found by what its
    /// installer wrote in the registry.
    #[cfg(windows)]
    #[test]
    fn a_client_elsewhere_is_found_through_the_registry() {
        use heronite_platform::registry::{delete_tree, set_string, Hive};
        let base = format!(r"Software\HeroniteClientsTest\{}-exe", std::process::id());
        let dir = tempfile::tempdir().unwrap();
        let (pc, ..) = place::tests::machine_in(dir.path(), &base);
        let elsewhere = dir.path().join("D/Games/Rockstar Launcher");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("Launcher.exe"), "MZ").unwrap();
        assert_eq!(find_exe(Client::Rockstar, &pc), None);
        let key = format!(r"{base}\Software\Wow6432Node\Rockstar Games\Launcher");
        set_string(Hive::CurrentUser, &key, "InstallFolder", &elsewhere.to_string_lossy()).unwrap();
        let found = find_exe(Client::Rockstar, &pc);
        delete_tree(Hive::CurrentUser, &base).unwrap();
        assert_eq!(found, Some(elsewhere.join("Launcher.exe")));
    }

    #[test]
    fn user_folders_leave_out_the_shared_one() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["steamuser", "Public", "public"] {
            std::fs::create_dir_all(dir.path().join("drive_c/users").join(name)).unwrap();
        }
        std::fs::write(dir.path().join("drive_c/users/file"), "x").unwrap();
        assert_eq!(user_dirs(dir.path()), [dir.path().join("drive_c/users/steamuser")]);
    }
}
