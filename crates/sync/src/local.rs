//! This PC's side: what its Collection holds for each field that syncs
//! ([`read`]), and writing what came from the other PCs ([`write`]), one
//! write per file through the same `heronite_data` functions the screens use.

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use heronite_data::models::{CompletionStatus, LocalGameSession, LocalMetaFile, SessionsFile, MAX_SESSIONS_PER_GAME};
use heronite_data::status::SetStatusExtras;
use heronite_data::{config_games, json_store, ratings, status, status_history, DataPaths, Library};
use serde_json::{json, Map, Value};

use crate::format::{Field, Playtime};

/// The status new games of a store start with. On a PC that never synced it
/// says nothing about the game: it never replaces another PC's status.
const NOT_PLAYED: &str = "not-played";

#[derive(Debug, Clone, Default)]
pub struct LocalGame {
    pub runner: String,
    pub app_name: String,
    pub title: String,
    /// Every [`Field`]; `null` for none or off.
    pub values: BTreeMap<Field, Value>,
    /// When the status became what it is (`status-history.json`), if known.
    pub status_at: Option<String>,
    pub playtime: Playtime,
    pub sessions: Vec<LocalGameSession>,
}

impl LocalGame {
    pub fn value(&self, field: Field) -> Value {
        self.values.get(&field).cloned().unwrap_or(Value::Null)
    }

    /// The value as a first sync sees it: what a new game starts with is no
    /// value at all.
    pub fn initial_value(&self, field: Field) -> Value {
        match self.value(field) {
            Value::String(s) if field == Field::Status && s == NOT_PLAYED => Value::Null,
            v => v,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LocalLibrary {
    /// By Collection key.
    pub games: BTreeMap<String, LocalGame>,
    /// The statuses the user made (not the built-in ones).
    pub custom_statuses: Vec<CompletionStatus>,
}

/// Reads the Collection (hidden games too).
pub fn read(paths: &DataPaths) -> Result<LocalLibrary> {
    let lib = Library::load_with(paths, true)?;
    let ratings = ratings::load(paths);
    let history = status_history::load(paths);
    let mut games = BTreeMap::new();
    for g in &lib.games {
        let text = |v: Option<&str>| v.map(str::trim).filter(|s| !s.is_empty()).map_or(Value::Null, |s| Value::String(s.to_owned()));
        let flag = |on: bool| if on { Value::Bool(true) } else { Value::Null };
        let mut values = BTreeMap::new();
        values.insert(Field::Status, text(g.status_raw.as_deref()));
        values.insert(Field::Favourite, flag(g.favourite));
        values.insert(Field::Hidden, flag(g.hidden));
        values.insert(Field::Rating, ratings.get(&g.key).map_or(Value::Null, |s| json!(s)));
        values.insert(Field::Notes, text(g.details.notes.as_deref()));
        let status_at =
            g.status_raw.as_deref().and_then(|now| history.get(&g.app_name)?.iter().rev().find(|c| c.status == now).map(|c| c.at.clone()));
        games.insert(
            g.key.clone(),
            LocalGame {
                runner: g.runner.clone(),
                app_name: g.app_name.clone(),
                title: g.title.clone(),
                values,
                status_at,
                playtime: Playtime {
                    minutes: g.playtime_minutes,
                    first_played: g.first_played.clone(),
                    last_played: g.last_played.clone(),
                    lowered: None,
                },
                sessions: g.sessions.as_ref().map(|s| s.as_ref().clone()).unwrap_or_default(),
            },
        );
    }
    let builtin: HashSet<String> = status::default_statuses().into_iter().map(|s| s.id).collect();
    let custom_statuses = lib.statuses.iter().filter(|s| !builtin.contains(&s.id)).cloned().collect();
    Ok(LocalLibrary { games, custom_statuses })
}

/// One field of one game to change here.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldChange {
    pub key: String,
    pub runner: String,
    pub app_name: String,
    pub title: String,
    pub field: Field,
    pub value: Value,
    /// When it was changed on the other PC.
    pub at: String,
}

/// What the other PCs bring to this one.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub fields: Vec<FieldChange>,
    /// New sessions by `appName`.
    pub sessions: BTreeMap<String, Vec<LocalGameSession>>,
    /// New totals by `appName`.
    pub playtime: BTreeMap<String, Playtime>,
    /// Statuses of the other PCs this one does not have.
    pub statuses: Vec<CompletionStatus>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.sessions.is_empty() && self.playtime.is_empty() && self.statuses.is_empty()
    }
}

