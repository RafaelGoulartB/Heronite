//! Heroic's game settings files, read and written the way `GameConfigV0` and
//! `GlobalConfigV0` do (Linux):
//!
//! * `<root>/config.json` -> `{ "defaultSettings": {..}, "version": "v0" }`
//!   (the global defaults every game inherits; `JSON.stringify(.., null, 2)`).
//! * `<root>/GamesConfig/<appName>.json` ->
//!   `{ "<appName>": {..}, "version": "v0", "explicit": true }` (same format).
//!
//! Writes touch only the key being changed (read-modify-write of the raw
//! document), so every other key and its order survive. A game without a
//! file gets the full default set Heroic would write on its first change.
//! Global changes are mirrored into `store/config.json` -> `settings` when
//! Heroic's own copy exists there (it reads it at window creation).

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Map, Value};

use crate::json_store::{self, INDENT_2};
use crate::paths::HeroicPaths;

pub type Settings = Map<String, Value>;

/// `<root>/config.json`.
pub fn global_config_path(paths: &HeroicPaths) -> PathBuf {
    paths.root.join("config.json")
}

/// `defaultWinePrefixDir` of Heroic (`~/Games/Heroic/Prefixes`).
pub fn default_wine_prefix_dir(home: &str) -> String {
    format!("{}/Games/Heroic/Prefixes", home.trim_end_matches('/'))
}

/// `sharedWinePrefix`.
pub fn shared_wine_prefix(home: &str) -> String {
    format!("{}/shared", default_wine_prefix_dir(home))
}

/// `GlobalConfigV0.getFactoryDefaults()` on Linux (not Flatpak). `wineVersion`
/// is left out: Heroic fills it with the first Wine it finds, which only the
/// Wine manager knows.
pub fn factory_defaults(home: &str) -> Settings {
    let home = home.trim_end_matches('/');
    let prefixes = default_wine_prefix_dir(home);
    // Heroic's literal, in its order; the path-like values are filled in below.
    let mut out: Settings = serde_json::from_str(FACTORY_JSON).expect("valid factory defaults");
    let set = |out: &mut Settings, k: &str, v: String| {
        out.insert(k.to_owned(), Value::String(v));
    };
    set(&mut out, "defaultInstallPath", format!("{home}/Games/Heroic"));
    set(&mut out, "defaultSteamPath", format!("{home}/.steam/steam"));
    set(&mut out, "defaultWinePrefix", prefixes.clone());
    set(&mut out, "defaultWinePrefixDir", prefixes);
    set(&mut out, "winePrefix", shared_wine_prefix(home));
    out
}

const FACTORY_JSON: &str = r#"{
  "analyticsOptIn": false,
  "checkUpdatesInterval": 10,
  "enableUpdates": false,
  "addDesktopShortcuts": false,
  "addStartMenuShortcuts": false,
  "autoInstallDxvk": true,
  "autoInstallVkd3d": true,
  "autoInstallDxvkNvapi": true,
  "addSteamShortcuts": false,
  "preferSystemLibs": false,
  "checkForUpdatesOnStartup": true,
  "autoUpdateGames": false,
  "customWinePaths": [],
  "defaultInstallPath": "",
  "libraryTopSection": "disabled",
  "defaultSteamPath": "",
  "defaultWinePrefix": "",
  "defaultWinePrefixDir": "",
  "hideChangelogsOnStartup": false,
  "language": "en",
  "maxWorkers": 0,
  "minimizeOnLaunch": false,
  "nvidiaPrime": false,
  "enviromentOptions": [],
  "wrapperOptions": [],
  "showFps": false,
  "useGameMode": false,
  "wineCrossoverBottle": "Heroic",
  "winePrefix": "",
  "enableEsync": true,
  "enableFsync": true,
  "enableMsync": false,
  "enableWineWayland": false,
  "enableHDR": false,
  "enableWoW64": false,
  "eacRuntime": true,
  "gamepadRepeatDelay": 50,
  "gamepadInitialRepeatDelay": 300,
  "battlEyeRuntime": true,
  "framelessWindow": false,
  "hideWindowOnProtocolLaunch": false,
  "beforeLaunchScriptPath": "",
  "afterLaunchScriptPath": "",
  "disableUMU": false,
  "verboseLogs": true,
  "downloadProtonToSteam": false,
  "advertiseAvxForRosetta": false,
  "noTrayIcon": false,
  "showValveProton": false,
  "steamGridDbApiKey": "",
  "disableGOGPresence": false
}"#;

