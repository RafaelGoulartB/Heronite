//! Playnite (Windows): its library is `games.db` in its library folder, a
//! LiteDB 4 file ([`crate::litedb`]), its emulators `emulators.db` beside it,
//! and the games' covers and backgrounds are under `files`. A game starts
//! with Playnite's link, `playnite://playnite/start/<id>`: the shell passes
//! it on and returns, and Playnite (opened if it is closed) starts the game
//! with the settings it has there. Play time is in seconds.
//!
//! Its installed games come in, but the hidden ones and the ones of the
//! stores this app brings in itself ([`LEFT_TO_STORES`]: Steam, and Epic, GOG,
//! Amazon, Xbox, Battle.net, EA and Ubisoft where they are on).
//!
//! Where it is: the program the `playnite://` link opens (the last Playnite
//! started, portable or not), else the usual install folder. A portable copy
//! keeps its settings in its own folder, an installed one (it has an
//! uninstaller) in `%APPDATA%\Playnite`; its settings may move the library
//! (`DatabasePath`). Playnite holds its library open while it runs, and
//! nothing can read it then ([`Found::in_use`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use heronite_data::launchers::Launcher;
use heronite_platform::Feature;

use crate::litedb::{self, Bson, Document};
use crate::{Found, Game};

/// Playnite's library plugins of the stores this app has its own page for,
/// with the part of the app that page is (Steam's is always there).
const LEFT_TO_STORES: [(&str, Option<Feature>); 9] = [
    ("cb91dfc9-b977-43bf-8e70-55f46e410fab", None),
    ("00000002-dbd1-46c6-b5d0-b1ba559d10e4", Some(Feature::EpicStore)),
    ("aebe8b7c-6dc3-4a66-af31-e7375c6b5e9e", Some(Feature::GogStore)),
    ("402674cd-4af6-4886-b6ec-0e695bfa0688", Some(Feature::AmazonStore)),
    // Twitch, Amazon's games before Amazon Games.
    ("e2a7d494-c138-489d-bb3f-1d786beeb675", Some(Feature::AmazonStore)),
    ("7e4fbb5e-2ae3-48d4-8ba0-6b30e7a4e287", Some(Feature::XboxStore)),
    // Battle.net, the EA app (Origin) and Ubisoft Connect.
    ("e3c26a3d-d695-4cb7-a769-5ff7612c7edd", Some(Feature::StoreClients)),
    ("85dd7072-2f20-4e76-a007-41035e390724", Some(Feature::StoreClients)),
    ("c2f038e5-8b92-4877-91f1-da9094155fc5", Some(Feature::StoreClients)),
];

/// Whether the games of Playnite's library plugin `plugin` are left to this
/// app's own store page.
fn left_to_stores(plugin: &str) -> bool {
    LEFT_TO_STORES.iter().any(|(id, feature)| *id == plugin && feature.is_none_or(heronite_platform::supported))
}

/// Where one Playnite keeps its program and its library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub program: PathBuf,
    pub library: PathBuf,
}

pub fn detect() -> Result<Option<Found>> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let usual = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Playnite"));
    let found = [linked_program(), usual].into_iter().flatten().find_map(|p| install(&p, appdata.as_deref()));
    found.map(|i| read(&i)).transpose()
}

/// The folder of the program the `playnite://` link opens.
fn linked_program() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use heronite_platform::registry::{string, Hive};
        program_of(&string(Hive::CurrentUser, r"Software\Classes\Playnite\shell\open\command", "")?)
    }
    #[cfg(not(windows))]
    None
}

/// The folder of the program of a command line
/// (`"C:\Playnite\Playnite.DesktopApp.exe" --uridata "%1"`).
#[cfg_attr(not(windows), allow(dead_code))]
fn program_of(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let exe = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => command.split_whitespace().next()?,
    };
    Path::new(exe).parent().filter(|p| p.is_absolute()).map(Path::to_path_buf)
}

