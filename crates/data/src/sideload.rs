//! Adding, editing and removing sideloaded ("Local") games: what Heroic's
//! `SideloadDialog` + `SideloadLibraryManager.addNewApp` write, plus the
//! Heronite overlay hook (`applySideloadAppToLocalMeta` and
//! `syncLocalExecutablePlatform`).
//!
//! * `sideload_apps/library.json` (`games`): the `GameInfo` entry, tab-indented
//!   electron-store file, read-modify-write of the raw document (unknown keys
//!   and key order survive; an edit is a shallow merge like
//!   `{ ...current[index], ...game }`).
//! * `local_library/library.json`: the overlay meta (`remappedExecutable`,
//!   `launchKind`, `runAs`, ...).
//! * `GamesConfig/<app>.json`: the Wine prefix/version chosen in the dialog
//!   (written the way `GameConfig.setSetting` does).
//!
//! Removing a game is `heronite_metadata::remove::remove_collection_game`;
//! this module only adds the optional Wine prefix removal.
//!
//! `app_name` ids are `short-uuid`'s default output (a UUIDv4 in the Flickr
//! base58 alphabet, 22 characters), exactly what Heroic generates.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use anyhow::Result;
use heronite_i18n::tr;
use serde_json::{json, Map, Value};

use crate::game_config;
use crate::json_store;
use crate::models::LocalMetaFile;
use crate::paths::DataPaths;

/// `short-uuid` default alphabet (`flickrBase58`).
const SHORT_ALPHABET: &[u8; 58] = b"123456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ";
/// `ceil(log(2^128) / log(58))`.
const SHORT_LEN: usize = 22;

/// `short.fromUUID(uuid)` for the 16 bytes of a UUID.
pub fn short_uuid(bytes: [u8; 16]) -> String {
    let mut n = u128::from_be_bytes(bytes);
    let mut out = Vec::with_capacity(SHORT_LEN);
    while n > 0 {
        out.push(SHORT_ALPHABET[(n % 58) as usize]);
        n /= 58;
    }
    while out.len() < SHORT_LEN {
        out.push(SHORT_ALPHABET[0]);
    }
    out.reverse();
    String::from_utf8(out).expect("ascii alphabet")
}

fn random_bytes() -> [u8; 16] {
    // `RandomState` keys come from the OS; two independent hashers give 128
    // bits without an extra dependency.
    let mut out = [0u8; 16];
    for half in out.as_chunks_mut::<8>().0 {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        half.copy_from_slice(&h.finish().to_be_bytes());
    }
    out
}

/// `short.generate()`: the `app_name` of a new sideloaded game.
pub fn generate_app_name() -> String {
    let mut b = random_bytes();
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
    short_uuid(b)
}

/// How the program runs; decides `install.platform` and `is_linux_native`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideloadPlatform {
    /// Native Linux program (`install.platform: "linux"`).
    Linux,
    /// Windows program through Wine/Proton (`"Windows"`).
    Windows,
    /// A web page (`"Browser"`).
    Browser,
}

impl SideloadPlatform {
    pub fn install_platform(self) -> &'static str {
        match self {
            SideloadPlatform::Linux => "linux",
            SideloadPlatform::Windows => "Windows",
            SideloadPlatform::Browser => "Browser",
        }
    }

    /// Reads `install.platform` (any case; macOS counts as Linux-native here).
    pub fn from_install_platform(p: &str) -> Option<Self> {
        match p.to_ascii_lowercase().as_str() {
            "linux" | "mac" | "osx" => Some(Self::Linux),
            "windows" | "win32" => Some(Self::Windows),
            "browser" => Some(Self::Browser),
            _ => None,
        }
    }
}

/// What the Add/Edit dialog collects.
#[derive(Clone, Debug)]
pub struct SideloadSpec {
    pub app_name: String,
    pub title: String,
    pub executable: String,
    pub platform: SideloadPlatform,
    /// `art_cover` (hero) and `art_square`; an empty value keeps what an
    /// edited game already has.
    pub art_cover: String,
    pub art_square: String,
    pub browser_url: String,
    pub custom_user_agent: String,
    pub launch_full_screen: bool,
    /// `runAs` for the overlay meta: `Some("linux" | "windows" | "auto")`
    /// sets it, `None` leaves it as it is.
    pub run_as: Option<String>,
    /// "Install manually" into this entry: it becomes an executable game,
    /// even when it was an emulated one (an edit keeps the emulator).
    pub install: bool,
    /// The game's folder (`install.install_path`). Empty: the executable's
    /// folder.
    pub install_folder: String,
    /// Linux games started by a shell command run inside `install_folder`
    /// instead of an executable ([`COMMAND_KEY`]).
    pub command: String,
}

