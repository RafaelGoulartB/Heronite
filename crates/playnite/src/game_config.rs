//! `applyLauncherArgs` of `import.ts`: sets `launcherArgs` in
//! `GamesConfig/<appName>.json` the way Heroic's `GameConfigV0` flushes it
//! (`JSON.stringify(.., null, 2)`, explicit mode, settings = global defaults
//! overlaid with the game's own section).

use std::fs;

use anyhow::{Context, Result};
use heronite_data::{json_store, HeroicPaths};
use serde_json::{Map, Value};

/// Keys `GameConfigV0.getSettings` copies from the global settings, in its
/// object-literal order.
const DEFAULT_KEYS: [&str; 24] = [
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
const DEFAULT_KEYS_2: [&str; 5] = ["useGameMode", "battlEyeRuntime", "eacRuntime", "beforeLaunchScriptPath", "afterLaunchScriptPath"];
const DEFAULT_KEYS_3: [&str; 3] = ["gamescope", "verboseLogs", "advertiseAvxForRosetta"];

fn default_settings(global: &Map<String, Value>, is_new_file: bool, home: &str) -> Map<String, Value> {
    let mut out = Map::new();
    let copy = |out: &mut Map<String, Value>, key: &str| {
        if let Some(v) = global.get(key) {
            out.insert(key.to_owned(), v.clone());
        }
    };
    for k in DEFAULT_KEYS {
        copy(&mut out, k);
    }
    for k in DEFAULT_KEYS_2 {
        copy(&mut out, k);
        if k == "eacRuntime" {
            out.insert("language".into(), Value::String(String::new()));
        }
    }
    for k in DEFAULT_KEYS_3 {
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
    let prefix = global
        .get("winePrefix")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{home}/Games/Heroic/Prefixes/shared"));
    out.insert("winePrefix".into(), Value::String(prefix));
    out
}

/// Writes `launcherArgs` for `app_name`. No-op for empty args.
pub fn apply_launcher_args(paths: &HeroicPaths, app_name: &str, launcher_args: Option<&str>, home: &str) -> Result<()> {
    let Some(args) = launcher_args.filter(|a| !a.is_empty()) else { return Ok(()) };
    let path = paths.game_config(app_name);
    let global_doc = json_store::read_value(&paths.root.join("config.json")).unwrap_or(Value::Null);
    let global = global_doc.get("defaultSettings").and_then(Value::as_object).cloned().unwrap_or_default();

    let existing = if path.exists() {
        Some(json_store::read_value(&path).with_context(|| format!("reading {}", path.display()))?)
    } else {
        None
    };
    let mut settings = default_settings(&global, existing.is_none(), home);
    if let Some(section) = existing.as_ref().and_then(|d| d.get(app_name)).and_then(Value::as_object) {
        for (k, v) in section {
            settings.insert(k.clone(), v.clone());
        }
    }
    settings.insert("launcherArgs".into(), Value::String(args.to_owned()));

    let mut doc = Map::new();
    doc.insert(app_name.to_owned(), Value::Object(settings));
    doc.insert("version".into(), Value::String("v0".into()));
    doc.insert("explicit".into(), Value::Bool(true));

    fs::create_dir_all(paths.games_config_dir())?;
    // Heroic writes with 2-space indentation and no trailing newline.
    let bytes = serde_json::to_vec_pretty(&Value::Object(doc))?;
    let tmp = path.with_file_name(format!(".{app_name}.heronite-rs.tmp"));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_new_and_patches_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        fs::write(
            paths.root.join("config.json"),
            r#"{"defaultSettings":{"useGameMode":true,"winePrefix":"/p","enableEsync":true,"unrelated":1},"version":"v0"}"#,
        )
        .unwrap();
        apply_launcher_args(&paths, "g1", Some("steam://rungameid/1"), "/home/u").unwrap();
        let v = json_store::read_value(&paths.game_config("g1")).unwrap();
        assert_eq!(v["g1"]["launcherArgs"], "steam://rungameid/1");
        assert_eq!(v["g1"]["useGameMode"], true);
        assert_eq!(v["g1"]["disableUMU"], false);
        assert_eq!(v["g1"]["winePrefix"], "/p");
        assert!(v["g1"].get("unrelated").is_none());
        assert_eq!(v["version"], "v0");
        assert_eq!(v["explicit"], true);
        let text = fs::read_to_string(paths.game_config("g1")).unwrap();
        assert!(text.starts_with("{\n  \"g1\": {"));

        // existing game section wins over defaults, disableUMU not added
        fs::write(paths.game_config("g2"), r#"{"g2":{"useGameMode":false,"custom":5},"version":"v0","explicit":true}"#).unwrap();
        apply_launcher_args(&paths, "g2", Some("-x"), "/home/u").unwrap();
        let v = json_store::read_value(&paths.game_config("g2")).unwrap();
        assert_eq!(v["g2"]["useGameMode"], false);
        assert_eq!(v["g2"]["custom"], 5);
        assert!(v["g2"].get("disableUMU").is_none());
        assert_eq!(v["g2"]["launcherArgs"], "-x");

        apply_launcher_args(&paths, "g3", None, "/h").unwrap();
        assert!(!paths.game_config("g3").exists());
    }
}