/// The Playnite whose program is in `program`, when its library is there.
/// `appdata` is `%APPDATA%`.
pub fn install(program: &Path, appdata: Option<&Path>) -> Option<Install> {
    let portable = !program.join("unins000.exe").is_file();
    let config = if portable { program.to_path_buf() } else { appdata?.join("Playnite") };
    let setting = std::fs::read_to_string(config.join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| v.get("DatabasePath")?.as_str().map(str::to_owned))
        .filter(|s| !s.trim().is_empty());
    let candidates = [setting.map(|s| database_path(&s, program, appdata)), Some(config.join("library"))];
    let library = candidates.into_iter().flatten().find(|l| l.join("games.db").is_file())?;
    Some(Install { program: program.to_path_buf(), library })
}

/// Playnite's `DatabasePath` setting, as a folder: `{PlayniteDir}` is its
/// program's folder, `%AppData%` the roaming one, and a relative path is in
/// its program's folder.
fn database_path(setting: &str, program: &Path, appdata: Option<&Path>) -> PathBuf {
    let mut path = setting.trim().replace("{PlayniteDir}", &program.to_string_lossy());
    if let (Some(at), Some(appdata)) = (path.to_ascii_lowercase().find("%appdata%"), appdata) {
        path.replace_range(at..at + "%appdata%".len(), &appdata.to_string_lossy());
    }
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        program.join(path)
    }
}

/// Whether reading a file failed because another program holds it (Windows'
/// sharing and lock violations).
fn held_open(e: &std::io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33))
}

/// The games of the Playnite `install`; its library held open (Playnite
/// runs) gives none, [`Found::in_use`].
pub fn read(install: &Install) -> Result<Found> {
    let mut found = Found {
        launcher: Launcher::Playnite,
        location: install.library.clone(),
        flatpak: false,
        games: Vec::new(),
        heroic: None,
        in_use: false,
    };
    let file = install.library.join("games.db");
    let bytes = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(e) if held_open(&e) => {
            found.in_use = true;
            return Ok(found);
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let docs = litedb::documents(&bytes).with_context(|| format!("reading {}", file.display()))?;
    let emulators = emulators(install);
    found.games = docs.iter().filter_map(|d| game(d, install, &emulators)).collect();
    found.games.sort_by_cached_key(|g| g.title.to_lowercase());
    Ok(found)
}

/// The folders of Playnite's emulators, by their id. None when they cannot
/// be read: the games then are followed by their own folder.
fn emulators(install: &Install) -> HashMap<String, PathBuf> {
    let file = install.library.join("emulators.db");
    let docs = std::fs::read(&file).map_err(anyhow::Error::from).and_then(|b| litedb::documents(&b));
    let docs = docs.unwrap_or_else(|e| {
        log::debug!("Reading {}: {e:#}", file.display());
        Vec::new()
    });
    docs.iter().filter_map(|d| Some((d.guid("_id")?, absolute(&expand(d.str("InstallDir")?, &install.program, None))?))).collect()
}

/// Playnite's variables in a path: `{PlayniteDir}`, and `{InstallDir}` (the
/// game's folder) when there is one.
fn expand(path: &str, program: &Path, folder: Option<&Path>) -> String {
    let mut out = path.trim().replace("{PlayniteDir}", &program.to_string_lossy());
    if let Some(folder) = folder {
        out = out.replace("{InstallDir}", &folder.to_string_lossy());
    }
    out
}

/// `path` without the separators at its end, when it is a full path.
fn absolute(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim_end_matches(['\\', '/']);
    // A drive's root keeps its separator (`D:\`).
    let path = PathBuf::from(if trimmed.ends_with(':') { path } else { trimmed });
    path.is_absolute().then_some(path)
}

/// A picture of the game: a file of the library's `files` folder (or a full
/// path). Links Playnite has not downloaded are left out.
fn media(value: &str, library: &Path) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.contains("://") {
        return None;
    }
    let path = Path::new(value);
    let path = if path.is_absolute() { path.to_path_buf() } else { library.join("files").join(path) };
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

/// Lower-case for comparing Windows paths.
fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase().replace('/', "\\")
}