/// Writes `changes` into the data folder.
pub fn write(paths: &DataPaths, changes: &Changes) -> Result<()> {
    // Statuses first: a game may move to one of them.
    for s in &changes.statuses {
        status::upsert_status(paths, s.clone())?;
    }
    write_statuses(paths, changes)?;
    let flags = |field: Field| -> Vec<(String, String, bool)> {
        changes
            .fields
            .iter()
            .filter(|c| c.field == field)
            .map(|c| (c.app_name.clone(), c.title.clone(), c.value.as_bool().unwrap_or(false)))
            .collect()
    };
    config_games::set_favourites(paths, &flags(Field::Favourite))?;
    config_games::set_hidden(paths, &flags(Field::Hidden))?;
    write_ratings(paths, changes)?;
    for c in changes.fields.iter().filter(|c| c.field == Field::Notes) {
        let patch = heronite_metadata::types::DetailsPatch {
            app_name: c.app_name.clone(),
            runner: c.runner.clone(),
            notes: Some(c.value.as_str().unwrap_or_default().to_owned()),
            ..Default::default()
        };
        heronite_metadata::edit::update_collection_game_details(paths, &patch)?;
    }
    write_sessions(paths, &changes.sessions)?;
    write_playtime(paths, &changes.playtime)
}

fn write_statuses(paths: &DataPaths, changes: &Changes) -> Result<()> {
    let statuses = changes.fields.iter().filter(|c| c.field == Field::Status);
    let (set, cleared): (Vec<&FieldChange>, Vec<&FieldChange>) = statuses.partition(|c| c.value.is_string());
    let set: Vec<(String, String, SetStatusExtras, String)> = set
        .into_iter()
        .map(|c| {
            let extras = SetStatusExtras { runner: Some(c.runner.clone()), title: Some(c.title.clone()), playnite_id: None };
            (c.app_name.clone(), c.value.as_str().unwrap_or_default().to_owned(), extras, c.at.clone())
        })
        .collect();
    status::set_games_status_made_at(paths, &set)?;
    if cleared.is_empty() {
        return Ok(());
    }
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for c in &cleared {
            if let Some(meta) = file.games.get_mut(&c.app_name) {
                meta.completion_status_id = None;
            }
        }
        Ok(())
    })
}

fn write_ratings(paths: &DataPaths, changes: &Changes) -> Result<()> {
    let list: Vec<&FieldChange> = changes.fields.iter().filter(|c| c.field == Field::Rating).collect();
    if list.is_empty() {
        return Ok(());
    }
    json_store::update_value(&paths.ratings(), |doc| {
        if !doc.is_object() {
            *doc = json!({});
        }
        let root = doc.as_object_mut().expect("object");
        let map = root.entry("ratings").or_insert_with(|| json!({}));
        if !map.is_object() {
            *map = json!({});
        }
        let map = map.as_object_mut().expect("object");
        for c in list {
            match c.value.as_u64().filter(|n| *n > 0) {
                Some(n) => {
                    map.insert(c.key.clone(), json!(n.min(5)));
                }
                None => {
                    map.shift_remove(&c.key);
                }
            }
        }
        Ok(())
    })
}

/// Adds the sessions (`mergeLocalSessions`: a session already there is not
/// added twice; the newest [`MAX_SESSIONS_PER_GAME`] are kept).
fn write_sessions(paths: &DataPaths, sessions: &BTreeMap<String, Vec<LocalGameSession>>) -> Result<()> {
    if sessions.is_empty() {
        return Ok(());
    }
    json_store::update::<SessionsFile, _>(&paths.sessions(), |doc| {
        for (app_name, new) in sessions {
            let list = doc.sessions.entry(app_name.clone()).or_default();
            for s in new {
                if !list.iter().any(|have| have.started_at == s.started_at) {
                    list.push(s.clone());
                }
            }
            if list.len() > MAX_SESSIONS_PER_GAME {
                list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
                list.truncate(MAX_SESSIONS_PER_GAME);
            }
        }
        Ok(())
    })
}

/// Sets `totalPlayed`, `firstPlayed` and `lastPlayed` (`store/timestamp.json`);
/// the entry's other keys stay.
fn write_playtime(paths: &DataPaths, playtime: &BTreeMap<String, Playtime>) -> Result<()> {
    if playtime.is_empty() {
        return Ok(());
    }
    json_store::update_value(&paths.timestamps(), |root| {
        if !root.is_object() {
            *root = Value::Object(Map::new());
        }
        let map = root.as_object_mut().expect("object");
        for (app_name, p) in playtime {
            let entry = map.entry(app_name.clone()).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            let entry = entry.as_object_mut().expect("object");
            entry.insert("firstPlayed".into(), json!(p.first_played.clone().unwrap_or_default()));
            entry.insert("lastPlayed".into(), json!(p.last_played.clone().unwrap_or_default()));
            entry.insert("totalPlayed".into(), json!(p.minutes));
        }
        Ok(())
    })
}
