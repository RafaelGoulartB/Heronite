//! The EA app: its games from their `__Installer/installerdata.xml`, its
//! sign-in from `user_<id>.ini`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use heronite_data::clients::drive_c;

use crate::files::{machine_keys, read_text, subfolders, unix_path, UNINSTALL};
use crate::winpath::join_ci;
use crate::xml::{elements, Element};
use crate::{user_dirs, Account, ClientGame};

const EA_DESKTOP: &str = "Program Files/Electronic Arts/EA Desktop";
/// The programs of the app, in the order it is started by.
const PROGRAMS: [&str; 2] = ["EALauncher.exe", "EADesktop.exe"];
const REGISTRY: [&str; 2] = ["Software\\Electronic Arts\\EA Desktop", "Software\\Wow6432Node\\Electronic Arts\\EA Desktop"];
/// Where the app keeps its settings, below the user's folder.
const USER_SETTINGS: &str = "AppData/Local/Electronic Arts/EA Desktop";

/// The EA app's program when a fresh install left it in a versioned folder
/// (`EA Desktop/<version>/EA Desktop/EALauncher.exe`).
pub fn versioned_exe(prefix: &Path) -> Option<PathBuf> {
    let root = join_ci(&drive_c(prefix), EA_DESKTOP);
    let program_in = |dir: &Path| PROGRAMS.iter().map(|p| join_ci(dir, p)).find(|p| p.is_file());
    subfolders(&root)
        .into_iter()
        .filter_map(|dir| {
            let key = version_key(&dir.file_name()?.to_string_lossy());
            Some((key, program_in(&join_ci(&dir, "EA Desktop"))?))
        })
        .max()
        .map(|(_, exe)| exe)
        .or_else(|| registered_exe(prefix).and_then(|dir| program_in(&dir)))
}

/// The numbers of a folder name (`12.0.4.5` is `[12, 0, 4, 5]`), to order
/// versions.
fn version_key(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit()).filter(|n| !n.is_empty()).filter_map(|n| n.parse().ok()).collect()
}

/// The folder the registry says the app is in (`LauncherAppPath` and
/// `ClientPath` name its program or its folder).
fn registered_exe(prefix: &Path) -> Option<PathBuf> {
    let keys = machine_keys(prefix, &REGISTRY);
    keys.iter().flat_map(|k| ["LauncherAppPath", "ClientPath"].into_iter().filter_map(move |v| k.str(v))).find_map(|win| {
        let path = unix_path(prefix, win)?;
        let is_file = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
        let dir = if is_file { path.parent()?.to_path_buf() } else { path };
        dir.is_dir().then_some(dir)
    })
}

/// A folder that may hold a game, and what the Uninstall key of the game
/// called it.
struct Candidate {
    folder: PathBuf,
    display_name: Option<String>,
}

/// The games the EA app has installed in `prefix`.
pub fn games(prefix: &Path) -> Vec<ClientGame> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut found: Vec<Candidate> = Vec::new();
    let mut add = |folder: PathBuf, display_name: Option<String>| {
        let key = folder.to_string_lossy().to_lowercase();
        match seen.get(&key) {
            Some(i) => {
                let known = &mut found[*i];
                known.display_name = known.display_name.take().or(display_name);
            }
            None => {
                seen.insert(key, found.len());
                found.push(Candidate { folder, display_name });
            }
        }
    };
    for folder in subfolders(&join_ci(&drive_c(prefix), "Program Files/EA Games")) {
        add(folder, None);
    }
    for root in library_roots(prefix) {
        for folder in subfolders(&root) {
            add(folder, None);
        }
    }
    for (folder, name) in installed_by_ea(prefix) {
        add(folder, name);
    }
    found.into_iter().filter_map(|c| game(&c)).collect()
}

