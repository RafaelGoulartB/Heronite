//! Carries what the user gave one Local game over to another, for a game that
//! ended up in the Collection twice (added here by hand, then brought in from
//! Lutris or Faugus): play time and sessions, status and its history, notes,
//! the journal, the rating, the date added, the actions, favourite / hidden /
//! recent, categories, custom fields and, when the other has none, its
//! Collection metadata and images. Both are `sideload` entries.
//!
//! Nothing of `from` is deleted here: the caller removes it from the
//! Collection afterwards (`heronite_metadata::remove`). Every file is
//! rewritten only when something changes in it.

use std::path::Path;

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::json_store;
use crate::launchers::PREFIX_KEY;
use crate::models::MAX_SESSIONS_PER_GAME;
use crate::paths::DataPaths;

const RUNNER: &str = "sideload";

/// Carries `from`'s data over to `to` (app names of two `sideload` games).
pub fn carry_over(paths: &DataPaths, from: &str, to: &str) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let (from_key, to_key) = (format!("{RUNNER}_{from}"), format!("{RUNNER}_{to}"));
    meta(paths, from, to)?;
    sessions(paths, from, to)?;
    playtime(paths, from, to)?;
    by_key(&paths.status_history(), "changes", from, to, Merge::Concat("at"))?;
    by_key(&paths.ratings(), "ratings", &from_key, &to_key, Merge::KeepTheirs)?;
    by_key(&paths.journal(), "entries", &from_key, &to_key, Merge::Concat("at"))?;
    by_key(&paths.journal(), "leftOff", &from_key, &to_key, Merge::Latest("at"))?;
    by_key(&paths.added(), "added", &from_key, &to_key, Merge::Earliest)?;
    by_key(&paths.game_actions(), "games", &from_key, &to_key, Merge::Concat(""))?;
    store_lists(paths, from, to)?;
    images(&paths.art(), &paths.art_dir(), "art", from, to)?;
    images(&paths.metadata(), &paths.metadata_art_dir(), "metadata-art", from, to)?;
    log::info!("Carried {from} over to {to}");
    Ok(())
}

/// How two values of a game merge.
#[derive(Clone, Copy)]
enum Merge {
    /// Lists joined, sorted by this field (kept in order when empty).
    Concat(&'static str),
    /// The one whose field is the latest.
    Latest(&'static str),
    /// The smallest (dates as text).
    Earliest,
    /// `to`'s when it has one.
    KeepTheirs,
}

fn merged(mine: Value, theirs: Option<Value>, how: Merge) -> Value {
    let Some(theirs) = theirs.filter(|t| !t.is_null()) else { return mine };
    match how {
        Merge::Concat(field) => {
            let (Value::Array(mut a), Value::Array(b)) = (theirs.clone(), mine) else { return theirs };
            a.extend(b);
            if !field.is_empty() {
                a.sort_by(|x, y| text(x, field).cmp(text(y, field)));
            }
            a.dedup();
            Value::Array(a)
        }
        Merge::Latest(field) => {
            if text(&mine, field) > text(&theirs, field) {
                mine
            } else {
                theirs
            }
        }
        Merge::Earliest => match (mine.as_str(), theirs.as_str()) {
            (Some(m), Some(t)) if !m.is_empty() && m < t => mine,
            _ => theirs,
        },
        Merge::KeepTheirs => theirs,
    }
}

fn text<'a>(v: &'a Value, field: &str) -> &'a str {
    v.get(field).and_then(Value::as_str).unwrap_or("")
}

/// Moves `doc[map][from]` onto `doc[map][to]` in the JSON file at `file`.
fn by_key(file: &Path, map: &str, from: &str, to: &str, how: Merge) -> Result<()> {
    if !file.exists() {
        return Ok(());
    }
    json_store::update_value_if_changed(file, |doc| {
        let Some(games) = doc.get_mut(map).and_then(Value::as_object_mut) else { return Ok(()) };
        let Some(mine) = games.get(from).cloned() else { return Ok(()) };
        let value = merged(mine, games.get(to).cloned(), how);
        games.insert(to.to_string(), value);
        Ok(())
    })
}