/// Where the program of the game's first play action runs from, when it is
/// not the game's folder: an emulator's folder, or the folder of a program
/// elsewhere. The session follows the programs running from there.
fn runs_from(d: &Document, folder: Option<&Path>, install: &Install, emulators: &HashMap<String, PathBuf>) -> Option<String> {
    let Some(Bson::Array(actions)) = d.get("GameActions") else { return None };
    let action = actions.iter().find_map(|a| match a {
        Bson::Document(a) if a.bool("IsPlayAction") == Some(true) => Some(a),
        _ => None,
    })?;
    let dir = match action.str("Type")? {
        "Emulator" => emulators.get(&action.guid("EmulatorId")?)?.clone(),
        "File" => absolute(&expand(action.str("Path")?, &install.program, folder))?.parent()?.to_path_buf(),
        _ => return None,
    };
    if folder.is_some_and(|f| key(&dir).starts_with(&key(f))) {
        return None;
    }
    Some(dir.to_string_lossy().into_owned())
}

/// One game of the library, unless it is not installed, hidden, has no name
/// or is a game of a store this app brings in itself.
fn game(d: &Document, install: &Install, emulators: &HashMap<String, PathBuf>) -> Option<Game> {
    let id = d.guid("_id")?;
    let title = d.str("Name").map(str::trim).filter(|t| !t.is_empty())?.to_string();
    if !d.bool("IsInstalled").unwrap_or(false) || d.bool("Hidden").unwrap_or(false) {
        return None;
    }
    if d.guid("PluginId").is_some_and(|p| left_to_stores(&p)) {
        return None;
    }
    let folder = d.str("InstallDirectory").and_then(|f| absolute(&expand(f, &install.program, None)));
    let seconds = d.int("Playtime").unwrap_or(0).max(0);
    let last_played = d
        .date("LastActivity")
        .filter(|ms| *ms > 0)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let media = |key: &str| d.str(key).and_then(|v| media(v, &install.library));
    Some(Game {
        command: format!("explorer.exe playnite://playnite/start/{id}"),
        runs_from: runs_from(d, folder.as_deref(), install, emulators),
        folder: folder.map(|f| f.to_string_lossy().into_owned()),
        title,
        windows: true,
        minutes: (seconds as f64 / 60.0).round() as u64,
        last_played,
        cover: media("CoverImage"),
        banner: media("BackgroundImage"),
        prefix: None,
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::litedb::write::{file, guid_bytes};

    fn guid(text: &str) -> Bson {
        Bson::Binary(4, guid_bytes(text))
    }

    fn s(text: &str) -> Bson {
        Bson::String(text.into())
    }

    fn doc(fields: Vec<(&str, Bson)>) -> Document {
        Document(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    const NO_PLUGIN: &str = "00000000-0000-0000-0000-000000000000";
    const STEAM: &str = "cb91dfc9-b977-43bf-8e70-55f46e410fab";
    const DUCKSTATION: &str = "b289681d-4922-4bc4-b54f-9fb0cf2525dc";

    fn played(id: &str, name: &str, plugin: &str, installed: bool, extra: Vec<(&str, Bson)>) -> Document {
        let mut fields = vec![("_id", guid(id)), ("Name", s(name)), ("PluginId", guid(plugin)), ("IsInstalled", Bson::Bool(installed))];
        fields.extend(extra);
        // Playnite writes it; the first one counts, so one in `extra` wins.
        fields.push(("Hidden", Bson::Bool(false)));
        doc(fields)
    }

    fn action(fields: Vec<(&str, Bson)>) -> Bson {
        Bson::Document(doc(fields))
    }

    /// A portable Playnite in `dir`, its library holding `games`.
    fn portable(dir: &Path, games: &[Document], emulators: &[Document]) -> Install {
        let library = dir.join("library");
        std::fs::create_dir_all(library.join("files/0001")).unwrap();
        std::fs::write(library.join("games.db"), file(games)).unwrap();
        std::fs::write(library.join("emulators.db"), file(emulators)).unwrap();
        std::fs::write(dir.join("config.json"), "\u{feff}{\"DatabasePath\": \"{PlayniteDir}\\\\library\"}").unwrap();
        std::fs::write(library.join("files/0001/cover.jpg"), b"x").unwrap();
        install(dir, None).unwrap()
    }

    // Playnite's paths are Windows ones.
    #[cfg(windows)]
    #[test]
    fn reads_its_installed_games_but_hidden_and_store_ones() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let games = [
            played(
                "00000001-0000-0000-0000-000000000001",
                "Sekiro: Shadows Die Twice",
                NO_PLUGIN,
                true,
                vec![
                    ("InstallDirectory", s(r"C:\Games\Sekiro\")),
                    ("Playtime", Bson::Int64(4081)),
                    ("LastActivity", Bson::DateTime(1_767_323_045_123)),
                    ("CoverImage", s(r"0001\cover.jpg")),
                    ("BackgroundImage", s("https://example.com/bg.jpg")),
                    (
                        "GameActions",
                        Bson::Array(vec![action(vec![
                            ("Type", s("File")),
                            ("Path", s(r"{InstallDir}\sekiro.exe")),
                            ("IsPlayAction", Bson::Bool(true)),
                        ])]),
                    ),
                ],
            ),
            played(
                "00000002-0000-0000-0000-000000000002",
                "Castlevania - Symphony of the Night",
                NO_PLUGIN,
                true,
                vec![
                    ("InstallDirectory", s(r"D:\Games\PS\Castlevania")),
                    (
                        "GameActions",
                        Bson::Array(vec![
                            action(vec![("Type", s("Emulator")), ("EmulatorId", guid(DUCKSTATION)), ("IsPlayAction", Bson::Bool(true))]),
                            action(vec![("Type", s("File")), ("Path", s(r"D:\Other\x.exe")), ("IsPlayAction", Bson::Bool(true))]),
                        ]),
                    ),
                ],
            ),
            played("00000003-0000-0000-0000-000000000003", "Portal", STEAM, true, vec![]),
            played("00000004-0000-0000-0000-000000000004", "Not installed", NO_PLUGIN, false, vec![]),
            played("00000005-0000-0000-0000-000000000005", "Hidden", NO_PLUGIN, true, vec![("Hidden", Bson::Bool(true))]),
            played("00000006-0000-0000-0000-000000000006", "  ", NO_PLUGIN, true, vec![]),
        ];
        let emulators = [doc(vec![("_id", guid(DUCKSTATION)), ("Name", s("DuckStation")), ("InstallDir", s(r"D:\Emulators\duckstation"))])];
        let install = portable(root, &games, &emulators);
        assert_eq!(install.library, root.join("library"));

        let found = read(&install).unwrap();
        assert!(!found.in_use && !found.flatpak);
        assert_eq!(found.launcher, Launcher::Playnite);
        let names: Vec<_> = found.games.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(names, ["Castlevania - Symphony of the Night", "Sekiro: Shadows Die Twice"]);

        let sekiro = &found.games[1];
        assert_eq!(sekiro.id, "00000001-0000-0000-0000-000000000001");
        assert_eq!(sekiro.command, "explorer.exe playnite://playnite/start/00000001-0000-0000-0000-000000000001");
        assert_eq!(sekiro.minutes, 68);
        assert_eq!(sekiro.last_played.as_deref(), Some("2026-01-02T03:04:05.123Z"));
        assert_eq!(sekiro.cover.as_deref(), Some(root.join(r"library\files\0001\cover.jpg").to_str().unwrap()));
        assert_eq!(sekiro.banner, None, "a link Playnite has not downloaded");
        assert!(sekiro.windows && sekiro.prefix.is_none());
        assert_eq!(sekiro.folder.as_deref(), Some(r"C:\Games\Sekiro"));
        // Its program is in its folder: followed by it.
        assert_eq!(sekiro.runs_from, None);
        let castlevania = &found.games[0];
        assert_eq!(castlevania.folder.as_deref(), Some(r"D:\Games\PS\Castlevania"));
        assert_eq!(castlevania.runs_from.as_deref(), Some(r"D:\Emulators\duckstation"), "its emulator's folder");
    }

    /// While Playnite runs its library cannot be read: no games, and no
    /// error either.
    #[cfg(windows)]
    #[test]
    fn a_library_held_open_is_in_use() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tempfile::tempdir().unwrap();
        let game = played("00000001-0000-0000-0000-000000000001", "Game", NO_PLUGIN, true, vec![]);
        let install = portable(d.path(), &[game], &[]);
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(install.library.join("games.db")).unwrap();
        let found = read(&install).unwrap();
        assert!(found.in_use && found.games.is_empty());
        assert_eq!(found.count(), 0);
        assert!(
            crate::sync(&heronite_data::DataPaths::at(d.path().join("data")), &found, &Default::default()).is_err(),
            "nothing is marked as not installed"
        );
        drop(held);
        let found = read(&install).unwrap();
        assert!(!found.in_use && found.games.len() == 1);
    }

    #[test]
    fn its_library_may_be_elsewhere() {
        let d = tempfile::tempdir().unwrap();
        let (program, appdata) = (d.path().join("Playnite"), d.path().join("Roaming"));
        let library = appdata.join("Playnite/library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(&program).unwrap();
        std::fs::write(library.join("games.db"), file(&[])).unwrap();
        // A portable copy without its library is not one.
        assert_eq!(install(&program, Some(&appdata)), None);
        // Installed: its settings are in the roaming folder.
        std::fs::write(program.join("unins000.exe"), b"").unwrap();
        assert_eq!(install(&program, Some(&appdata)), Some(Install { program: program.clone(), library: library.clone() }));
        assert_eq!(install(&program, None), None);
        // Moved by its settings.
        let moved = d.path().join("Moved");
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::write(moved.join("games.db"), file(&[])).unwrap();
        let setting = serde_json::json!({ "DatabasePath": moved.to_string_lossy() }).to_string();
        std::fs::write(appdata.join("Playnite/config.json"), setting).unwrap();
        assert_eq!(install(&program, Some(&appdata)).map(|i| i.library), Some(moved));
        assert!(read(&install(&program, Some(&appdata)).unwrap()).unwrap().games.is_empty());
    }

    #[test]
    fn database_paths_and_commands_are_expanded() {
        let (program, appdata) = (Path::new(r"C:\Playnite"), Path::new(r"C:\Users\u\AppData\Roaming"));
        assert_eq!(database_path(r"{PlayniteDir}\library", program, None), PathBuf::from(r"C:\Playnite\library"));
        if cfg!(windows) {
            assert_eq!(database_path(r"%AppData%\Playnite\library", program, Some(appdata)), appdata.join(r"Playnite\library"));
            assert_eq!(database_path("library", program, None), program.join("library"));
            assert_eq!(program_of(r#""C:\Play nite\Playnite.DesktopApp.exe" --uridata "%1""#), Some(PathBuf::from(r"C:\Play nite")));
            assert_eq!(program_of(r"C:\Playnite\Playnite.DesktopApp.exe --uridata %1"), Some(PathBuf::from(r"C:\Playnite")));
            assert_eq!(absolute(r"D:\"), Some(PathBuf::from(r"D:\")));
        }
        assert_eq!(program_of(r#""Playnite.DesktopApp.exe""#), None);
        assert_eq!(absolute("games"), None);
    }

    #[test]
    fn only_the_stores_this_app_brings_in_are_left_out() {
        assert!(left_to_stores(STEAM));
        assert!(!left_to_stores(NO_PLUGIN));
        // itch.io: no page of its own here.
        assert!(!left_to_stores("00000001-ebb2-4eec-abcb-7c89937a42bb"));
        let epic = "00000002-dbd1-46c6-b5d0-b1ba559d10e4";
        assert_eq!(left_to_stores(epic), heronite_platform::supported(Feature::EpicStore));
    }
}