/// The shell command that starts a Linux game, run inside its folder.
pub const COMMAND_KEY: &str = "heroniteCommand";

/// `(command, folder)` of a game started by a command (see
/// [`SideloadSpec::command`]).
pub fn command_of(game: &crate::models::GameInfo) -> Option<(&str, &str)> {
    let command = game.rest.get(COMMAND_KEY).and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty())?;
    let folder = game.install.install_path.as_deref().map(str::trim).filter(|f| !f.is_empty())?;
    Some((command, folder))
}

#[derive(Clone, Debug)]
pub struct SaveOutcome {
    /// `false` when an existing entry was edited.
    pub added: bool,
    /// The entry as written to `sideload_apps/library.json`.
    pub game: Value,
}

fn dirname(path: &str) -> String {
    match Path::new(path).parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_string_lossy().into_owned(),
        _ => ".".to_string(),
    }
}

/// The `GameInfo` object `addNewApp` builds, in its key order, with the
/// platform sync of `syncLocalExecutablePlatform` applied (`is_linux_native`
/// last).
fn build_game(spec: &SideloadSpec) -> Map<String, Value> {
    let mut g = Map::new();
    g.insert("runner".into(), json!("sideload"));
    g.insert("app_name".into(), json!(spec.app_name));
    g.insert("title".into(), json!(spec.title));
    let mut install = json!({
        "executable": spec.executable,
        "platform": spec.platform.install_platform(),
        "is_dlc": false,
    });
    let folder = spec.install_folder.trim().trim_end_matches('/');
    if !folder.is_empty() {
        install["install_path"] = json!(folder);
    }
    g.insert("install".into(), install);
    if !folder.is_empty() {
        g.insert("folder_name".into(), json!(folder));
    } else if !spec.executable.is_empty() {
        g.insert("folder_name".into(), json!(dirname(&spec.executable)));
    }
    g.insert("art_cover".into(), json!(spec.art_cover));
    g.insert("is_installed".into(), json!(true));
    g.insert("art_square".into(), json!(spec.art_square));
    g.insert("canRunOffline".into(), json!(spec.browser_url.is_empty()));
    g.insert("browserUrl".into(), json!(spec.browser_url));
    g.insert("customUserAgent".into(), json!(spec.custom_user_agent));
    g.insert("launchFullScreen".into(), json!(spec.launch_full_screen));
    if spec.platform != SideloadPlatform::Browser {
        g.insert("is_linux_native".into(), json!(spec.platform == SideloadPlatform::Linux));
    }
    if spec.platform == SideloadPlatform::Linux && !spec.command.trim().is_empty() {
        g.insert(COMMAND_KEY.into(), json!(spec.command.trim()));
    }
    g
}

/// Shallow merge of `game` into `current` (`{ ...current, ...game }`); keys
/// the new game leaves out because they do not apply (`folder_name` without an
/// executable, `is_linux_native` for a web page) are dropped, and empty art
/// never wipes art the game already has.
fn merge_into(current: &mut Map<String, Value>, game: Map<String, Value>) {
    for key in ["folder_name", "is_linux_native", COMMAND_KEY] {
        if !game.contains_key(key) {
            current.shift_remove(key);
        }
    }
    for (k, v) in game {
        let keep_art = matches!(k.as_str(), "art_cover" | "art_square")
            && v.as_str() == Some("")
            && current.get(&k).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
        if keep_art {
            continue;
        }
        if k == "install" {
            // Keep other `install` keys (install_path, ...) of an edited game.
            if let (Some(Value::Object(old)), Value::Object(new)) = (current.get_mut("install"), &v) {
                for (ik, iv) in new {
                    old.insert(ik.clone(), iv.clone());
                }
                continue;
            }
        }
        current.insert(k, v);
    }
}

fn looks_like_steam_binary(path: &str) -> bool {
    let n = path.replace('\\', "/").to_lowercase();
    path == "steam" || n.ends_with("/steam") || n.ends_with("/steam.exe") || n.ends_with("/steam.sh")
}