/// Status and notes (`to` keeps its launcher's own keys), and custom fields
/// `to` does not have.
fn meta(paths: &DataPaths, from: &str, to: &str) -> Result<()> {
    json_store::update_value_if_changed(&paths.local_meta(), |doc| {
        let Some(games) = doc.get_mut("games").and_then(Value::as_object_mut) else { return Ok(()) };
        let Some(Value::Object(mine)) = games.get(from).cloned() else { return Ok(()) };
        let Some(Value::Object(theirs)) = games.get_mut(to) else { return Ok(()) };
        // What says where the game comes from and how it starts stays `to`'s.
        const OWN: &[&str] = &[
            "appName",
            "runner",
            "source",
            "launchKind",
            "storeGameId",
            "title",
            "launcherArgs",
            "windowsInstallDirectory",
            "windowsExecutable",
            "remappedExecutable",
            "runAs",
            "playniteId",
            "pluginId",
            "steamAppId",
            PREFIX_KEY,
        ];
        for (k, v) in mine {
            if OWN.contains(&k.as_str()) || v.is_null() {
                continue;
            }
            match k.as_str() {
                // The status the user set wins over the one the import guessed.
                "completionStatusId" => {
                    theirs.insert(k, v);
                }
                "notes" => {
                    let old = theirs.get("notes").and_then(Value::as_str).unwrap_or("").trim().to_string();
                    let new = v.as_str().unwrap_or("").trim();
                    let notes = match (old.is_empty(), new.is_empty()) {
                        (_, true) => old,
                        (true, false) => new.to_string(),
                        (false, false) if old.contains(new) => old,
                        (false, false) => format!("{old}\n\n{new}"),
                    };
                    theirs.insert(k, json!(notes));
                }
                _ => {
                    theirs.entry(k).or_insert(v);
                }
            }
        }
        Ok(())
    })
}

/// Both histories, newest first.
fn sessions(paths: &DataPaths, from: &str, to: &str) -> Result<()> {
    if !paths.sessions().exists() {
        return Ok(());
    }
    json_store::update_value_if_changed(&paths.sessions(), |doc| {
        let Some(all) = doc.get_mut("sessions").and_then(Value::as_object_mut) else { return Ok(()) };
        let Some(Value::Array(mine)) = all.get(from).cloned() else { return Ok(()) };
        let mut list = all.get(to).and_then(Value::as_array).cloned().unwrap_or_default();
        list.extend(mine);
        list.sort_by(|a, b| text(b, "startedAt").cmp(text(a, "startedAt")));
        list.dedup();
        list.truncate(MAX_SESSIONS_PER_GAME);
        all.insert(to.to_string(), Value::Array(list));
        Ok(())
    })
}

/// `totalPlayed` added up (the two were never played at the same time: one
/// started here, the other in its launcher), first and last played widened.
fn playtime(paths: &DataPaths, from: &str, to: &str) -> Result<()> {
    if !paths.timestamps().exists() {
        return Ok(());
    }
    json_store::update_value_if_changed(&paths.timestamps(), |doc| {
        let Some(all) = doc.as_object_mut() else { return Ok(()) };
        let Some(Value::Object(mine)) = all.get(from).cloned() else { return Ok(()) };
        let entry = all.entry(to.to_string()).or_insert_with(|| json!({"firstPlayed": "", "lastPlayed": "", "totalPlayed": 0}));
        if !entry.is_object() {
            *entry = json!({});
        }
        let theirs = entry.as_object_mut().expect("object");
        let minutes = |m: &Map<String, Value>| m.get("totalPlayed").and_then(Value::as_f64).unwrap_or(0.0);
        let total = minutes(theirs) + minutes(&mine);
        theirs.insert("totalPlayed".into(), json!(total.round() as u64));
        let date = |m: &Map<String, Value>, k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (first, last) = (date(&mine, "firstPlayed"), date(&mine, "lastPlayed"));
        let their_first = date(theirs, "firstPlayed");
        if !first.is_empty() && (their_first.is_empty() || first < their_first) {
            theirs.insert("firstPlayed".into(), json!(first));
        }
        if last > date(theirs, "lastPlayed") {
            theirs.insert("lastPlayed".into(), json!(last));
        }
        Ok(())
    })
}

/// Favourite, hidden and recent lists (`{appName, title}`) and the
/// categories (`<appName>_<runner>`) of `store/config.json`.
fn store_lists(paths: &DataPaths, from: &str, to: &str) -> Result<()> {
    if !paths.store_config().exists() {
        return Ok(());
    }
    let (from_entry, to_entry) = (format!("{from}_{RUNNER}"), format!("{to}_{RUNNER}"));
    json_store::update_value_if_changed(&paths.store_config(), |doc| {
        let Some(games) = doc.get_mut("games").and_then(Value::as_object_mut) else { return Ok(()) };
        for list in ["favourites", "hidden", "recent"] {
            let Some(items) = games.get_mut(list).and_then(Value::as_array_mut) else { continue };
            let app = |v: &Value| v.get("appName").and_then(Value::as_str).map(str::to_owned);
            if !items.iter().any(|v| app(v).as_deref() == Some(from)) {
                continue;
            }
            if items.iter().any(|v| app(v).as_deref() == Some(to)) {
                items.retain(|v| app(v).as_deref() != Some(from));
            } else {
                for v in items.iter_mut().filter(|v| app(v).as_deref() == Some(from)) {
                    v["appName"] = json!(to);
                }
            }
        }
        if let Some(cats) = games.get_mut("customCategories").and_then(Value::as_object_mut) {
            for members in cats.values_mut().filter_map(Value::as_array_mut) {
                if !members.iter().any(|m| m.as_str() == Some(&from_entry)) {
                    continue;
                }
                let has_to = members.iter().any(|m| m.as_str() == Some(&to_entry));
                if has_to {
                    members.retain(|m| m.as_str() != Some(&from_entry));
                } else {
                    for m in members.iter_mut().filter(|m| m.as_str() == Some(&from_entry)) {
                        *m = json!(to_entry);
                    }
                }
            }
        }
        Ok(())
    })
}

