//! The Rockstar Games Launcher: its games from the registry, its sign-in
//! from the Social Club profiles.

use std::collections::HashSet;
use std::path::Path;

use crate::files::{last_part, machine_keys, subfolders, unix_path, UNINSTALL};
use crate::reg::RegKey;
use crate::winpath::join_ci;
use crate::{user_dirs, Account, ClientGame};

/// Where the games the launcher knows keep their folder, in the 32-bit view
/// and in the native one.
const GAMES: [&str; 2] = ["Software\\Wow6432Node\\Rockstar Games", "Software\\Rockstar Games"];
/// Keys of `Rockstar Games` that are not games.
const NOT_GAMES: [&str; 3] = ["launcher", "social club", "rockstar games social club"];
const SOCIAL_CLUB: &str = "Documents/Rockstar Games/Social Club";

/// The launcher's games by title id: `(id, registry key, title)`.
const KNOWN: &[(&str, &str, &str)] = &[
    ("gta5", "Grand Theft Auto V", "Grand Theft Auto V"),
    ("rdr2", "Red Dead Redemption 2", "Red Dead Redemption 2"),
    ("gta4", "Grand Theft Auto IV", "Grand Theft Auto IV"),
    ("lanoire", "LANoireMTL", "L.A. Noire"),
    ("mp3", "MP3MTL", "Max Payne 3"),
    ("gtasa", "GTA: San Andreas", "Grand Theft Auto: San Andreas"),
    ("gta3", "Grand Theft Auto III", "Grand Theft Auto III"),
    ("gtavc", "Grand Theft Auto: Vice City", "Grand Theft Auto: Vice City"),
    ("bully", "Bully", "Bully: Scholarship Edition"),
    ("gta3unreal", "GTA III - Definitive Edition", "GTA III \u{2013} Definitive Edition"),
    ("gtavcunreal", "GTA Vice City - Definitive Edition", "GTA Vice City \u{2013} Definitive Edition"),
    ("gtasaunreal", "GTA San Andreas - Definitive Edition", "GTA San Andreas \u{2013} Definitive Edition"),
];

/// A game as the registry has it, with its folder as a Windows path.
struct Found {
    id: String,
    title: Option<String>,
    windows_path: String,
}

/// The games the Rockstar Games Launcher has installed in `prefix`.
pub fn games(prefix: &Path) -> Vec<ClientGame> {
    let under: Vec<&str> = GAMES.iter().chain(&UNINSTALL).copied().collect();
    let keys = machine_keys(prefix, &under);
    // The launcher's own record of each game first: it has the title id.
    let found = from_uninstall(&keys).into_iter().chain(from_game_keys(&keys));
    let mut folders = HashSet::new();
    let mut ids = HashSet::new();
    found
        .filter(|f| folders.insert(windows_key(&f.windows_path)) && ids.insert(f.id.to_lowercase()))
        .filter_map(|f| {
            let folder = unix_path(prefix, &f.windows_path);
            // A key the game's uninstaller left behind.
            if folder.as_ref().is_some_and(|d| !d.is_dir()) {
                return None;
            }
            let known = KNOWN.iter().find(|(id, ..)| id.eq_ignore_ascii_case(&f.id)).map(|(.., title)| title.to_string());
            let title = f.title.or(known).or_else(|| last_part(&f.windows_path).map(str::to_string)).unwrap_or_else(|| f.id.clone());
            let launch = format!("-launchTitleInFolder \"{}\"", f.windows_path);
            Some(ClientGame { id: f.id, title, folder, launch })
        })
        .collect()
}