pub fn expand_tilde(s: &str, home: &str) -> String {
    if s.contains('~') {
        s.replacen('~', home, 1)
    } else {
        s.to_string()
    }
}

/// `GlobalConfig.getSettings()`: factory defaults overlaid with the file's
/// `defaultSettings` (what the UI shows).
pub fn load_global(paths: &HeroicPaths, home: &str) -> Settings {
    let mut out = factory_defaults(home);
    let file = json_store::read_value(&global_config_path(paths)).unwrap_or(Value::Null);
    if let Some(Value::Object(saved)) = file.get("defaultSettings") {
        for (k, v) in saved {
            out.insert(k.clone(), v.clone());
        }
    }
    // Old configs only have `defaultWinePrefix`.
    let missing_dir = saved_str(&out, "defaultWinePrefixDir").is_none();
    if missing_dir {
        if let Some(old) = saved_str(&out, "defaultWinePrefix") {
            out.insert("defaultWinePrefixDir".into(), Value::String(old));
        }
    }
    if let Some(prefix) = saved_str(&out, "winePrefix") {
        out.insert("winePrefix".into(), Value::String(expand_tilde(&prefix, home)));
    } else {
        out.insert("winePrefix".into(), Value::String(shared_wine_prefix(home)));
    }
    out
}

fn saved_str(m: &Settings, key: &str) -> Option<String> {
    match m.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Keys `GameConfigV0.getSettings` copies from the global settings, in its
/// object-literal order.
const INHERITED_KEYS: [&str; 24] = [
    "autoInstallDxvk",
    "autoInstallDxvkNvapi",
    "autoInstallVkd3d",
    "DXVKFpsCap",
    "preferSystemLibs",
    "autoSyncSaves",
    "enableEsync",
    "enableMsync",
    "enableFSR",
    "enableFsync",
    "enableWineWayland",
    "enableHDR",
    "enableWoW64",
    "enableDXVKFpsLimit",
    "maxSharpness",
    "launcherArgs",
    "nvidiaPrime",
    "offlineMode",
    "enviromentOptions",
    "wrapperOptions",
    "savesPath",
    "showFps",
    "showMangohud",
    "targetExe",
];
const INHERITED_KEYS_2: [&str; 5] =
    ["useGameMode", "battlEyeRuntime", "eacRuntime", "beforeLaunchScriptPath", "afterLaunchScriptPath"];
// `heroniteSharedFolders` is Heronite's ("Folders visible to Proton").
const INHERITED_KEYS_3: [&str; 4] = ["gamescope", "verboseLogs", "advertiseAvxForRosetta", "heroniteSharedFolders"];

/// The defaults of `GameConfigV0.getSettings()` before the game's own section
/// is laid over them. `is_new_file`: the game has no config file yet
/// (`disableUMU: false` is only set then).
pub fn default_game_settings(global: &Settings, is_new_file: bool, home: &str) -> Settings {
    let mut out = Settings::new();
    let copy = |out: &mut Settings, key: &str| {
        if let Some(v) = global.get(key) {
            out.insert(key.to_owned(), v.clone());
        }
    };
    for k in INHERITED_KEYS {
        copy(&mut out, k);
    }
    for k in INHERITED_KEYS_2 {
        copy(&mut out, k);
        if k == "eacRuntime" {
            out.insert("language".into(), Value::String(String::new()));
        }
    }
    for k in INHERITED_KEYS_3 {
        copy(&mut out, k);
    }
    out.insert("enableQuickSavesMenu".into(), Value::Bool(false));
    out.insert("steamRuntime".into(), Value::Bool(false));
    if is_new_file {
        out.insert("disableUMU".into(), Value::Bool(false));
    }
    if let Some(v) = global.get("wineVersion") {
        out.insert("wineVersion".into(), v.clone());
    }
    let prefix = saved_str(global, "winePrefix").unwrap_or_else(|| shared_wine_prefix(home));
    out.insert("winePrefix".into(), Value::String(prefix));
    out
}

fn read_game_section(paths: &HeroicPaths, app_name: &str) -> (bool, Settings) {
    let path = paths.game_config(app_name);
    let existed = path.exists();
    let doc = json_store::read_value(&path).unwrap_or(Value::Null);
    let section = match doc.get(app_name) {
        Some(Value::Object(o)) => o.clone(),
        _ => Settings::new(),
    };
    (existed, section)
}

/// Effective settings of a game: global defaults overlaid with the game's
/// section (`GameConfigV0.getSettings()`).
pub fn load_game(paths: &HeroicPaths, app_name: &str, home: &str) -> Settings {
    let global = load_global(paths, home);
    let (existed, section) = read_game_section(paths, app_name);
    let mut out = default_game_settings(&global, !existed, home);
    for (k, v) in section {
        out.insert(k, v);
    }
    if let Some(Value::String(p)) = out.get("winePrefix") {
        let fixed = expand_tilde(p, home);
        out.insert("winePrefix".into(), Value::String(fixed));
    }
    out
}

/// Only the keys the game's file sets itself (to tell overrides from
/// inherited values).
pub fn game_overrides(paths: &HeroicPaths, app_name: &str) -> Settings {
    read_game_section(paths, app_name).1
}

/// `GameConfig.setSetting` for one or more keys of one game.
pub fn set_game_settings(
    paths: &HeroicPaths,
    app_name: &str,
    home: &str,
    changes: &[(String, Value)],
) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let path = paths.game_config(app_name);
    let new_file_defaults = if path.exists() {
        None
    } else {
        Some(default_game_settings(&load_global(paths, home), true, home))
    };
    json_store::update_value_indented(&path, INDENT_2, |doc| {
        let obj = doc.as_object_mut().expect("update_value gives an object or replaces it");
        if obj.is_empty() {
            // New file: Heroic writes the whole effective configuration.
            obj.insert(app_name.to_owned(), Value::Object(new_file_defaults.unwrap_or_default()));
            obj.insert("version".into(), Value::String("v0".into()));
            obj.insert("explicit".into(), Value::Bool(true));
        }
        if !matches!(obj.get(app_name), Some(Value::Object(_))) {
            obj.insert(app_name.to_owned(), Value::Object(Settings::new()));
        }
        if !obj.contains_key("version") {
            obj.insert("version".into(), Value::String("v0".into()));
        }
        let Some(Value::Object(section)) = obj.get_mut(app_name) else { unreachable!() };
        for (k, v) in changes {
            section.insert(k.clone(), v.clone());
        }
        Ok(())
    })
}