/// The games the Uninstall keys say the EA app's installer put in
/// `InstallLocation`.
fn installed_by_ea(prefix: &Path) -> Vec<(PathBuf, Option<String>)> {
    machine_keys(prefix, &UNINSTALL)
        .iter()
        .filter(|k| k.str("UninstallString").is_some_and(|s| s.to_ascii_lowercase().contains("eainstaller")))
        .filter_map(|k| Some((unix_path(prefix, k.str("InstallLocation")?)?, k.str("DisplayName").map(str::to_string))))
        .collect()
}

/// The folders games are installed to besides `Program Files/EA Games`
/// (`downloadinplacedir` of the machine's and the users' settings).
fn library_roots(prefix: &Path) -> Vec<PathBuf> {
    let mut inis = vec![join_ci(&drive_c(prefix), "ProgramData/EA Desktop/machine.ini")];
    for user in user_dirs(prefix) {
        inis.extend(user_inis(&user));
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for text in inis.iter().filter_map(|ini| read_text(ini)) {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim().to_ascii_lowercase();
            if key == "downloadinplacedir" || key.ends_with(".downloadinplacedir") {
                if let Some(root) = unix_path(prefix, value).filter(|r| !roots.contains(r)) {
                    roots.push(root);
                }
            }
        }
    }
    roots
}

/// `user_<id>.ini` files of a user's EA app settings.
fn user_inis(user: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(join_ci(user, USER_SETTINGS)) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("user_") && name.ends_with(".ini")
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// The game in `candidate`'s folder, when the EA app installed one there.
fn game(candidate: &Candidate) -> Option<ClientGame> {
    let data = join_ci(&candidate.folder, "__Installer/installerdata.xml");
    let installer = parse_installer(&read_text(&data)?);
    let id = installer.content_ids.first()?.clone();
    let title = installer
        .title
        .or_else(|| candidate.display_name.clone())
        .or_else(|| candidate.folder.file_name().map(|n| n.to_string_lossy().into_owned()))?;
    // One argument for the launch side: the link has `&` and `?` in it.
    let launch = format!("\"origin2://game/launch?offerIds={}&autoDownload=1\"", installer.content_ids.join(","));
    Some(ClientGame { id, title, folder: Some(candidate.folder.clone()), launch })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Installer {
    /// The base game's first, then its extra content's.
    content_ids: Vec<String>,
    title: Option<String>,
}

fn parse_installer(xml: &str) -> Installer {
    let mut content_ids: Vec<String> = Vec::new();
    for id in elements(xml, "contentID").iter().filter_map(Element::text) {
        // The id goes into a link inside quotes.
        if !id.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '&' | ',')) && !content_ids.contains(&id) {
            content_ids.push(id);
        }
    }
    Installer { content_ids, title: title_of(xml) }
}

/// `<gameTitle locale="en_US">`, or the first `<gameTitle>`, or (newer
/// files) the `<title>` of `<localeInfo locale="en_US">`, or of the first.
fn title_of(xml: &str) -> Option<String> {
    let english = |e: &Element| e.attr("locale").is_some_and(|l| l.replace('-', "_").eq_ignore_ascii_case("en_US"));
    let game_titles = elements(xml, "gameTitle");
    if let Some(t) = game_titles.iter().find(|e| english(e)).and_then(Element::text).or_else(|| game_titles.iter().find_map(Element::text))
    {
        return Some(t);
    }
    let locales = elements(xml, "localeInfo");
    let title = |e: &Element| elements(e.inner, "title").iter().find_map(Element::text);
    locales.iter().filter(|e| english(e)).find_map(title).or_else(|| locales.iter().find_map(title))
}