/// A folder as a comparable key: no case, one kind of separator, no
/// trailing one.
fn windows_key(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// A Windows folder for the launch argument: backslashes and no trailing
/// one, which would escape the closing quote.
fn windows_folder(path: &str) -> String {
    path.trim().replace('/', "\\").trim_end_matches('\\').to_string()
}

/// Games whose Uninstall key runs the launcher's uninstaller for a title
/// (`... Launcher.exe" -uninstall=gta5`).
fn from_uninstall(keys: &[RegKey]) -> Vec<Found> {
    keys.iter()
        .filter(|k| UNINSTALL.iter().any(|u| k.is_child_of(u)))
        .filter_map(|k| {
            let id = uninstall_title_id(k.str("UninstallString")?)?;
            let path = windows_folder(k.str("InstallLocation")?);
            (!path.is_empty()).then(|| Found { id, title: k.str("DisplayName").map(str::to_string), windows_path: path })
        })
        .collect()
}

/// `(launcher|uninstall)\.exe.*-uninstall=([^\s"]+)`, case-insensitive: the
/// text after the last `-uninstall=`.
fn uninstall_title_id(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    let after_exe = ["launcher.exe", "uninstall.exe"].iter().filter_map(|exe| lower.find(exe).map(|i| i + exe.len())).min()?;
    let mut end = lower.len();
    while let Some(at) = lower[after_exe..end].rfind("-uninstall=").map(|i| i + after_exe) {
        let value = &command[at + "-uninstall=".len()..];
        let id: String = value.chars().take_while(|c| !c.is_whitespace() && *c != '"').collect();
        if !id.is_empty() {
            return Some(id);
        }
        end = at;
    }
    None
}

/// Games under `Rockstar Games\<game>` that name their `InstallFolder`.
fn from_game_keys(keys: &[RegKey]) -> Vec<Found> {
    keys.iter()
        .filter(|k| GAMES.iter().any(|g| k.is_child_of(g)))
        .filter(|k| !NOT_GAMES.iter().any(|n| k.name().eq_ignore_ascii_case(n)))
        .filter_map(|k| {
            let path = windows_folder(k.str("InstallFolder")?);
            let known = KNOWN.iter().find(|(_, key, _)| key.eq_ignore_ascii_case(k.name()));
            let (id, title) = match known {
                Some((id, _, title)) => (id.to_string(), Some(title.to_string())),
                None => (k.name().to_string(), None),
            };
            (!path.is_empty()).then_some(Found { id, title, windows_path: path })
        })
        .collect()
}

/// Whether the Rockstar Games Launcher is signed in.
pub fn account(prefix: &Path) -> Account {
    let mut out = Account::Unknown;
    for user in user_dirs(prefix) {
        let club = join_ci(&user, SOCIAL_CLUB);
        if !club.is_dir() {
            continue;
        }
        if !subfolders(&join_ci(&club, "Profiles")).is_empty() {
            return Account::SignedIn(None);
        }
        out = Account::SignedOut;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: &str = r#"WINE REGISTRY Version 2

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Rockstar Games Launcher] 1
"DisplayName"="Rockstar Games Launcher"
"InstallLocation"="C:\\Program Files\\Rockstar Games\\Launcher"
"UninstallString"="\"C:\\Program Files\\Rockstar Games\\Launcher\\uninstall.exe\""

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Red Dead Redemption 2] 1
"DisplayName"="Red Dead Redemption 2 (Registry)"
"InstallLocation"="C:\\Program Files\\Rockstar Games\\Red Dead Redemption 2\\"
"UninstallString"="\"C:\\Program Files\\Rockstar Games\\Launcher\\Launcher.exe\" -uninstall=rdr2 -silent"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Casino] 1
"DisplayName"="Casino Night"
"InstallLocation"="C:/Games/Casino Night"
"UninstallString"="C:\\Games\\Casino Night\\UNINSTALL.EXE -UNINSTALL=casino"

[Software\\Wow6432Node\\Rockstar Games\\Grand Theft Auto V] 1
"InstallFolder"="C:\\Program Files\\Rockstar Games\\Grand Theft Auto V"

[Software\\Wow6432Node\\Rockstar Games\\Red Dead Redemption 2] 1
"InstallFolder"="C:\\Program Files\\Rockstar Games\\Red Dead Redemption 2"

[Software\\Wow6432Node\\Rockstar Games\\LANoireMTL] 1
"InstallFolder"="C:\\Program Files\\Rockstar Games\\L.A. Noire"

[Software\\Wow6432Node\\Rockstar Games\\Launcher] 1
"InstallFolder"="C:\\Program Files\\Rockstar Games\\Launcher"

[Software\\Wow6432Node\\Rockstar Games\\Social Club] 1
"InstallFolder"="C:\\Program Files\\Rockstar Games\\Social Club"

[Software\\Wow6432Node\\Rockstar Games\\Some Other Game] 1
"InstallFolder"="C:\\Program Files\\Rockstar Games\\Some Other Game"

