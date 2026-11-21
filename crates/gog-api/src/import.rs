//! A GOG game already on disk: what `gogdl import <folder>` printed, read
//! from the `goggame-<id>.info` files GOG's installers leave (in the folder,
//! in `Contents/Resources` of a Mac app, or in `game/` of a Linux one) or a
//! Linux installer's `gameinfo`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::api::{self, Client};
use crate::builds::Builds;
use crate::linux;

/// What the folder holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// `goggame-<root id>.info`, if any (none for a Linux installer's game).
    pub info_file: Option<PathBuf>,
    /// `goggame-<root id>.id`, if any.
    pub id_file: Option<PathBuf>,
    /// `windows`, `osx` or `linux`.
    pub platform: &'static str,
    /// Ids of the DLCs installed with it.
    pub dlcs: Vec<String>,
}

fn info_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("goggame-") && n.ends_with(".info")))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// gogdl's search of `folder`.
pub fn find(folder: &Path) -> Option<Found> {
    let places: [(PathBuf, &'static str); 3] =
        [(folder.to_path_buf(), "windows"), (folder.join("Contents").join("Resources"), "osx"), (folder.join("game"), "linux")];
    let Some((base, platform, infos)) = places
        .into_iter()
        .map(|(dir, os)| {
            let found = info_files(&dir);
            (dir, os, found)
        })
        .find(|(_, _, f)| !f.is_empty())
    else {
        return folder.join("gameinfo").exists().then(|| Found { info_file: None, id_file: None, platform: "linux", dlcs: Vec::new() });
    };
    let mut root: Option<String> = None;
    let mut dlcs = Vec::new();
    for path in &infos {
        let Some(doc) = read_json(path) else { continue };
        if root.is_none() {
            root = doc.get("rootGameId").and_then(api::id_text);
        }
        match doc.get("gameId").and_then(api::id_text) {
            Some(id) if Some(&id) != root.as_ref() => dlcs.push(id),
            _ => {}
        }
    }
    let root = root?;
    let id_file = base.join(format!("goggame-{root}.id"));
    Some(Found { info_file: Some(base.join(format!("goggame-{root}.info"))), id_file: id_file.exists().then_some(id_file), platform, dlcs })
}

/// The JSON `gogdl import` printed (`appName`, `buildId`, `title`, `tasks`,
/// `installedLanguage`, `dlcs`, `platform`, `versionName`). The version name
/// of a build id comes from GOG (network); without it the build id stands in.
pub fn import(client: &Client, folder: &Path) -> Result<Value> {
    if !folder.exists() {
        bail!(tr!("The folder does not exist"));
    }
    let Some(found) = find(folder) else { bail!(tr!("No GOG game was found in that folder")) };
    let mut app_name: Option<String> = None;
    let mut build_id: Option<String> = None;
    let mut title = String::new();
    let mut tasks = Value::Null;
    let mut language: Option<String> = None;
    let mut version: Option<String> = None;
    if found.platform != "linux" || found.info_file.is_some() {
        let Some(info) = found.info_file.as_deref().and_then(read_json) else { bail!(tr!("No GOG game was found in that folder")) };
        title = info.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        app_name = info.get("rootGameId").and_then(api::id_text);
        build_id = info.get("buildId").and_then(api::id_text);
        language = info
            .get("languages")
            .and_then(|l| l.get(0))
            .or(info.get("language"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(Some("en-US".into()));
        if let Some(id) = found.id_file.as_deref().and_then(read_json).and_then(|d| d.get("buildId").and_then(api::id_text)) {
            build_id = Some(id);
        }
        tasks = info.get("playTasks").cloned().filter(|t| t.as_array().is_some_and(|l| !l.is_empty())).unwrap_or(Value::Null);
        version = build_id.clone();
        if let (Some(game), Some(build)) = (app_name.as_deref(), build_id.as_deref()) {
            if found.platform != "linux" {
                match client.get_json(&api::builds_url(game, found.platform, 2)) {
                    Ok(doc) => {
                        let builds = Builds::parse(&doc);
                        let target = builds.items.iter().find(|b| b.build_id == build).or(builds.items.first());
                        version = target.map(|b| b.version_name.clone()).or(version);
                    }
                    Err(err) => log::warn!("reading the builds of GOG game {game}: {err:#}"),
                }
            }
        }
    }
    if found.platform == "linux" {
        if let Ok(text) = std::fs::read_to_string(folder.join("gameinfo")) {
            let info = linux::parse_gameinfo(&text);
            title = info.title;
            version = Some(info.version);
            if language.is_none() {
                language = info.language;
            }
            app_name = info.game_id;
            build_id = info.build_id;
        }
    }
    Ok(json!({
        "appName": app_name,
        "buildId": build_id,
        "title": title,
        "tasks": tasks,
        "installedLanguage": language,
        "dlcs": found.dlcs,
        "platform": found.platform,
        "versionName": version,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_install_with_a_dlc() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("goggame-100.info"), r#"{"gameId": "100", "rootGameId": "100", "name": "The Game", "buildId": "5", "languages": ["de-DE"], "playTasks": [{"isPrimary": true, "path": "game.exe"}]}"#).unwrap();
        std::fs::write(d.join("goggame-200.info"), r#"{"gameId": "200", "rootGameId": "100", "name": "DLC"}"#).unwrap();
        std::fs::write(d.join("goggame-100.id"), r#"{"buildId": "6"}"#).unwrap();
        let found = find(d).unwrap();
        assert_eq!((found.platform, found.dlcs.clone()), ("windows", vec!["200".to_string()]));
        assert!(found.id_file.is_some());
        assert!(find(&d.join("nothing")).is_none());
    }

    #[test]
    fn a_linux_installers_game() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gameinfo"), "Linux Game\n1.2\n\nen-US\n300\n1\n77\n").unwrap();
        let doc = import(&Client::anonymous(), dir.path()).unwrap();
        assert_eq!(doc["appName"], "300");
        assert_eq!(doc["buildId"], "77");
        assert_eq!(doc["platform"], "linux");
        assert_eq!(doc["versionName"], "1.2");
        assert_eq!(doc["installedLanguage"], "en-US");
        assert_eq!(doc["title"], "Linux Game");
    }
}
