//! Ubisoft Connect: its games from the registry (`Ubisoft\Launcher\Installs`),
//! its sign-in from `ConnectSecureStorage.dat`.

use std::collections::HashSet;
use std::path::Path;

use crate::files::{last_part, machine_keys, unix_path, UNINSTALL};
use crate::pbuf::{Fields, Value};
use crate::winpath::join_ci;
use crate::{user_dirs, Account, ClientGame};

/// The key of each game the launcher installed, in the 32-bit view (where
/// the launcher writes it) and in the native one.
const INSTALLS: [&str; 2] = ["Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs", "Software\\Ubisoft\\Launcher\\Installs"];
const STORAGE: &str = "AppData/Local/Ubisoft Game Launcher/ConnectSecureStorage.dat";
const USER_DAT: &str = "AppData/Local/Ubisoft Game Launcher/user.dat";
/// In the secure storage while the account is remembered.
const REMEMBER_ME: &[u8] = b"RememberMeTicket";
/// The first byte of `uplay_install.state` once the game is installed.
const INSTALLED_STATE: u8 = 0x0A;

/// The games Ubisoft Connect has installed in `prefix`.
pub fn games(prefix: &Path) -> Vec<ClientGame> {
    let under: Vec<&str> = INSTALLS.iter().chain(&UNINSTALL).copied().collect();
    let keys = machine_keys(prefix, &under);
    // The title the launcher gave the game's Uninstall key (`Uplay Install 635`).
    let title_of = |id: &str| {
        let key = format!("Uplay Install {id}");
        keys.iter()
            .filter(|k| UNINSTALL.iter().any(|u| k.is_child_of(u)) && k.name().eq_ignore_ascii_case(&key))
            .find_map(|k| k.str("DisplayName").map(str::to_string))
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for key in keys.iter().filter(|k| INSTALLS.iter().any(|i| k.is_child_of(i))) {
        let id = key.name().trim().to_string();
        let Some(dir) = key.str("InstallDir") else { continue };
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let folder = unix_path(prefix, dir);
        if let Some(folder) = &folder {
            if !folder.is_dir() || still_downloading(folder) {
                continue;
            }
        }
        let title = title_of(&id).or_else(|| last_part(dir).map(str::to_string)).unwrap_or_else(|| id.clone());
        let launch = format!("uplay://launch/{id}/0");
        out.push(ClientGame { id, title, folder, launch });
    }
    out
}

/// Whether the game's `uplay_install.state` says it is not installed yet.
/// A game without the file, or with one that cannot be read, is.
fn still_downloading(folder: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(join_ci(folder, "uplay_install.state")) else { return false };
    let mut first = [0u8; 1];
    matches!(file.read(&mut first), Ok(1)) && first[0] != INSTALLED_STATE
}

/// Whether Ubisoft Connect is signed in.
pub fn account(prefix: &Path) -> Account {
    let mut out = Account::Unknown;
    for user in user_dirs(prefix) {
        let Ok(storage) = std::fs::read(join_ci(&user, STORAGE)) else { continue };
        if storage.windows(REMEMBER_ME.len()).any(|w| w == REMEMBER_ME) {
            let name = std::fs::read(join_ci(&user, USER_DAT)).ok().and_then(|dat| account_name(&dat));
            return Account::SignedIn(name);
        }
        out = Account::SignedOut;
    }
    out
}

/// The username (field 1 of `user.dat`), else the email (field 5), when the
/// file is a message and they read as a name.
fn account_name(dat: &[u8]) -> Option<String> {
    let (mut name, mut email) = (None, None);
    let mut fields = Fields::new(dat);
    for (number, value) in fields.by_ref() {
        match (number, value) {
            (1, Value::Bytes(_)) if name.is_none() => name = value.as_str().filter(|s| readable(s)).map(str::to_string),
            (5, Value::Bytes(_)) if email.is_none() => email = value.as_str().filter(|s| readable(s)).map(str::to_string),
            _ => {}
        }
    }
    if fields.is_malformed() {
        return None;
    }
    name.or(email)
}

/// A short text of printable characters, not a piece of binary.
fn readable(s: &str) -> bool {
    (1..=64).contains(&s.chars().count()) && !s.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbuf::tests::Writer;

    fn put(path: &Path, bytes: impl AsRef<[u8]>) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    const REG: &str = r#"WINE REGISTRY Version 2

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install 635] 1
"DisplayName"="Far Cry\x00ae 3"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install 5059] 1
"DisplayName"="Assassin's Creed Valhalla"

[Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\635] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Far Cry 3/"

[Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\5059] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Assassin's Creed Valhalla/"

[Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\7] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Downloading/"

[Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\8] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Plain Folder/"

[Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\9] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Gone/"

[Software\\Ubisoft\\Launcher\\Installs\\635] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Far Cry 3/"

[Software\\Wow6432Node\\Ubisoft\\Launcher] 1
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/"
"#;

    #[test]
    fn lists_the_installs_with_the_title_of_their_uninstall_key() {
        let dir = tempfile::tempdir().unwrap();
        let games_dir = dir.path().join("drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games");
        put(&games_dir.join("Far Cry 3/uplay_install.state"), [0x0A, 0x02]);
        std::fs::create_dir_all(games_dir.join("Assassin's Creed Valhalla")).unwrap();
        put(&games_dir.join("Downloading/uplay_install.state"), [0x08, 0x01]);
        std::fs::create_dir_all(games_dir.join("Plain Folder")).unwrap();
        put(&dir.path().join("system.reg"), REG);

        let games = games(dir.path());
        let got: Vec<_> = games.iter().map(|g| (g.id.as_str(), g.title.as_str(), g.launch.as_str())).collect();
        assert_eq!(
            got,
            [
                ("635", "Far Cry® 3", "uplay://launch/635/0"),
                ("5059", "Assassin's Creed Valhalla", "uplay://launch/5059/0"),
                ("8", "Plain Folder", "uplay://launch/8/0"),
            ]
        );
        assert_eq!(games[0].folder.as_deref(), Some(games_dir.join("Far Cry 3").as_path()));
    }

    #[test]
    fn no_registry_means_no_games() {
        assert!(games(tempfile::tempdir().unwrap().path()).is_empty());
    }

    fn prefix_with(storage: Option<&[u8]>, user_dat: Option<&[u8]>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("drive_c/users/steamuser");
        std::fs::create_dir_all(user.join("AppData/Local/Ubisoft Game Launcher")).unwrap();
        if let Some(bytes) = storage {
            put(&user.join(STORAGE), bytes);
        }
        if let Some(bytes) = user_dat {
            put(&user.join(USER_DAT), bytes);
        }
        dir
    }

    #[test]
    fn the_remember_me_ticket_means_signed_in() {
        let storage = [&[0u8, 1, 2][..], b"xx RememberMeTicket xx"].concat();
        let dat = Writer::default().text(1, "player_one").text(5, "me@example.com").0;
        assert_eq!(account(prefix_with(Some(&storage), Some(&dat)).path()), Account::SignedIn(Some("player_one".into())));
        // Without a username the email is the name; without a readable file, none.
        let email_only = Writer::default().num(2, 1).text(5, "me@example.com").0;
        assert_eq!(account(prefix_with(Some(&storage), Some(&email_only)).path()), Account::SignedIn(Some("me@example.com".into())));
        assert_eq!(account(prefix_with(Some(&storage), None).path()), Account::SignedIn(None));
        assert_eq!(account(prefix_with(Some(&storage), Some(&[0xFF, 0x00, 0x13, 0x99])).path()), Account::SignedIn(None));
        let binary_name = Writer::default().bytes(1, &[0x01, 0xFF, 0x00, 0x7F]).0;
        assert_eq!(account(prefix_with(Some(&storage), Some(&binary_name)).path()), Account::SignedIn(None));
    }

    #[test]
    fn storage_without_the_ticket_is_signed_out_and_none_is_unknown() {
        assert_eq!(account(prefix_with(Some(b"empty storage"), None).path()), Account::SignedOut);
        assert_eq!(account(prefix_with(Some(b""), None).path()), Account::SignedOut);
        assert_eq!(account(prefix_with(None, None).path()), Account::Unknown);
        assert_eq!(account(tempfile::tempdir().unwrap().path()), Account::Unknown);
    }
}