/// Whether the EA app is signed in.
pub fn account(prefix: &Path) -> Account {
    let mut out = Account::Unknown;
    for user in user_dirs(prefix) {
        if !join_ci(&user, USER_SETTINGS).is_dir() {
            continue;
        }
        out = Account::SignedOut;
        let signed_in = user_inis(&user).iter().filter_map(|ini| read_text(ini)).any(|text| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .any(|(k, v)| k.trim().eq_ignore_ascii_case("user.userid") && !v.trim().is_empty())
        });
        if signed_in {
            return Account::SignedIn(None);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<DiPManifest version="4.0">
  <gameTitles><gameTitle locale="de_DE">Spiel</gameTitle><gameTitle locale="en_US">Mass Effect&#8482; Andromeda</gameTitle></gameTitles>
  <contentIDs><contentID>71067</contentID><contentID>71068</contentID></contentIDs>
</DiPManifest>"#;

    const NEW: &str = r#"<game><metadata>
  <localeInfo locale="fr_FR"><title>Jeu</title></localeInfo>
  <localeInfo locale="en_US"><title>It Takes Two</title></localeInfo></metadata>
  <contentIDs><contentID>1026481</contentID></contentIDs></game>"#;

    fn utf16(text: &str) -> Vec<u8> {
        [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect()
    }

    fn put(path: &Path, bytes: impl AsRef<[u8]>) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn game_folder(drive: &Path, rel: &str, manifest: impl AsRef<[u8]>) -> PathBuf {
        let folder = drive.join(rel);
        put(&folder.join("__Installer/installerdata.xml"), manifest);
        folder
    }

    #[test]
    fn reads_both_schemas_and_the_content_ids() {
        let old = parse_installer(OLD);
        assert_eq!(old.content_ids, ["71067", "71068"]);
        assert_eq!(old.title.as_deref(), Some("Mass Effect™ Andromeda"));
        let new = parse_installer(NEW);
        assert_eq!((new.content_ids.as_slice(), new.title.as_deref()), (&["1026481".to_string()][..], Some("It Takes Two")));
        let other_language =
            parse_installer("<gameTitles><gameTitle locale=\"de_DE\">Spiel</gameTitle></gameTitles><contentID>1</contentID>");
        assert_eq!(other_language.title.as_deref(), Some("Spiel"));
        assert_eq!(parse_installer("<nothing/>"), Installer::default());
    }

    #[test]
    fn lists_games_from_every_place_the_app_puts_them() {
        let dir = tempfile::tempdir().unwrap();
        let drive = dir.path().join("drive_c");
        let user = drive.join("users/steamuser");
        // The default library (UTF-8), a custom one named by the user's ini
        // (UTF-16, new schema), one only the Uninstall key knows.
        let mass = game_folder(&drive, "Program Files/EA Games/Mass Effect Andromeda", OLD);
        let custom = game_folder(&drive, "Games/EA/It Takes Two", utf16(NEW));
        let elsewhere = game_folder(&drive, "Stuff/Odd Place", "<contentID>555</contentID>");
        std::fs::create_dir_all(drive.join("Program Files/EA Games/Not A Game")).unwrap();
        put(&drive.join("ProgramData/EA Desktop/machine.ini"), "machine.other=1\nmachine.downloadinplacedir=C:\\Games\\Nothing\n");
        put(&user.join("AppData/Local/Electronic Arts/EA Desktop/user_100.ini"), "user.userid=1\nuser.downloadinplacedir=C:\\Games\\EA\n");
        put(
            &dir.path().join("system.reg"),
            r#"WINE REGISTRY Version 2

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Odd] 1
"DisplayName"="Odd Game"
"InstallLocation"="C:\\Stuff\\Odd Place"
"UninstallString"="\"C:\\Program Files\\Electronic Arts\\EA Desktop\\EAInstaller.exe\" /uninstall"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Mass] 1
"DisplayName"="Mass Effect Registry Name"
"InstallLocation"="C:\\Program Files\\EA Games\\Mass Effect Andromeda"
"UninstallString"="C:\\Program Files\\Electronic Arts\\EA Desktop\\EAInstaller.exe"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Other] 1
"DisplayName"="Other App"
"InstallLocation"="C:\\Stuff\\Other"
"UninstallString"="C:\\other\\uninstall.exe"
"#,
        );

        let mut games = games(dir.path());
        games.sort_by(|a, b| a.id.cmp(&b.id));
        let got: Vec<_> = games.iter().map(|g| (g.id.as_str(), g.title.as_str(), g.launch.as_str())).collect();
        assert_eq!(
            got,
            [
                ("1026481", "It Takes Two", "\"origin2://game/launch?offerIds=1026481&autoDownload=1\""),
                ("555", "Odd Game", "\"origin2://game/launch?offerIds=555&autoDownload=1\""),
                ("71067", "Mass Effect™ Andromeda", "\"origin2://game/launch?offerIds=71067,71068&autoDownload=1\""),
            ]
        );
        let folders: Vec<_> = games.iter().map(|g| g.folder.clone().unwrap()).collect();
        assert_eq!(folders, [custom, elsewhere, mass]);
        assert_eq!(shlex::split(&games[2].launch).unwrap(), ["origin2://game/launch?offerIds=71067,71068&autoDownload=1"]);
    }

    #[test]
    fn a_game_without_a_title_takes_the_registry_name_then_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let drive = dir.path().join("drive_c");
        game_folder(&drive, "Program Files/EA Games/Folder Name", "<contentID>9</contentID>");
        assert_eq!(games(dir.path())[0].title, "Folder Name");
        put(
            &dir.path().join("system.reg"),
            "[Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\X] 1\n\"DisplayName\"=\"Registry Name\"\n\"InstallLocation\"=\"C:\\\\Program Files\\\\EA Games\\\\Folder Name\"\n\"UninstallString\"=\"EAInstaller.exe -x\"\n",
        );
        let games = games(dir.path());
        assert_eq!(games.len(), 1, "one folder is one game");
        assert_eq!(games[0].title, "Registry Name");
    }

    #[test]
    fn the_app_is_found_in_a_versioned_folder_or_by_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let drive = dir.path().join("drive_c");
        assert_eq!(versioned_exe(dir.path()), None);
        let old = drive.join("Program Files/Electronic Arts/EA Desktop/12.9.0.1/EA Desktop/EALauncher.exe");
        let new = drive.join("Program Files/Electronic Arts/EA Desktop/12.10.2.5/EA Desktop/EALauncher.exe");
        put(&old, "MZ");
        put(&new, "MZ");
        std::fs::create_dir_all(drive.join("Program Files/Electronic Arts/EA Desktop/empty")).unwrap();
        assert_eq!(versioned_exe(dir.path()), Some(new));

        let registered = tempfile::tempdir().unwrap();
        let exe = registered.path().join("drive_c/Apps/EA/EALauncher.exe");
        put(&exe, "MZ");
        put(
            &registered.path().join("system.reg"),
            "[Software\\\\Wow6432Node\\\\Electronic Arts\\\\EA Desktop] 1\n\"ClientPath\"=\"C:\\\\Apps\\\\EA\\\\EADesktop.exe\"\n",
        );
        assert_eq!(versioned_exe(registered.path()), Some(exe));
    }

    fn prefix_with_settings(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("drive_c/users/steamuser").join(USER_SETTINGS);
        std::fs::create_dir_all(&settings).unwrap();
        for (name, text) in files {
            put(&settings.join(name), text);
        }
        dir
    }

    #[test]
    fn a_user_id_in_the_settings_means_signed_in() {
        let signed = prefix_with_settings(&[("user_1000.ini", "user.country=BR\nuser.userid=1000123\n")]);
        assert_eq!(account(signed.path()), Account::SignedIn(None));
        let out = prefix_with_settings(&[("user_1000.ini", "user.country=BR\n"), ("other.ini", "user.userid=5")]);
        assert_eq!(account(out.path()), Account::SignedOut);
        assert_eq!(account(prefix_with_settings(&[]).path()), Account::SignedOut);
        assert_eq!(account(tempfile::tempdir().unwrap().path()), Account::Unknown);
    }
}
