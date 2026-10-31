//! Games the Epic Games Launcher installed on this PC (Windows), taken in
//! without downloading them again.
//!
//! The launcher keeps one `<guid>.item` file per installed game in
//! `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`, and the game's
//! manifest in `<game>\.egstore`. [`importable`] only reads them. [`import`]
//! runs `legendary import <app> <folder> --platform Windows --with-dlcs` for
//! each of them: legendary records the game in its own `installed.json`,
//! with the version the launcher installed (read from the game's
//! `.egstore`), and the game is legendary's from then on. Nothing of the
//! launcher's is written, moved or deleted.
//!
//! Not `legendary egl-sync`: it links the games to the launcher, so a later
//! sync takes the launcher's (older) version back over one Heronite updated,
//! and it may write manifests into the launcher's folder.
//!
//! It runs after each library refresh when the launcher has games
//! legendary does not know yet (so after the login too), and from the Epic
//! page's button.

use std::path::{Path, PathBuf};

use heronite_data::{json_store, DataPaths};
use heronite_i18n::{tr, trn};
use serde_json::Value;

use crate::cmd::Legendary;
use crate::library;
use crate::ops::OpResult;

/// One game of the launcher's `.item` files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherGame {
    pub app_name: String,
    pub title: String,
    pub install_location: String,
}

/// The launcher's manifests folder on this PC, when it is there (Windows).
pub fn manifests_dir() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let program_data = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    Some(program_data.join("Epic").join("EpicGamesLauncher").join("Data").join("Manifests")).filter(|d| d.is_dir())
}

fn text(doc: &Value, key: &str) -> String {
    doc.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The complete installs of base games listed in `dir` (DLC and installs
/// the launcher did not finish are left out, like legendary does).
pub fn launcher_games(dir: &Path) -> Vec<LauncherGame> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<LauncherGame> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("item")))
        .filter_map(|e| json_store::read_value(&e.path()).ok())
        .filter(|doc| {
            let (app, main) = (text(doc, "AppName"), text(doc, "MainGameAppName"));
            !app.is_empty() && (main.is_empty() || main == app) && doc.get("bIsIncompleteInstall").and_then(Value::as_bool) != Some(true)
        })
        .filter(|doc| {
            // legendary takes the version from the game's own manifest.
            let (location, guid) = (text(doc, "InstallLocation"), text(doc, "InstallationGuid"));
            !guid.is_empty() && Path::new(&location).join(".egstore").join(format!("{guid}.manifest")).is_file()
        })
        .map(|doc| LauncherGame {
            app_name: text(&doc, "AppName"),
            title: Some(text(&doc, "DisplayName")).filter(|t| !t.is_empty()).unwrap_or_else(|| text(&doc, "AppName")),
            install_location: text(&doc, "InstallLocation"),
        })
        .collect();
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

/// The launcher's games legendary does not have yet and the account owns
/// (its metadata is in legendary's folder). Reads files only.
pub fn importable(data_paths: &DataPaths) -> Vec<LauncherGame> {
    manifests_dir().map(|dir| importable_from(data_paths, &dir)).unwrap_or_default()
}

pub fn importable_from(data_paths: &DataPaths, dir: &Path) -> Vec<LauncherGame> {
    let installed = library::installed_games(data_paths);
    let metadata = library::metadata_dir(data_paths);
    launcher_games(dir)
        .into_iter()
        .filter(|g| !installed.contains_key(&g.app_name) && metadata.join(format!("{}.json", g.app_name)).is_file())
        .collect()
}

/// legendary's command line that takes `game` in.
pub fn import_args(game: &LauncherGame) -> Vec<String> {
    let args = ["import", &game.app_name, &game.install_location, "--platform", "Windows", "--with-dlcs"];
    args.into_iter().map(String::from).collect()
}

/// legendary takes in the launcher's games (see the module docs). Blocking.
pub fn import(data_paths: &DataPaths, legendary: &Legendary) -> OpResult {
    import_games(data_paths, legendary, &importable(data_paths)).1
}