/// `applySideloadAppToLocalMeta`: keeps the overlay meta of the game in line
/// with the sideload entry (the executable is what the Collection launches).
fn apply_local_meta(paths: &DataPaths, spec: &SideloadSpec) -> Result<()> {
    let executable = spec.executable.trim().to_string();
    // A game started by a command runs like an executable one.
    let runnable = !executable.is_empty() || (spec.platform == SideloadPlatform::Linux && !spec.command.trim().is_empty());
    // Decide before touching the file: a skipped game must not even rewrite it.
    let current: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let existing = current.games.get(&spec.app_name);
    if !runnable && existing.is_none() {
        return Ok(());
    }
    if let Some(e) = existing {
        if e.launch_kind == "steam-uri" && (!runnable || looks_like_steam_binary(&executable)) {
            return Ok(());
        }
    }
    // Installing into an emulated entry: the emulator's arguments go away.
    let emulator_args =
        existing.filter(|e| spec.install && runnable && e.launch_kind == "emulator").map(|e| e.launcher_args.clone().unwrap_or_default());
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        let existing = file.games.get(&spec.app_name).cloned();
        let mut meta = existing.clone().unwrap_or_default();
        if emulator_args.is_some() {
            meta.launcher_args = None;
            // No longer an emulated game: it reads as a local one.
            if meta.source == "emulator" {
                meta.source = "manual".into();
            }
        }
        meta.app_name = spec.app_name.clone();
        meta.runner = "sideload".into();
        if meta.playnite_id.is_empty() {
            meta.playnite_id = format!("heroic_{}", spec.app_name);
        }
        if meta.source.is_empty() {
            meta.source = "manual".into();
        }
        meta.launch_kind = if runnable {
            if meta.launch_kind == "emulator" && !spec.install {
                "emulator".into()
            } else {
                "executable".into()
            }
        } else if meta.launch_kind.is_empty() {
            "unavailable".into()
        } else {
            meta.launch_kind.clone()
        };
        if !spec.title.is_empty() {
            meta.title = spec.title.clone();
        } else if meta.title.is_empty() {
            meta.title = spec.app_name.clone();
        }
        meta.remapped_executable = (!executable.is_empty()).then_some(executable.clone());
        if let Some(run_as) = &spec.run_as {
            meta.run_as = Some(run_as.clone());
        }
        file.games.insert(spec.app_name.clone(), meta);
        Ok(())
    })?;
    match emulator_args {
        Some(args) => game_config::forget_emulator_args(paths, &spec.app_name, &args),
        None => Ok(()),
    }
}

/// `SideloadLibraryManager.addNewApp` + the overlay hooks: adds the game, or
/// edits it when `app_name` exists.
pub fn save_game(paths: &DataPaths, spec: &SideloadSpec) -> Result<SaveOutcome> {
    let game = build_game(spec);
    let (added, written) = json_store::update_value(&paths.sideload_library(), |doc| {
        let obj = doc.as_object_mut().expect("update_value gives an object");
        let games = obj.entry("games").or_insert_with(|| Value::Array(Vec::new()));
        if !games.is_array() {
            *games = Value::Array(Vec::new());
        }
        let list = games.as_array_mut().expect("array");
        let index = list.iter().position(|g| g.get("app_name").and_then(Value::as_str) == Some(spec.app_name.as_str()));
        match index {
            Some(i) => {
                if let Some(cur) = list[i].as_object_mut() {
                    merge_into(cur, game);
                } else {
                    list[i] = Value::Object(game);
                }
                Ok((false, list[i].clone()))
            }
            None => {
                let v = Value::Object(game);
                list.push(v.clone());
                Ok((true, v))
            }
        }
    })?;
    apply_local_meta(paths, spec)?;
    Ok(SaveOutcome { added, game: written })
}

/// The Wine settings a new Windows game gets (`writeConfig` of the dialog):
/// prefix, Wine version and CrossOver bottle, over the defaults Heroic writes
/// for a new game file. `wine_version` is the chosen `WineInstallation`.
pub fn init_game_config(
    paths: &DataPaths,
    home: &str,
    app_name: &str,
    wine_prefix: &str,
    wine_version: Option<Value>,
    crossover_bottle: &str,
) -> Result<()> {
    let mut changes: Vec<(String, Value)> = Vec::new();
    if !wine_prefix.trim().is_empty() {
        changes.push(("winePrefix".into(), json!(wine_prefix.trim())));
    }
    if let Some(v) = wine_version {
        changes.push(("wineVersion".into(), v));
    }
    changes.push(("wineCrossoverBottle".into(), json!(crossover_bottle)));
    game_config::set_game_settings(paths, app_name, home, &changes)
}

