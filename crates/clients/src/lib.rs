//! The Windows store clients Heronite runs under Wine: Battle.net, the EA
//! app, Ubisoft Connect and the Rockstar Games Launcher (the clients
//! themselves are [`heronite_data::clients::Client`]).
//!
//! Each client lives in a Wine prefix of its own; the user signs in and
//! installs games inside it. Heronite reads, without running anything, what
//! the client installed ([`games`]) and whether it is signed in
//! ([`account`]) and brings those games into the Collection
//! ([`collection`]). Its games are started through it by `heronite_launch`
//! (`launchKind` `client`).
//!
//! * [`reg`]: Wine's registry files, read without Wine.
//! * [`winpath`]: the prefix's Windows paths as Linux paths.
//! * [`battlenet`], [`ea`], [`ubisoft`], [`rockstar`]: where each client
//!   keeps its games and its sign-in.

pub mod battlenet;
pub mod collection;
pub mod ea;
pub mod reg;
pub mod rockstar;
pub mod ubisoft;
pub mod winpath;

mod files;
mod pbuf;
mod xml;

use std::path::{Path, PathBuf};

pub use heronite_data::clients::Client;
use heronite_data::DataPaths;

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
    /// The game's folder (a Linux path), when known.
    pub folder: Option<PathBuf>,
    /// What the client is asked to start it: the entry's `launcherArgs`
    /// (`--exec="launch WoW"`, `uplay://launch/635/0`).
    pub launch: String,
}

/// What Heronite knows about a client on this PC (a few file reads).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// The prefix the client is set up in; `None` while it never was.
    pub prefix: Option<PathBuf>,
    /// The client's program, when it is installed in the prefix.
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

/// Looks at `client` on this PC: its prefix, program, sign-in and games.
/// Reads files only; never starts Wine.
pub fn status(paths: &DataPaths, client: Client, home: &str) -> Status {
    let Some(prefix) = heronite_data::clients::prefix_of(paths, client, home) else { return Status::default() };
    let exe = find_exe(client, &prefix);
    let (account, mut games) = match exe {
        Some(_) => (account(client, &prefix), self::games(client, &prefix)),
        None => (Account::Unknown, Vec::new()),
    };
    games.sort_by_key(|g| g.title.to_lowercase());
    Status { prefix: Some(prefix), exe, account, games }
}

/// The client's program in `prefix` (`Client::exe_candidates`, plus where
/// a client that moves its program between versions keeps it).
pub fn find_exe(client: Client, prefix: &Path) -> Option<PathBuf> {
    client.exe_in(prefix).or_else(|| match client {
        Client::Ea => ea::versioned_exe(prefix),
        _ => None,
    })
}

/// The games `client` has installed in `prefix`.
pub fn games(client: Client, prefix: &Path) -> Vec<ClientGame> {
    match client {
        Client::BattleNet => battlenet::games(prefix),
        Client::Ea => ea::games(prefix),
        Client::Ubisoft => ubisoft::games(prefix),
        Client::Rockstar => rockstar::games(prefix),
    }
}

/// Whether `client` is signed in, from its files in `prefix`.
pub fn account(client: Client, prefix: &Path) -> Account {
    match client {
        Client::BattleNet => battlenet::account(prefix),
        Client::Ea => ea::account(prefix),
        Client::Ubisoft => ubisoft::account(prefix),
        Client::Rockstar => rockstar::account(prefix),
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

    #[test]
    fn a_client_that_was_never_set_up_has_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let status = status(&DataPaths::at(dir.path()), Client::Ubisoft, "/home/u");
        assert_eq!(status, Status::default());
        assert!(!status.installed());
    }

    #[test]
    fn follows_the_settings_to_the_prefix_the_program_and_the_games() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let prefix = dir.path().join("prefixes/Ubisoft Connect");
        heronite_data::game_config::set_game_setting(&paths, Client::Ubisoft.config_app(), "/home/u", "winePrefix", json!(prefix)).unwrap();

        // Set up, but the client is not installed yet.
        let status_now = || status(&paths, Client::Ubisoft, "/home/u");
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
        assert_eq!(find_exe(Client::Ea, dir.path()), Some(exe));
        assert_eq!(find_exe(Client::Ubisoft, dir.path()), None);
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