/// Takes `games` in one by one: how many legendary took, and what to say.
fn import_games(data_paths: &DataPaths, legendary: &Legendary, games: &[LauncherGame]) -> (usize, OpResult) {
    let mut added = 0;
    let mut failed: Option<(String, String)> = None;
    for game in games {
        let why = match legendary.run(&import_args(game), &[]) {
            // legendary may exit with 0 after an error: its record tells.
            Ok(_) if library::installed_games(data_paths).contains_key(&game.app_name) => {
                added += 1;
                continue;
            }
            Ok(out) => {
                let why = out.error_lines();
                if why.is_empty() {
                    tr!("legendary exited with {code}", code = format!("{:?}", out.code))
                } else {
                    why
                }
            }
            Err(err) => format!("{err:#}"),
        };
        log::warn!("taking {} in from the Epic Games Launcher: {why}", game.app_name);
        failed.get_or_insert((game.title.clone(), why));
    }
    if added > 0 {
        if let Err(err) = library::write_library(data_paths) {
            log::warn!("rebuilding the library: {err:#}");
        }
    }
    log::info!("{added} game(s) taken in from the Epic Games Launcher");
    let r = match (added, failed) {
        (0, None) => OpResult::ok(tr!("No new games from the Epic Games Launcher")),
        (0, Some((title, error))) => {
            OpResult::err(tr!("{title} could not be taken in from the Epic Games Launcher: {error}", title, error))
        }
        (n, _) => {
            OpResult::ok(trn!(n, "{n} game taken in from the Epic Games Launcher", "{n} games taken in from the Epic Games Launcher"))
        }
    };
    (added, r)
}