/// `GameInfo` of a sideloaded game from the raw library file.
pub fn find_game(paths: &DataPaths, app_name: &str) -> Result<Option<Value>> {
    let doc = json_store::read_value(&paths.sideload_library())?;
    Ok(doc
        .get("games")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name)))
        .cloned())
}

// Removing: the game itself goes through `heronite_metadata::remove`
// (Heronite's "Remove from Collection"); only the Wine prefix is handled here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefixOutcome {
    Removed(PathBuf),
    /// Left alone, with the reason (Heroic's sanity checks).
    Skipped(String),
}

/// Heroic's `removePrefix` sanity checks, plus refusing the home folder, its
/// parents and the filesystem root.
pub fn check_prefix_removable(prefix: &Path, protected: &[PathBuf], home: &Path) -> Result<(), String> {
    if !prefix.exists() {
        return Err(tr!("Prefix folder {path} doesn't exist", path = prefix.display()));
    }
    if prefix.parent().is_none() || home.starts_with(prefix) {
        return Err(tr!("Won't delete {path}: it contains your home folder", path = prefix.display()));
    }
    if protected.iter().any(|p| !p.as_os_str().is_empty() && p == prefix) {
        return Err(tr!("Can't delete {path}: it is a shared or default folder", path = prefix.display()));
    }
    let non_empty = std::fs::read_dir(prefix).map(|mut d| d.next().is_some()).unwrap_or(false);
    if non_empty && !prefix.join("drive_c").exists() && !prefix.join("pfx").exists() {
        return Err(tr!(
            "Can't delete {path}: it has no drive_c/pfx folder. If this is the right prefix, delete it by hand",
            path = prefix.display()
        ));
    }
    Ok(())
}