/// The folder `heronite_metadata` keeps a game's images in.
fn image_folder(app: &str) -> String {
    let mut out = String::new();
    for c in format!("{RUNNER}_{app}").chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.chars().take(120).collect()
}

/// `from`'s entry of `art.json` / `metadata.json` (`games["sideload:<app>"]`)
/// when `to` has none, with its image files copied to `to`'s folder (`from`'s
/// go with it when it is removed).
fn images(file: &Path, dir: &Path, kind: &str, from: &str, to: &str) -> Result<()> {
    if !file.exists() {
        return Ok(());
    }
    let (from_id, to_id) = (format!("{RUNNER}:{from}"), format!("{RUNNER}:{to}"));
    let doc = json_store::read_value(file)?;
    let Some(entry) = doc.pointer(&format!("/games/{}", escape(&from_id))).cloned() else { return Ok(()) };
    if doc.pointer(&format!("/games/{}", escape(&to_id))).is_some() {
        return Ok(());
    }
    let (from_dir, to_dir) = (image_folder(from), image_folder(to));
    let source = dir.join(&from_dir);
    if source.is_dir() && !dir.join(&to_dir).exists() {
        copy_dir(&source, &dir.join(&to_dir))?;
    }
    let mut entry = rewrite(entry, &format!("localart://{kind}/{from_dir}/"), &format!("localart://{kind}/{to_dir}/"));
    if let Some(o) = entry.as_object_mut() {
        if o.contains_key("appName") {
            o.insert("appName".into(), json!(to));
        }
    }
    json_store::update_value(file, |doc| {
        if let Some(games) = doc.get_mut("games").and_then(Value::as_object_mut) {
            games.entry(to_id).or_insert(entry);
        }
        Ok(())
    })
}