[Software\\Wow6432Node\\Rockstar Games\\Bully] 1
"InstallFolder"="C:\\Program Files\\Rockstar Games\\Gone"
"#;

    fn prefix_with_games(reg: &str, folders: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for f in folders {
            std::fs::create_dir_all(dir.path().join("drive_c").join(f)).unwrap();
        }
        std::fs::write(dir.path().join("system.reg"), reg).unwrap();
        dir
    }

    #[test]
    fn reads_the_uninstall_command_and_the_known_game_keys() {
        let dir = prefix_with_games(
            REG,
            &[
                "Program Files/Rockstar Games/Red Dead Redemption 2",
                "Program Files/Rockstar Games/Grand Theft Auto V",
                "Program Files/Rockstar Games/L.A. Noire",
                "Program Files/Rockstar Games/Some Other Game",
                "Program Files/Rockstar Games/Launcher",
                "Games/Casino Night",
            ],
        );
        let games = games(dir.path());
        let got: Vec<_> = games.iter().map(|g| (g.id.as_str(), g.title.as_str(), g.launch.as_str())).collect();
        assert_eq!(
            got,
            [
                // The Uninstall key comes first and names the game its way.
                (
                    "rdr2",
                    "Red Dead Redemption 2 (Registry)",
                    r#"-launchTitleInFolder "C:\Program Files\Rockstar Games\Red Dead Redemption 2""#
                ),
                ("casino", "Casino Night", r#"-launchTitleInFolder "C:\Games\Casino Night""#),
                ("gta5", "Grand Theft Auto V", r#"-launchTitleInFolder "C:\Program Files\Rockstar Games\Grand Theft Auto V""#),
                ("lanoire", "L.A. Noire", r#"-launchTitleInFolder "C:\Program Files\Rockstar Games\L.A. Noire""#),
                ("Some Other Game", "Some Other Game", r#"-launchTitleInFolder "C:\Program Files\Rockstar Games\Some Other Game""#),
            ]
        );
        // The launch argument stays one piece for the launch side.
        assert_eq!(
            shlex::split(&games[0].launch).unwrap(),
            ["-launchTitleInFolder", r"C:\Program Files\Rockstar Games\Red Dead Redemption 2"]
        );
        assert_eq!(games[1].folder.as_deref(), Some(dir.path().join("drive_c/Games/Casino Night").as_path()));
    }

    #[test]
    fn matches_the_uninstall_command_like_the_launchers_own_pattern() {
        let id = uninstall_title_id;
        assert_eq!(id(r#""C:\R\Launcher\launcher.exe" -uninstall=gta5"#).as_deref(), Some("gta5"));
        assert_eq!(id(r#"C:\R\Uninstall.exe -uninstall="rdr2""#), None);
        assert_eq!(id(r#"C:\R\Uninstall.exe -uninstall=rdr2" other"#).as_deref(), Some("rdr2"));
        assert_eq!(id(r#"C:\R\UNINSTALL.EXE -Uninstall=a -uninstall=b c"#).as_deref(), Some("b"));
        assert_eq!(id(r#"C:\R\setup.exe -uninstall=gta5"#), None);
        assert_eq!(id(r#"-uninstall=gta5 C:\R\launcher.exe"#), None);
        assert_eq!(id(r#"C:\R\launcher.exe -uninstall="#), None);
        assert_eq!(id(""), None);
    }

    fn prefix_with_profiles(club: bool, profile: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("drive_c/users/steamuser").join(SOCIAL_CLUB);
        std::fs::create_dir_all(dir.path().join("drive_c/users/steamuser")).unwrap();
        if club {
            std::fs::create_dir_all(root.join("Profiles")).unwrap();
        }
        if profile {
            std::fs::create_dir_all(root.join("Profiles/0x1234")).unwrap();
        }
        dir
    }

    #[test]
    fn a_social_club_profile_means_signed_in() {
        assert_eq!(account(prefix_with_profiles(true, true).path()), Account::SignedIn(None));
        assert_eq!(account(prefix_with_profiles(true, false).path()), Account::SignedOut);
        assert_eq!(account(prefix_with_profiles(false, false).path()), Account::Unknown);
        assert_eq!(account(tempfile::tempdir().unwrap().path()), Account::Unknown);
    }
}
