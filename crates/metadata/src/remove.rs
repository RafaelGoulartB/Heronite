//! "Remove from Collection" (port of `local-library/remove.ts` plus the
//! renderer part of `CollectionCard.handleRemoveFromCollection`).
//!
//! Only `sideload` games can be removed. The game does not get uninstalled:
//! its entry in `sideload_apps/library.json`, its Collection data (overlay
//! meta, sessions, metadata, art, playtime) and its recent/favourite/hidden
//! entries are deleted. Desktop and start menu shortcuts go too.
//!
//! Not ported: removing the game from Steam's non-Steam shortcuts
//! (`removeNonSteamGame`).

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use heronite_data::models::{LocalMetaFile, SessionsFile};
use heronite_data::paths::HeroicPaths;
use heronite_data::{config_games, json_store};
use serde_json::Value;

use crate::art::remove_all_collection_game_art;
use crate::images::remove_metadata_art;
use crate::store::delete_game_metadata;

/// Message of the refusal for stores other than the Local Collection.
pub const ONLY_LOCAL_MESSAGE: &str = "Only Local Collection games can be removed from here";

/// Where the shortcuts live (`app.getPath('desktop')` and `$HOME`).
#[derive(Debug, Clone)]
pub struct ShortcutDirs {
    pub desktop: PathBuf,
    pub applications: PathBuf,
}

impl ShortcutDirs {
    /// `$HOME/.local/share/applications` and the XDG desktop directory.
    pub fn for_home(home: &Path) -> Self {
        Self { desktop: desktop_dir(home), applications: home.join(".local/share/applications") }
    }

    pub fn detect() -> Self {
        match dirs_home() {
            Some(home) => Self::for_home(&home),
            None => Self { desktop: PathBuf::new(), applications: PathBuf::new() },
        }
    }