/// JSON pointer escaping of one key.
fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn rewrite(v: Value, from: &str, to: &str) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace(from, to)),
        Value::Array(a) => Value::Array(a.into_iter().map(|v| rewrite(v, from, to)).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, rewrite(v, from, to))).collect()),
        other => other,
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, dest) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dest)?;
        } else {
            std::fs::copy(&src, &dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, value: Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    fn read(path: &Path) -> Value {
        json_store::read_value(path).unwrap()
    }

    #[test]
    fn carries_everything_the_user_gave_the_game() {
        let dir = tempfile::tempdir().unwrap();
        let p = DataPaths::at(dir.path());
        write(
            &p.local_meta(),
            json!({"games": {
                "mine": {"appName": "mine", "runner": "sideload", "source": "manual", "title": "HoMM3",
                         "completionStatusId": "beaten", "notes": "Use the HD mod", "myField": "x"},
                "lutris_7": {"appName": "lutris_7", "runner": "sideload", "source": "lutris", "launchKind": "launcher",
                             "title": "Heroes III", "launcherArgs": "lutris lutris:rungameid/7",
                             "completionStatusId": "not-played", "heroniteLauncherPrefix": "/p"}
            }}),
        );
        write(
            &p.sessions(),
            json!({"sessions": {
                "mine": [{"startedAt": "2026-01-02T00:00:00Z", "endedAt": "", "elapsedSeconds": 60, "source": "heroic"}],
                "lutris_7": [{"startedAt": "2026-03-01T00:00:00Z", "endedAt": "", "elapsedSeconds": 30, "source": "heroic"}]
            }}),
        );
        write(
            &p.timestamps(),
            json!({
                "mine": {"firstPlayed": "2026-01-02T00:00:00Z", "lastPlayed": "2026-01-05T00:00:00Z", "totalPlayed": 120},
                "lutris_7": {"firstPlayed": "2026-03-01T00:00:00Z", "lastPlayed": "2026-03-01T00:00:00Z", "totalPlayed": 30}
            }),
        );
        write(&p.ratings(), json!({"ratings": {"sideload_mine": 5}}));
        write(
            &p.journal(),
            json!({"entries": {"sideload_mine": [{"id": "1", "at": "2026-01-03T00:00:00Z", "text": "Castle"}]},
                   "leftOff": {"sideload_mine": {"text": "Day 3", "at": "2026-01-05T00:00:00Z"}}}),
        );
        write(&p.added(), json!({"added": {"sideload_mine": "2026-01-01", "sideload_lutris_7": "2026-10-22"}}));
        write(&p.status_history(), json!({"changes": {"mine": [{"status": "beaten", "at": "2026-01-05T00:00:00Z"}]}}));
        write(
            &p.store_config(),
            json!({"games": {
                "favourites": [{"appName": "mine", "title": "HoMM3"}],
                "recent": [{"appName": "mine", "title": "HoMM3"}, {"appName": "lutris_7", "title": "Heroes III"}],
                "customCategories": {"Strategy": ["mine_sideload", "x_gog"]}
            }, "other": 1}),
        );
        fs::create_dir_all(p.art_dir().join("sideload_mine")).unwrap();
        fs::write(p.art_dir().join("sideload_mine/cover-1.png"), b"png").unwrap();
        write(&p.art(), json!({"games": {"sideload:mine": {"coverUrl": "localart://art/sideload_mine/cover-1.png"}}}));

        carry_over(&p, "mine", "lutris_7").unwrap();

        let meta = read(&p.local_meta());
        let theirs = &meta["games"]["lutris_7"];
        assert_eq!(theirs["completionStatusId"], "beaten");
        assert_eq!(theirs["notes"], "Use the HD mod");
        assert_eq!(theirs["myField"], "x");
        assert_eq!(theirs["title"], "Heroes III");
        assert_eq!(theirs["source"], "lutris");
        assert_eq!(theirs["launcherArgs"], "lutris lutris:rungameid/7");
        assert_eq!(theirs["heroniteLauncherPrefix"], "/p");

        let sessions = read(&p.sessions());
        let list: Vec<&str> = sessions["sessions"]["lutris_7"].as_array().unwrap().iter().map(|s| text(s, "startedAt")).collect();
        assert_eq!(list, ["2026-03-01T00:00:00Z", "2026-01-02T00:00:00Z"]);

        let t = &read(&p.timestamps())["lutris_7"];
        assert_eq!(t["totalPlayed"], 150);
        assert_eq!(t["firstPlayed"], "2026-01-02T00:00:00Z");
        assert_eq!(t["lastPlayed"], "2026-03-01T00:00:00Z");

        assert_eq!(read(&p.ratings())["ratings"]["sideload_lutris_7"], 5);
        let journal = read(&p.journal());
        assert_eq!(journal["entries"]["sideload_lutris_7"][0]["text"], "Castle");
        assert_eq!(journal["leftOff"]["sideload_lutris_7"]["text"], "Day 3");
        assert_eq!(read(&p.added())["added"]["sideload_lutris_7"], "2026-01-01");
        assert_eq!(read(&p.status_history())["changes"]["lutris_7"][0]["status"], "beaten");

        let store = read(&p.store_config());
        assert_eq!(store["games"]["favourites"], json!([{"appName": "lutris_7", "title": "HoMM3"}]));
        assert_eq!(store["games"]["recent"], json!([{"appName": "lutris_7", "title": "Heroes III"}]));
        assert_eq!(store["games"]["customCategories"]["Strategy"], json!(["lutris_7_sideload", "x_gog"]));
        assert_eq!(store["other"], 1);

        let art = read(&p.art());
        assert_eq!(art["games"]["sideload:lutris_7"]["coverUrl"], "localart://art/sideload_lutris_7/cover-1.png");
        assert_eq!(fs::read(p.art_dir().join("sideload_lutris_7/cover-1.png")).unwrap(), b"png");
    }

    #[test]
    fn keeps_what_the_other_already_has() {
        let dir = tempfile::tempdir().unwrap();
        let p = DataPaths::at(dir.path());
        write(&p.ratings(), json!({"ratings": {"sideload_mine": 2, "sideload_lutris_7": 4}}));
        write(
            &p.art(),
            json!({"games": {"sideload:mine": {"coverUrl": "localart://art/sideload_mine/a.png"},
                             "sideload:lutris_7": {"coverUrl": "localart://art/sideload_lutris_7/b.png"}}}),
        );
        write(
            &p.local_meta(),
            json!({"games": {"mine": {"appName": "mine", "runner": "sideload", "notes": "same"},
                             "lutris_7": {"appName": "lutris_7", "runner": "sideload", "notes": "same"}}}),
        );
        carry_over(&p, "mine", "lutris_7").unwrap();
        assert_eq!(read(&p.ratings())["ratings"]["sideload_lutris_7"], 4);
        assert_eq!(read(&p.art())["games"]["sideload:lutris_7"]["coverUrl"], "localart://art/sideload_lutris_7/b.png");
        assert_eq!(read(&p.local_meta())["games"]["lutris_7"]["notes"], "same");
        // Missing files stay missing.
        assert!(!p.sessions().exists() && !p.timestamps().exists() && !p.journal().exists());
    }
}