/// Deletes the Wine prefix of a game (Heroic's `removePrefix`). Never touches
/// the shared prefix, the default install/prefix folders or anything that
/// does not look like a prefix.
pub fn remove_prefix(paths: &DataPaths, app_name: &str, home: &str) -> PrefixOutcome {
    let settings = game_config::load_game(paths, app_name, home);
    let global = game_config::load_global(paths, home);
    let Some(prefix) = settings.get("winePrefix").and_then(Value::as_str).filter(|p| !p.is_empty()) else {
        return PrefixOutcome::Skipped(tr!("The game has no Wine prefix").into());
    };
    let text = |k: &str| global.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let protected: Vec<PathBuf> = [
        text("defaultInstallPath"),
        game_config::shared_wine_prefix(home),
        text("defaultWinePrefix"),
        text("defaultWinePrefixDir"),
        text("winePrefix"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    let prefix = PathBuf::from(prefix);
    if let Err(why) = check_prefix_removable(&prefix, &protected, Path::new(home)) {
        return PrefixOutcome::Skipped(why);
    }
    match std::fs::remove_dir_all(&prefix) {
        Ok(()) => PrefixOutcome::Removed(prefix),
        Err(e) => PrefixOutcome::Skipped(tr!("Could not delete {path}: {e}", path = prefix.display(), e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths)
    }

    fn spec(app: &str, exe: &str, platform: SideloadPlatform) -> SideloadSpec {
        SideloadSpec {
            app_name: app.into(),
            title: "My Game".into(),
            executable: exe.into(),
            platform,
            art_cover: "https://example.com/hero.jpg".into(),
            art_square: "https://example.com/grid.jpg".into(),
            browser_url: String::new(),
            custom_user_agent: String::new(),
            launch_full_screen: false,
            run_as: None,
            install: false,
            install_folder: String::new(),
            command: String::new(),
        }
    }

    const LIBRARY: &str = "{\n\t\"games\": [\n\t\t{\n\t\t\t\"runner\": \"sideload\",\n\t\t\t\"app_name\": \"steam_400\",\n\t\t\t\"title\": \"Portal\",\n\t\t\t\"is_installed\": false,\n\t\t\t\"install\": {\n\t\t\t\t\"executable\": \"\",\n\t\t\t\t\"platform\": \"linux\",\n\t\t\t\t\"is_dlc\": false\n\t\t\t},\n\t\t\t\"zzCustom\": [\n\t\t\t\t1\n\t\t\t]\n\t\t}\n\t],\n\t\"zzOther\": true\n}";

    #[test]
    fn short_uuid_matches_the_js_library() {
        // Values produced by `short-uuid` 4.2.2 `fromUUID`.
        let hex = |s: &str| -> [u8; 16] {
            let h: String = s.chars().filter(|c| *c != '-').collect();
            let mut b = [0u8; 16];
            for i in 0..16 {
                b[i] = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap();
            }
            b
        };
        assert_eq!(short_uuid(hex("00000000-0000-4000-8000-000000000000")), "111111114Ay6byqtQtQyDN");
        assert_eq!(short_uuid(hex("ffffffff-ffff-4fff-bfff-ffffffffffff")), "xBuEXKpzVpVzLaDrb3orAX");
        assert_eq!(short_uuid(hex("123e4567-e89b-42d3-a456-426614174000")), "3fEgj34VZ4kh5joEmQykym");
    }

    #[test]
    fn generated_names_look_like_heroics() {
        let a = generate_app_name();
        let b = generate_app_name();
        assert_ne!(a, b);
        assert_eq!(a.len(), 22);
        assert!(a.bytes().all(|c| SHORT_ALPHABET.contains(&c)));
    }

    #[test]
    fn adding_appends_in_heroics_key_order_and_keeps_the_rest_byte_identical() {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.sideload_library().parent().unwrap()).unwrap();
        fs::write(paths.sideload_library(), LIBRARY).unwrap();
        let out = save_game(&paths, &spec("abc", "/g/My Game/game.exe", SideloadPlatform::Windows)).unwrap();
        assert!(out.added);
        let text = fs::read_to_string(paths.sideload_library()).unwrap();
        // The old entry and the unknown top-level key are untouched.
        assert!(text.starts_with(&LIBRARY[..LIBRARY.find("\n\t\t}\n\t],").unwrap()]), "{text}");
        assert!(text.ends_with("\n\t\"zzOther\": true\n}"));
        assert!(!text.ends_with('\n'));
        let doc: Value = serde_json::from_str(&text).unwrap();
        let g = &doc["games"][1];
        let keys: Vec<&str> = g.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "runner",
                "app_name",
                "title",
                "install",
                "folder_name",
                "art_cover",
                "is_installed",
                "art_square",
                "canRunOffline",
                "browserUrl",
                "customUserAgent",
                "launchFullScreen",
                "is_linux_native"
            ]
        );
        assert_eq!(g["install"], json!({"executable": "/g/My Game/game.exe", "platform": "Windows", "is_dlc": false}));
        assert_eq!(g["folder_name"], "/g/My Game");
        assert_eq!(g["is_linux_native"], false);
        assert_eq!(g["canRunOffline"], true);
        assert_eq!(g["art_square"], "https://example.com/grid.jpg");
    }

    #[test]
    fn missing_library_file_is_created_with_games_only() {
        let (_d, paths) = setup();
        save_game(&paths, &spec("abc", "/g/x.sh", SideloadPlatform::Linux)).unwrap();
        let doc = json_store::read_value(&paths.sideload_library()).unwrap();
        assert_eq!(doc.as_object().unwrap().len(), 1);
        assert_eq!(doc["games"][0]["is_linux_native"], true);
        assert_eq!(doc["games"][0]["install"]["platform"], "linux");
    }

    #[test]
    fn overlay_meta_follows_addnewapp() {
        let (_d, paths) = setup();
        save_game(&paths, &spec("abc", "/g/x.exe", SideloadPlatform::Windows)).unwrap();
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        let m = &metas.games["abc"];
        assert_eq!(m.runner, "sideload");
        assert_eq!(m.playnite_id, "heroic_abc");
        assert_eq!(m.source, "manual");
        assert_eq!(m.launch_kind, "executable");
        assert_eq!(m.remapped_executable.as_deref(), Some("/g/x.exe"));
        assert_eq!(m.title, "My Game");
        assert_eq!(m.run_as, None);
        // Raw order matches the TS object literal.
        let raw = json_store::read_value(&paths.local_meta()).unwrap();
        let keys: Vec<&str> = raw["games"]["abc"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["appName", "runner", "playniteId", "source", "launchKind", "title", "remappedExecutable"]);
    }

    #[test]
    fn browser_games_get_no_meta_and_no_native_flag() {
        let (_d, paths) = setup();
        let mut s = spec("web", "", SideloadPlatform::Browser);
        s.browser_url = "https://example.com/play".into();
        s.custom_user_agent = "UA".into();
        s.launch_full_screen = true;
        let out = save_game(&paths, &s).unwrap();
        assert!(!paths.local_meta().exists());
        let g = out.game;
        assert!(g.get("is_linux_native").is_none());
        assert!(g.get("folder_name").is_none());
        assert_eq!(g["canRunOffline"], false);
        assert_eq!(g["browserUrl"], "https://example.com/play");
        assert_eq!(g["launchFullScreen"], true);
        assert_eq!(g["install"]["platform"], "Browser");
    }

    #[test]
    fn editing_merges_and_keeps_unknown_fields_art_and_position() {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.sideload_library().parent().unwrap()).unwrap();
        fs::write(paths.sideload_library(), LIBRARY).unwrap();
        save_game(&paths, &spec("steam_400", "/g/portal.sh", SideloadPlatform::Linux)).unwrap();
        // Second edit with empty art keeps the art and changes the title.
        let mut s = spec("steam_400", "/g/portal2.sh", SideloadPlatform::Linux);
        s.title = "Portal Renamed".into();
        s.art_cover = String::new();
        s.art_square = String::new();
        let out = save_game(&paths, &s).unwrap();
        assert!(!out.added);
        let doc = json_store::read_value(&paths.sideload_library()).unwrap();
        assert_eq!(doc["games"].as_array().unwrap().len(), 1);
        let g = &doc["games"][0];
        assert_eq!(g["title"], "Portal Renamed");
        assert_eq!(g["zzCustom"], json!([1]));
        assert_eq!(g["art_square"], "https://example.com/grid.jpg");
        assert_eq!(g["install"]["executable"], "/g/portal2.sh");
        assert_eq!(g["is_installed"], true);
        // `runner` stays first: edited keys keep their position.
        assert_eq!(g.as_object().unwrap().keys().next().unwrap(), "runner");
    }

    #[test]
    fn switching_to_a_web_page_drops_native_keys() {
        let (_d, paths) = setup();
        save_game(&paths, &spec("g", "/g/x.sh", SideloadPlatform::Linux)).unwrap();
        let mut s = spec("g", "", SideloadPlatform::Browser);
        s.browser_url = "https://x.test".into();
        let out = save_game(&paths, &s).unwrap();
        assert!(out.game.get("is_linux_native").is_none());
        assert!(out.game.get("folder_name").is_none());
        // The overlay meta of an existing game stays, without an executable.
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(metas.games["g"].remapped_executable, None);
        assert_eq!(metas.games["g"].launch_kind, "executable");
    }

    #[test]
    fn existing_meta_is_preserved_and_run_as_is_applied() {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.local_meta().parent().unwrap()).unwrap();
        fs::write(
            paths.local_meta(),
            "{\n\t\"games\": {\n\t\t\"g\": {\n\t\t\t\"appName\": \"g\",\n\t\t\t\"runner\": \"sideload\",\n\t\t\t\"playniteId\": \"pid\",\n\t\t\t\"source\": \"steam\",\n\t\t\t\"launchKind\": \"emulator\",\n\t\t\t\"title\": \"Old\",\n\t\t\t\"emulatorId\": \"pcsx2\",\n\t\t\t\"zz\": 1\n\t\t}\n\t}\n}",
        )
        .unwrap();
        let mut s = spec("g", "/g/emu.AppImage", SideloadPlatform::Linux);
        s.run_as = Some("linux".into());
        save_game(&paths, &s).unwrap();
        let raw = json_store::read_value(&paths.local_meta()).unwrap();
        let m = &raw["games"]["g"];
        assert_eq!(m["playniteId"], "pid");
        assert_eq!(m["source"], "steam");
        assert_eq!(m["launchKind"], "emulator");
        assert_eq!(m["emulatorId"], "pcsx2");
        assert_eq!(m["zz"], 1);
        assert_eq!(m["runAs"], "linux");
        assert_eq!(m["title"], "My Game");
    }

    #[test]
    fn installing_an_emulated_game_makes_it_an_executable_game() {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.local_meta().parent().unwrap()).unwrap();
        fs::write(
            paths.local_meta(),
            "{\n\t\"games\": {\n\t\t\"g\": {\n\t\t\t\"appName\": \"g\",\n\t\t\t\"runner\": \"sideload\",\n\t\t\t\"source\": \"emulator\",\n\t\t\t\"launchKind\": \"emulator\",\n\t\t\t\"launcherArgs\": \"-batch {ImagePath}\",\n\t\t\t\"title\": \"Old\"\n\t\t}\n\t}\n}",
        )
        .unwrap();
        fs::create_dir_all(paths.game_config("g").parent().unwrap()).unwrap();
        fs::write(paths.game_config("g"), r#"{"g": {"launcherArgs": "-batch {ImagePath}", "showFps": true}, "version": "v0"}"#).unwrap();
        let mut s = spec("g", "/g/game.exe", SideloadPlatform::Windows);
        s.install = true;
        save_game(&paths, &s).unwrap();
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(metas.games["g"].launch_kind, "executable");
        assert_eq!(metas.games["g"].remapped_executable.as_deref(), Some("/g/game.exe"));
        assert_eq!(metas.games["g"].launcher_args, None);
        let config = json_store::read_value(&paths.game_config("g")).unwrap();
        assert_eq!(config["g"]["launcherArgs"], "");
        assert_eq!(config["g"]["showFps"], true);
    }

    #[test]
    fn an_emulated_game_can_become_a_linux_game_started_by_a_command() {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.local_meta().parent().unwrap()).unwrap();
        fs::write(
            paths.local_meta(),
            "{\n\t\"games\": {\n\t\t\"g\": {\n\t\t\t\"appName\": \"g\",\n\t\t\t\"runner\": \"sideload\",\n\t\t\t\"source\": \"emulator\",\n\t\t\t\"launchKind\": \"emulator\",\n\t\t\t\"launcherArgs\": \"--no-gui {ImagePath}\",\n\t\t\t\"title\": \"GoW3\"\n\t\t}\n\t}\n}",
        )
        .unwrap();
        let mut s = spec("g", "", SideloadPlatform::Linux);
        s.install = true;
        s.install_folder = "/games/gow3/".into();
        s.command = "  ./gow3 --fullscreen ".into();
        let out = save_game(&paths, &s).unwrap();
        assert_eq!(out.game["install"]["install_path"], "/games/gow3");
        assert_eq!(out.game["folder_name"], "/games/gow3");
        assert_eq!(out.game[COMMAND_KEY], "./gow3 --fullscreen");
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(metas.games["g"].launch_kind, "executable", "a command game runs like an executable one");
        assert_eq!(metas.games["g"].remapped_executable, None);
        assert_eq!(metas.games["g"].launcher_args, None, "the emulator's arguments go away");
        assert_eq!(metas.games["g"].source, "manual", "it no longer reads as an emulated game");
        let game: crate::models::GameInfo = serde_json::from_value(out.game).unwrap();
        assert_eq!(command_of(&game), Some(("./gow3 --fullscreen", "/games/gow3")));

        // Back to an executable: the command is dropped, the folder stays.
        let mut s = spec("g", "/games/gow3/gow3", SideloadPlatform::Linux);
        s.install_folder = "/games/gow3".into();
        let out = save_game(&paths, &s).unwrap();
        assert!(out.game.get(COMMAND_KEY).is_none());
        assert_eq!(out.game["install"]["install_path"], "/games/gow3");
        // A command is a Linux thing only.
        let mut s = spec("w", "/w/game.exe", SideloadPlatform::Windows);
        s.command = "./x".into();
        assert!(save_game(&paths, &s).unwrap().game.get(COMMAND_KEY).is_none());
    }

    #[test]
    fn steam_uri_games_keep_their_meta_when_the_executable_is_steam() {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.local_meta().parent().unwrap()).unwrap();
        let before = "{\n\t\"games\": {\n\t\t\"g\": {\n\t\t\t\"appName\": \"g\",\n\t\t\t\"runner\": \"sideload\",\n\t\t\t\"launchKind\": \"steam-uri\",\n\t\t\t\"title\": \"S\"\n\t\t}\n\t}\n}";
        fs::write(paths.local_meta(), before).unwrap();
        save_game(&paths, &spec("g", "/usr/bin/steam", SideloadPlatform::Linux)).unwrap();
        assert_eq!(fs::read_to_string(paths.local_meta()).unwrap(), before);
    }

    #[test]
    fn game_config_for_a_new_windows_game_has_prefix_and_wine() {
        let (_d, paths) = setup();
        fs::create_dir_all(paths.games_config_dir()).unwrap();
        init_game_config(
            &paths,
            "/home/u",
            "abc",
            "/home/u/Games/Heroic/Prefixes/My Game",
            Some(json!({"bin": "/w/proton", "name": "GE", "type": "proton"})),
            "",
        )
        .unwrap();
        let text = fs::read_to_string(paths.game_config("abc")).unwrap();
        assert!(!text.ends_with('\n'));
        assert!(text.starts_with("{\n  \"abc\": {\n"), "{text}");
        let doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["version"], "v0");
        assert_eq!(doc["explicit"], true);
        assert_eq!(doc["abc"]["winePrefix"], "/home/u/Games/Heroic/Prefixes/My Game");
        assert_eq!(doc["abc"]["wineVersion"]["type"], "proton");
        assert_eq!(doc["abc"]["wineCrossoverBottle"], "");
        assert_eq!(doc["abc"]["disableUMU"], false);
        assert_eq!(doc["abc"]["autoInstallDxvk"], true);
    }

    #[test]
    fn prefix_removal_follows_heroics_sanity_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let pfx = home.join("Games/Heronite/Prefixes/My Game");
        fs::create_dir_all(pfx.join("drive_c")).unwrap();
        let shared = home.join("Games/Heronite/Prefixes/shared");
        fs::create_dir_all(shared.join("drive_c")).unwrap();
        let junk = home.join("Games/Heronite/Prefixes/junk");
        fs::create_dir_all(&junk).unwrap();
        fs::write(junk.join("notes.txt"), "x").unwrap();
        let protected = vec![shared.clone(), home.join("Games/Heronite")];
        assert!(check_prefix_removable(&pfx, &protected, &home).is_ok());
        assert!(check_prefix_removable(&shared, &protected, &home).unwrap_err().contains("shared or default"));
        assert!(check_prefix_removable(&junk, &protected, &home).unwrap_err().contains("drive_c/pfx"));
        assert!(check_prefix_removable(&home, &protected, &home).unwrap_err().contains("home folder"));
        assert!(check_prefix_removable(Path::new("/"), &protected, &home).is_err());
        assert!(check_prefix_removable(&home.join("nope"), &protected, &home).unwrap_err().contains("doesn't exist"));
        // An empty folder is fine to remove.
        let empty = home.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(check_prefix_removable(&empty, &protected, &home).is_ok());
    }

    #[test]
    fn remove_prefix_reads_the_games_own_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let paths = DataPaths::at(tmp.path().join("cfg"));
        fs::create_dir_all(paths.games_config_dir()).unwrap();
        let pfx = home.join("Games/Heronite/Prefixes/G");
        fs::create_dir_all(pfx.join("pfx")).unwrap();
        init_game_config(&paths, home.to_str().unwrap(), "g", pfx.to_str().unwrap(), None, "").unwrap();
        assert_eq!(remove_prefix(&paths, "g", home.to_str().unwrap()), PrefixOutcome::Removed(pfx.clone()));
        assert!(!pfx.exists());
        // The shared prefix is never removed.
        let shared = home.join("Games/Heronite/Prefixes/shared");
        fs::create_dir_all(shared.join("drive_c")).unwrap();
        init_game_config(&paths, home.to_str().unwrap(), "s", shared.to_str().unwrap(), None, "").unwrap();
        assert!(matches!(remove_prefix(&paths, "s", home.to_str().unwrap()), PrefixOutcome::Skipped(_)));
        assert!(shared.exists());
    }

    /// Run with `HERONITE_FIXTURE=/tmp/heronite-fixture`: adding a game to a
    /// copy of the real library leaves every existing byte alone.
    #[test]
    fn real_library_keeps_existing_entries_byte_identical() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE").map(PathBuf::from) else {
            eprintln!("HERONITE_FIXTURE not set; skipping");
            return;
        };
        let src = root.join("sideload_apps/library.json");
        if !src.exists() {
            return;
        }
        let (_d, paths) = setup();
        fs::create_dir_all(paths.sideload_library().parent().unwrap()).unwrap();
        fs::copy(&src, paths.sideload_library()).unwrap();
        let before = fs::read_to_string(paths.sideload_library()).unwrap();
        // Heroic's own file is already in the shape this module writes.
        assert!(before.starts_with("{\n\t\"games\": ["), "unexpected library shape");
        save_game(&paths, &spec("brand-new", "/g/x.exe", SideloadPlatform::Windows)).unwrap();
        let after = fs::read_to_string(paths.sideload_library()).unwrap();
        let cut = before.rfind("\n\t]").expect("games array end");
        assert!(after.starts_with(&before[..cut]), "existing entries changed");
        assert!(after.ends_with("\n\t]\n}"));
    }

    #[test]
    fn platform_names_round_trip() {
        for p in [SideloadPlatform::Linux, SideloadPlatform::Windows, SideloadPlatform::Browser] {
            assert_eq!(SideloadPlatform::from_install_platform(p.install_platform()), Some(p));
        }
        assert_eq!(SideloadPlatform::from_install_platform("Win32"), Some(SideloadPlatform::Windows));
        assert_eq!(SideloadPlatform::from_install_platform("nonsense"), None);
    }
}