    /// `shortcutFiles(title)` on Linux: `[desktopFile, menuFile]`.
    pub fn files(&self, title: &str) -> Option<[PathBuf; 2]> {
        if self.desktop.as_os_str().is_empty() || self.applications.as_os_str().is_empty() {
            return None;
        }
        let name = format!("{}.desktop", sanitize_filename(title));
        Some([self.desktop.join(&name), self.applications.join(name)])
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// `XDG_DESKTOP_DIR` of `~/.config/user-dirs.dirs`, else `~/Desktop`.
pub fn desktop_dir(home: &Path) -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    parse_desktop_dir(&text, home).unwrap_or_else(|| home.join("Desktop"))
}

fn parse_desktop_dir(text: &str, home: &Path) -> Option<PathBuf> {
    let line = text.lines().map(str::trim).find(|l| l.starts_with("XDG_DESKTOP_DIR="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => PathBuf::from(format!("{}{rest}", home.display())),
        None => PathBuf::from(value),
    };
    (path.is_absolute() && path != home).then_some(path)
}

/// `sanitize-filename`: drops path separators, reserved characters and
/// control characters, trailing dots/spaces, Windows reserved names; at most
/// 255 bytes.
pub fn sanitize_filename(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| !matches!(c, '/' | '?' | '<' | '>' | '\\' | ':' | '*' | '|' | '"') && !c.is_control())
        .collect();
    let mut name = cleaned.trim_end_matches(['.', ' ']).to_string();
    if name == "." || name == ".." {
        name.clear();
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        name.clear();
    }
    while name.len() > 255 {
        name.pop();
    }
    name
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RemoveOutcome {
    /// The game had an entry in `sideload_apps/library.json`.
    pub was_in_library: bool,
    /// Shortcut files that existed and were deleted.
    pub removed_shortcuts: Vec<PathBuf>,
}

/// `removeCollectionGame` + the renderer follow-up (hidden, favourites).
pub fn remove_collection_game(
    paths: &HeroicPaths,
    shortcuts: &ShortcutDirs,
    app_name: &str,
    runner: &str,
) -> Result<RemoveOutcome> {
    if runner != "sideload" {
        bail!(ONLY_LOCAL_MESSAGE);
    }
    let mut outcome = RemoveOutcome::default();

    // Shortcuts first (they are named after the title in the library).
    let library = json_store::read_value(&paths.sideload_library())?;
    let title = library
        .get("games")
        .and_then(Value::as_array)
        .and_then(|games| games.iter().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name)))
        .and_then(|g| g.get("title").and_then(Value::as_str))
        .map(str::to_owned);
    if let Some(title) = &title {
        outcome.was_in_library = true;
        if let Some(files) = shortcuts.files(title) {
            for file in files {
                if file.is_file() {
                    match std::fs::remove_file(&file) {
                        Ok(()) => outcome.removed_shortcuts.push(file),
                        Err(e) => log::warn!("Could not remove shortcut {}: {e}", file.display()),
                    }
                }
            }
        }
        json_store::update_value(&paths.sideload_library(), |doc| {
            if let Some(games) = doc.get_mut("games").and_then(Value::as_array_mut) {
                games.retain(|g| g.get("app_name").and_then(Value::as_str) != Some(app_name));
            }
            Ok(())
        })?;
    }

    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        file.games.shift_remove(app_name);
        Ok(())
    })?;
    json_store::update::<SessionsFile, _>(&paths.sessions(), |file| {
        file.sessions.shift_remove(app_name);
        Ok(())
    })?;
    remove_all_collection_game_art(paths, runner, app_name)?;
    delete_game_metadata(paths, runner, app_name)?;
    remove_metadata_art(paths, runner, app_name);
    json_store::update_value(&paths.timestamps(), |doc| {
        if let Some(map) = doc.as_object_mut() {
            map.shift_remove(app_name);
        }
        Ok(())
    })?;
    config_games::remove_recent(paths, app_name)?;
    // Renderer: `hiddenGames.remove`, `favouriteGames.remove`.
    config_games::unhide_game(paths, app_name)?;
    config_games::remove_favourite(paths, app_name)?;
    log::info!("Removed {app_name} from Collection");
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    struct Env {
        _dir: tempfile::TempDir,
        paths: HeroicPaths,
        shortcuts: ShortcutDirs,
    }

    fn write(path: &Path, value: Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path().join("heroic"));
        let home = dir.path().join("home");
        let shortcuts = ShortcutDirs { desktop: home.join("Desktop"), applications: home.join(".local/share/applications") };
        write(
            &paths.sideload_library(),
            json!({"version":"v0","games":[
                {"runner":"sideload","app_name":"a","title":"Game: A?","is_installed":true,"install":{}},
                {"runner":"sideload","app_name":"b","title":"B","is_installed":false,"install":{}}
            ]}),
        );
        write(
            &paths.local_meta(),
            json!({"games":{"a":{"appName":"a","runner":"sideload","title":"A"},"b":{"appName":"b","runner":"sideload","title":"B"}}}),
        );
        write(
            &paths.sessions(),
            json!({"sessions":{"a":[{"startedAt":"2025-01-01T00:00:00.000Z","endedAt":"2025-01-01T00:01:00.000Z","elapsedSeconds":60,"source":"heroic"}],"b":[]}}),
        );
        write(
            &paths.metadata(),
            json!({"games":{"sideload:a":{"appName":"a","runner":"sideload","title":"A"},"sideload:b":{"appName":"b","runner":"sideload"}}}),
        );
        write(&paths.art(), json!({"games":{"sideload:a":{"coverUrl":"https://x/y.png"}}}));
        write(&paths.timestamps(), json!({"a":{"firstPlayed":"","lastPlayed":"","totalPlayed":3},"b":{"totalPlayed":1}}));
        write(
            &paths.config(),
            json!({"games":{
                "recent":[{"appName":"a","title":"A"},{"appName":"b","title":"B"}],
                "favourites":[{"appName":"a","title":"A"}],
                "hidden":[{"appName":"a","title":"A"},{"appName":"b","title":"B"}]
            },"other":1}),
        );
        fs::create_dir_all(paths.metadata_art_dir().join("sideload_a")).unwrap();
        fs::write(paths.metadata_art_dir().join("sideload_a/cover.png"), b"x").unwrap();
        fs::create_dir_all(&shortcuts.desktop).unwrap();
        fs::create_dir_all(&shortcuts.applications).unwrap();
        fs::write(shortcuts.desktop.join("Game A.desktop"), "[Desktop Entry]").unwrap();
        fs::write(shortcuts.applications.join("Game A.desktop"), "[Desktop Entry]").unwrap();
        fs::write(shortcuts.applications.join("B.desktop"), "[Desktop Entry]").unwrap();
        Env { _dir: dir, paths, shortcuts }
    }

    #[test]
    fn removes_everything_of_the_game_and_nothing_else() {
        let e = env();
        let out = remove_collection_game(&e.paths, &e.shortcuts, "a", "sideload").unwrap();
        assert!(out.was_in_library);
        assert_eq!(out.removed_shortcuts.len(), 2);

        let lib = json_store::read_value(&e.paths.sideload_library()).unwrap();
        let names: Vec<_> = lib["games"].as_array().unwrap().iter().map(|g| g["app_name"].as_str().unwrap()).collect();
        assert_eq!(names, ["b"]);
        assert_eq!(lib["version"], "v0");

        let meta: LocalMetaFile = json_store::read(&e.paths.local_meta()).unwrap();
        assert!(!meta.games.contains_key("a") && meta.games.contains_key("b"));
        let sessions: SessionsFile = json_store::read(&e.paths.sessions()).unwrap();
        assert!(!sessions.sessions.contains_key("a") && sessions.sessions.contains_key("b"));
        let md = json_store::read_value(&e.paths.metadata()).unwrap();
        assert!(md["games"].get("sideload:a").is_none() && md["games"].get("sideload:b").is_some());
        let art = json_store::read_value(&e.paths.art()).unwrap();
        let cover = art["games"].get("sideload:a").and_then(|a| a.get("coverUrl"));
        assert!(cover.is_none_or(Value::is_null));
        assert!(!e.paths.metadata_art_dir().join("sideload_a").exists());
        let ts = json_store::read_value(&e.paths.timestamps()).unwrap();
        assert!(ts.get("a").is_none() && ts.get("b").is_some());
        let cfg = json_store::read_value(&e.paths.config()).unwrap();
        assert_eq!(cfg["games"]["recent"].as_array().unwrap().len(), 1);
        assert_eq!(cfg["games"]["favourites"].as_array().unwrap().len(), 0);
        assert_eq!(cfg["games"]["hidden"].as_array().unwrap().len(), 1);
        assert_eq!(cfg["other"], 1);

        assert!(!e.shortcuts.desktop.join("Game A.desktop").exists());
        assert!(!e.shortcuts.applications.join("Game A.desktop").exists());
        assert!(e.shortcuts.applications.join("B.desktop").exists(), "other shortcuts stay");
    }

    #[test]
    fn other_runners_are_refused_and_nothing_changes() {
        let e = env();
        let err = remove_collection_game(&e.paths, &e.shortcuts, "a", "legendary").unwrap_err();
        assert_eq!(err.to_string(), ONLY_LOCAL_MESSAGE);
        let lib = json_store::read_value(&e.paths.sideload_library()).unwrap();
        assert_eq!(lib["games"].as_array().unwrap().len(), 2);
        assert!(e.shortcuts.desktop.join("Game A.desktop").exists());
    }

    #[test]
    fn game_missing_from_the_library_still_cleans_the_overlay() {
        let e = env();
        remove_collection_game(&e.paths, &e.shortcuts, "a", "sideload").unwrap();
        // Second call: no library entry any more, no error.
        let out = remove_collection_game(&e.paths, &e.shortcuts, "a", "sideload").unwrap();
        assert!(!out.was_in_library);
        assert!(out.removed_shortcuts.is_empty());
    }

    #[test]
    fn sanitize_matches_sanitize_filename() {
        assert_eq!(sanitize_filename("Game: A?"), "Game A");
        assert_eq!(sanitize_filename("a/b\\c"), "abc");
        assert_eq!(sanitize_filename("Name. "), "Name");
        assert_eq!(sanitize_filename("con"), "");
        assert_eq!(sanitize_filename("LPT1.txt"), "");
        assert_eq!(sanitize_filename("Ação"), "Ação");
    }

    #[test]
    fn desktop_dir_from_user_dirs() {
        let home = Path::new("/home/u");
        assert_eq!(
            parse_desktop_dir("XDG_DESKTOP_DIR=\"$HOME/Área de Trabalho\"\n", home),
            Some(PathBuf::from("/home/u/Área de Trabalho"))
        );
        assert_eq!(parse_desktop_dir("XDG_DESKTOP_DIR=\"$HOME/\"\n", home), None);
        assert_eq!(parse_desktop_dir("", home), None);
    }
}