/// After a refresh: takes the launcher's new games in, when it has any.
pub fn import_new(data_paths: &DataPaths, legendary: &Legendary) -> Option<OpResult> {
    let new = importable(data_paths);
    if new.is_empty() {
        return None;
    }
    log::info!("the Epic Games Launcher has {} game(s) to take in: {:?}", new.len(), new.iter().map(|g| &g.app_name).collect::<Vec<_>>());
    let (added, r) = import_games(data_paths, legendary, &new);
    // legendary took none of them: said once, in the log.
    if added == 0 {
        log::info!("{}", r.message);
        return None;
    }
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::{fake_tool, Reply};
    use serde_json::json;

    fn item(dir: &Path, guid: &str, doc: Value) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{guid}.item")), doc.to_string()).unwrap();
    }

    /// A launcher install of `app` in `<root>/<app>` with its manifest.
    fn launcher_install(root: &Path, manifests: &Path, app: &str, title: &str, extra: Value) {
        let guid = format!("GUID{app}");
        let folder = root.join(app);
        std::fs::create_dir_all(folder.join(".egstore")).unwrap();
        std::fs::write(folder.join(".egstore").join(format!("{guid}.manifest")), "x").unwrap();
        let mut doc = json!({
            "AppName": app, "MainGameAppName": app, "DisplayName": title,
            "InstallLocation": folder.to_string_lossy(), "InstallationGuid": guid, "bIsIncompleteInstall": false
        });
        for (k, v) in extra.as_object().unwrap() {
            doc[k] = v.clone();
        }
        item(manifests, &guid, doc);
    }

    #[test]
    fn complete_base_game_installs_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let manifests = dir.path().join("Manifests");
        launcher_install(dir.path(), &manifests, "Calluna", "Control", json!({}));
        launcher_install(dir.path(), &manifests, "Fish", "Hades", json!({}));
        launcher_install(dir.path(), &manifests, "Dlc1", "Control: AWE", json!({"MainGameAppName": "Calluna"}));
        launcher_install(dir.path(), &manifests, "Half", "Half", json!({"bIsIncompleteInstall": true}));
        // Its manifest is gone: legendary could not take it.
        launcher_install(dir.path(), &manifests, "Gone", "Gone", json!({}));
        std::fs::remove_dir_all(dir.path().join("Gone").join(".egstore")).unwrap();
        std::fs::write(manifests.join("broken.item"), "{").unwrap();
        let games = launcher_games(&manifests);
        assert_eq!(games.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Control", "Hades"]);
        assert_eq!(games[0].install_location, dir.path().join("Calluna").to_string_lossy());
    }

    #[test]
    fn only_owned_games_legendary_does_not_have_are_importable() {
        let dir = tempfile::tempdir().unwrap();
        let manifests = dir.path().join("Manifests");
        let data_paths = DataPaths::at(dir.path().join("h"));
        let cfg = data_paths.legendary_config_dir();
        std::fs::create_dir_all(cfg.join("metadata")).unwrap();
        for app in ["Calluna", "Fish"] {
            launcher_install(dir.path(), &manifests, app, app, json!({}));
            std::fs::write(cfg.join("metadata").join(format!("{app}.json")), "{}").unwrap();
        }
        launcher_install(dir.path(), &manifests, "NotOwned", "NotOwned", json!({}));
        std::fs::write(cfg.join("installed.json"), json!({"Fish": {"app_name": "Fish"}}).to_string()).unwrap();
        let names: Vec<String> = importable_from(&data_paths, &manifests).into_iter().map(|g| g.app_name).collect();
        assert_eq!(names, ["Calluna"]);
    }

    fn game(app: &str, folder: &str) -> LauncherGame {
        LauncherGame { app_name: app.into(), title: format!("{app} title"), install_location: folder.into() }
    }

    #[test]
    fn each_game_is_imported_with_its_folder_and_the_new_ones_counted() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        std::fs::create_dir_all(data_paths.legendary_config_dir()).unwrap();
        let tool = fake_tool(dir.path())
            .on(
                &["import", "Calluna", "*"],
                Reply::new().write_file("${LEGENDARY_CONFIG_PATH}/installed.json", r#"{"Calluna": {"app_name": "Calluna"}}"#),
            )
            // legendary logs the error and exits with 0.
            .otherwise(Reply::new().err("[cli] ERROR: Game executable could not be found"));
        let lg = Legendary::with_bin(tool.bin(), data_paths.legendary_config_dir());
        let games = [game("Calluna", r"D:\Epic Games\Control"), game("Fish", r"D:\Epic Games\Hades")];
        let (added, r) = import_games(&data_paths, &lg, &games);
        assert!(r.ok, "{}", r.message);
        assert_eq!((added, r.message.as_str()), (1, "1 game taken in from the Epic Games Launcher"));
        assert_eq!(import_args(&games[0]), ["import", "Calluna", r"D:\Epic Games\Control", "--platform", "Windows", "--with-dlcs"]);
        assert_eq!(tool.calls().len(), 2, "{:?}", tool.calls());
        assert!(data_paths.legendary_library().exists(), "the library is rebuilt");
        // Nothing to take in: nothing runs.
        assert_eq!(import_games(&data_paths, &lg, &[]).1.message, "No new games from the Epic Games Launcher");
    }

    #[test]
    fn an_import_that_takes_nothing_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let tool = fake_tool(dir.path()).otherwise(Reply::new().err("[cli] ERROR: Legendary is missing game metadata").exit(1));
        let lg = Legendary::with_bin(tool.bin(), data_paths.legendary_config_dir());
        let (added, r) = import_games(&data_paths, &lg, &[game("Calluna", r"D:\Control")]);
        assert_eq!(added, 0);
        assert!(!r.ok && r.message.contains("Calluna title") && r.message.contains("missing game metadata"), "{}", r.message);
    }

    /// Read-only look at this PC's launcher, if it has one:
    /// `cargo test -p heronite-epic egl::tests::this_pcs_launcher -- --nocapture`.
    #[test]
    fn this_pcs_launcher() {
        match manifests_dir() {
            Some(dir) => {
                let games = launcher_games(&dir);
                println!("Epic Games Launcher manifests at {}: {} game(s)", dir.display(), games.len());
                for g in games {
                    println!("  {} ({}) in {}", g.title, g.app_name, g.install_location);
                }
            }
            None => println!("no Epic Games Launcher manifests on this PC"),
        }
    }
}