/// An emulated game that became an executable game: the emulator's
/// arguments the import copied into the game's `launcherArgs` (`-batch
/// "{InstallDir}/game.cue"`...) would be passed to the new program. Clears
/// them when they are still `emulator_args`; arguments the user changed stay.
pub fn forget_emulator_args(paths: &HeroicPaths, app_name: &str, emulator_args: &str) -> Result<()> {
    let path = paths.game_config(app_name);
    if emulator_args.trim().is_empty() || !path.exists() {
        return Ok(());
    }
    if read_game_section(paths, app_name).1.get("launcherArgs").and_then(Value::as_str) != Some(emulator_args) {
        return Ok(());
    }
    json_store::update_value_indented(&path, INDENT_2, |doc| {
        if let Some(Value::Object(section)) = doc.get_mut(app_name) {
            section.insert("launcherArgs".into(), Value::String(String::new()));
        }
        Ok(())
    })
}

/// [`set_game_settings`] for a single key.
pub fn set_game_setting(
    paths: &HeroicPaths,
    app_name: &str,
    home: &str,
    key: &str,
    value: Value,
) -> Result<()> {
    set_game_settings(paths, app_name, home, &[(key.to_owned(), value)])
}

/// `GlobalConfig.setSetting` for one or more keys.
pub fn set_global_settings(paths: &HeroicPaths, home: &str, changes: &[(String, Value)]) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    json_store::update_value_indented(&global_config_path(paths), INDENT_2, |doc| {
        let obj = doc.as_object_mut().expect("object");
        if !matches!(obj.get("defaultSettings"), Some(Value::Object(_))) {
            obj.insert("defaultSettings".into(), Value::Object(factory_defaults(home)));
        }
        if !obj.contains_key("version") {
            obj.insert("version".into(), Value::String("v0".into()));
        }
        let Some(Value::Object(section)) = obj.get_mut("defaultSettings") else { unreachable!() };
        for (k, v) in changes {
            section.insert(k.clone(), v.clone());
        }
        Ok(())
    })?;
    // Heroic keeps a copy in its electron-store (read when the window opens).
    let store = paths.config();
    if store.exists() {
        json_store::update_value(&store, |doc| {
            if let Some(Value::Object(copy)) = doc.get_mut("settings") {
                for (k, v) in changes {
                    copy.insert(k.clone(), v.clone());
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// [`set_global_settings`] for a single key.
pub fn set_global_setting(paths: &HeroicPaths, home: &str, key: &str, value: Value) -> Result<()> {
    set_global_settings(paths, home, &[(key.to_owned(), value)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn setup() -> (tempfile::TempDir, HeroicPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        fs::create_dir_all(paths.games_config_dir()).unwrap();
        (dir, paths)
    }

    const GLOBAL: &str = "{\n  \"defaultSettings\": {\n    \"useGameMode\": true,\n    \"winePrefix\": \"~/pfx\",\n    \"wineVersion\": {\n      \"bin\": \"/w/wine\",\n      \"name\": \"W\",\n      \"type\": \"wine\"\n    },\n    \"zzCustom\": [\n      1,\n      2\n    ]\n  },\n  \"version\": \"v0\"\n}";

    #[test]
    fn global_load_merges_factory_defaults_and_expands_tilde() {
        let (_d, paths) = setup();
        fs::write(global_config_path(&paths), GLOBAL).unwrap();
        let g = load_global(&paths, "/home/u");
        assert_eq!(g["useGameMode"], true);
        assert_eq!(g["winePrefix"], "/home/u/pfx");
        assert_eq!(g["enableFsync"], true); // factory default
        assert_eq!(g["defaultInstallPath"], "/home/u/Games/Heroic");
        assert_eq!(g["wineVersion"]["name"], "W");
        // Missing file: factory defaults only.
        let (_d2, empty) = setup();
        let g = load_global(&empty, "/home/u");
        assert_eq!(g["winePrefix"], "/home/u/Games/Heroic/Prefixes/shared");
        assert!(g.get("wineVersion").is_none());
    }

    #[test]
    fn global_write_keeps_everything_else_byte_identical() {
        let (_d, paths) = setup();
        fs::write(global_config_path(&paths), GLOBAL).unwrap();
        set_global_setting(&paths, "/home/u", "useGameMode", json!(false)).unwrap();
        let text = fs::read_to_string(global_config_path(&paths)).unwrap();
        assert_eq!(text, GLOBAL.replace("\"useGameMode\": true", "\"useGameMode\": false"));
        // New key is appended inside defaultSettings, 2-space indent.
        set_global_setting(&paths, "/home/u", "maxWorkers", json!(4)).unwrap();
        let text = fs::read_to_string(global_config_path(&paths)).unwrap();
        assert!(text.contains("    ],\n    \"maxWorkers\": 4\n  },\n  \"version\": \"v0\"\n}"), "{text}");
        assert!(!text.ends_with('\n'));
    }

    #[test]
    fn global_write_mirrors_into_the_electron_store_copy() {
        let (_d, paths) = setup();
        fs::write(global_config_path(&paths), GLOBAL).unwrap();
        fs::create_dir_all(paths.config().parent().unwrap()).unwrap();
        let store = "{\n\t\"language\": \"en\",\n\t\"settings\": {\n\t\t\"maxWorkers\": 0\n\t}\n}";
        fs::write(paths.config(), store).unwrap();
        set_global_setting(&paths, "/home/u", "maxWorkers", json!(6)).unwrap();
        let text = fs::read_to_string(paths.config()).unwrap();
        assert_eq!(text, store.replace("\"maxWorkers\": 0", "\"maxWorkers\": 6"));
        // No `settings` object there: the store is left alone.
        let before = fs::read_to_string(paths.config()).unwrap();
        fs::write(paths.config(), "{\n\t\"language\": \"en\"\n}").unwrap();
        set_global_setting(&paths, "/home/u", "maxWorkers", json!(7)).unwrap();
        assert_eq!(fs::read_to_string(paths.config()).unwrap(), "{\n\t\"language\": \"en\"\n}");
        drop(before);
    }

    #[test]
    fn new_game_file_gets_defaults_then_the_change() {
        let (_d, paths) = setup();
        fs::write(global_config_path(&paths), GLOBAL).unwrap();
        set_game_setting(&paths, "g1", "/home/u", "launcherArgs", json!("-windowed")).unwrap();
        let text = fs::read_to_string(paths.game_config("g1")).unwrap();
        assert!(text.starts_with("{\n  \"g1\": {\n    "), "{text}");
        assert!(text.ends_with("  },\n  \"version\": \"v0\",\n  \"explicit\": true\n}"), "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["g1"]["launcherArgs"], "-windowed");
        assert_eq!(v["g1"]["useGameMode"], true); // inherited from the defaults
        assert_eq!(v["g1"]["disableUMU"], false);
        assert_eq!(v["g1"]["winePrefix"], "/home/u/pfx");
        assert_eq!(v["g1"]["wineVersion"]["bin"], "/w/wine");
        assert!(v["g1"].get("zzCustom").is_none());
    }

    #[test]
    fn existing_game_file_changes_only_the_key() {
        let (_d, paths) = setup();
        let original = "{\n  \"g2\": {\n    \"useGameMode\": false,\n    \"custom\": {\n      \"a\": [\n        1\n      ]\n    },\n    \"winePrefix\": \"/mine\"\n  },\n  \"version\": \"v0\",\n  \"explicit\": true\n}";
        fs::write(paths.game_config("g2"), original).unwrap();
        set_game_setting(&paths, "g2", "/h", "useGameMode", json!(true)).unwrap();
        let text = fs::read_to_string(paths.game_config("g2")).unwrap();
        assert_eq!(text, original.replace("\"useGameMode\": false", "\"useGameMode\": true"));
        set_game_setting(&paths, "g2", "/h", "verboseLogs", json!(false)).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(paths.game_config("g2")).unwrap()).unwrap();
        assert_eq!(v["g2"]["verboseLogs"], false);
        assert_eq!(v["g2"]["custom"]["a"][0], 1);
        assert_eq!(v["explicit"], true);
        let keys: Vec<_> = v["g2"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["useGameMode", "custom", "winePrefix", "verboseLogs"]);
        // Other games' files are not touched.
        assert!(!paths.game_config("g3").exists());
    }

    #[test]
    fn load_game_overlays_the_section_on_the_defaults() {
        let (_d, paths) = setup();
        fs::write(global_config_path(&paths), GLOBAL).unwrap();
        fs::write(
            paths.game_config("g1"),
            "{\"g1\":{\"useGameMode\":false,\"winePrefix\":\"~/mine\"},\"version\":\"v0\",\"explicit\":true}",
        )
        .unwrap();
        let s = load_game(&paths, "g1", "/home/u");
        assert_eq!(s["useGameMode"], false);
        assert_eq!(s["winePrefix"], "/home/u/mine");
        assert_eq!(s["wineVersion"]["name"], "W");
        assert!(s.get("disableUMU").is_none()); // existing file: left undefined
        assert_eq!(game_overrides(&paths, "g1").len(), 2);
        let n = load_game(&paths, "nope", "/home/u");
        assert_eq!(n["disableUMU"], false);
        assert_eq!(n["useGameMode"], true);
    }

    /// Against a copy of the real data (`HERONITE_FIXTURE=/tmp/heronite-fixture`):
    /// parsing and re-serializing every game file with Heroic's format must
    /// give back the same bytes.
    #[test]
    fn real_game_files_reserialize_identically() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE").map(PathBuf::from) else {
            eprintln!("HERONITE_FIXTURE not set; skipping");
            return;
        };
        let mut checked = 0;
        for entry in fs::read_dir(root.join("GamesConfig")).into_iter().flatten().flatten() {
            // Heroic also keeps `<app>-lastPlay.log` files there.
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = fs::read_to_string(entry.path()).unwrap();
            let value: Value = serde_json::from_str(&text).unwrap();
            let again = String::from_utf8(json_store::to_bytes_indented(&value, INDENT_2).unwrap()).unwrap();
            assert_eq!(again, text, "{}", entry.path().display());
            checked += 1;
        }
        eprintln!("checked {checked} GamesConfig files");
    }
}
