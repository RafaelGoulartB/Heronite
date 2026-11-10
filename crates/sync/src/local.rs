//! This PC's side: what its Collection holds for each field that syncs, its
//! custom categories and its smart groups ([`read`]), and writing what came
//! from the other PCs ([`write`]), one write per file through the same
//! `heronite_data` functions the screens use.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::Result;
use heronite_data::categories::{self, Categories};
use heronite_data::models::{CompletionStatus, LocalGameSession, LocalMetaFile, SessionsFile, MAX_SESSIONS_PER_GAME};
use heronite_data::smart_groups::{self, SmartGroup, SmartGroups};
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
    /// The custom categories it is in.
    pub categories: BTreeSet<String>,
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
    /// Smart groups by id, as they sync ([`shared_group`]).
    pub groups: BTreeMap<String, Value>,
}

/// A smart group as the PCs share it: as stored, without `pinned` (each PC
/// pins its own).
pub fn shared_group(group: &SmartGroup) -> Value {
    let mut value = group.to_value();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("pinned");
    }
    value
}

/// `value` can be stored here as group `id`: a group this version reads, or
/// a deletion. [`write_groups`] skips anything else.
pub fn storable_group(id: &str, value: &Value) -> bool {
    value.is_null() || SmartGroup::from_value(value).is_some_and(|g| g.id == id)
}

/// Reads the Collection (hidden games too).
pub fn read(paths: &DataPaths) -> Result<LocalLibrary> {
    let lib = Library::load_with(paths, true)?;
    let cats = Categories::load(paths);
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
                categories: cats.of_game(&g.runner, &g.app_name).into_iter().map(str::to_owned).collect(),
            },
        );
    }
    let builtin: HashSet<String> = status::default_statuses().into_iter().map(|s| s.id).collect();
    let custom_statuses = lib.statuses.iter().filter(|s| !builtin.contains(&s.id)).cloned().collect();
    let groups = SmartGroups::load(paths).groups.iter().map(|g| (g.id.clone(), shared_group(g))).collect();
    Ok(LocalLibrary { games, custom_statuses, groups })
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

/// A game put in a custom category, or taken out.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryChange {
    pub runner: String,
    pub app_name: String,
    pub category: String,
    pub on: bool,
}

/// A smart group to store as `value` ([`shared_group`]), or to delete
/// (`null`).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupChange {
    pub id: String,
    pub value: Value,
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
    pub categories: Vec<CategoryChange>,
    pub groups: Vec<GroupChange>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
            && self.sessions.is_empty()
            && self.playtime.is_empty()
            && self.statuses.is_empty()
            && self.categories.is_empty()
            && self.groups.is_empty()
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
    write_playtime(paths, &changes.playtime)?;
    write_categories(paths, &changes.categories)?;
    write_groups(paths, &changes.groups)
}

/// One write per category.
fn write_categories(paths: &DataPaths, changes: &[CategoryChange]) -> Result<()> {
    let mut by_name: BTreeMap<&str, Vec<(String, String, bool)>> = BTreeMap::new();
    for c in changes {
        by_name.entry(&c.category).or_default().push((c.runner.clone(), c.app_name.clone(), c.on));
    }
    for (name, games) in by_name {
        categories::set_many_in_category(paths, name, &games)?;
    }
    Ok(())
}

/// Stores or deletes the groups, keeping each one's pin here (a new one
/// starts unpinned) and the place of those already here.
fn write_groups(paths: &DataPaths, changes: &[GroupChange]) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut groups = SmartGroups::load(paths).groups;
    for c in changes {
        let at = groups.iter().position(|g| g.id == c.id);
        let theirs = SmartGroup::from_value(&c.value).filter(|g| g.id == c.id); // see storable_group
        match (at, theirs) {
            (Some(i), Some(mut g)) => {
                g.pinned = groups[i].pinned;
                groups[i] = g;
            }
            (None, Some(g)) => groups.push(SmartGroup { pinned: false, ..g }),
            (Some(i), None) if c.value.is_null() => {
                groups.remove(i);
            }
            _ => {}
        }
    }
    smart_groups::save(paths, &groups)
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
    })?;
    // A total made smaller by hand on another PC holds here too against
    // what the Xbox account counted (only Xbox games carry that count).
    let lowered = playtime.iter().filter(|(_, p)| p.lowered.is_some()).map(|(app, p)| (app.as_str(), p.minutes));
    heronite_data::xbox::note_playtime_edits(paths, lowered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_from_other_pcs_keep_the_pins_of_this_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let rule = heronite_data::smart_groups::Rule::default();
        let mut pinned = SmartGroup::new("g1", "Weekend", rule.clone());
        pinned.pinned = true;
        smart_groups::save(&paths, &[pinned.clone(), SmartGroup::new("g2", "Old", rule.clone())]).unwrap();
        assert!(shared_group(&pinned).get("pinned").is_none(), "pins stay on each PC");

        let renamed = shared_group(&SmartGroup::new("g1", "Weekend games", rule.clone()));
        let new = shared_group(&SmartGroup::new("g3", "Co-op", rule));
        let changes = Changes {
            groups: vec![
                GroupChange { id: "g1".into(), value: renamed },
                GroupChange { id: "g2".into(), value: Value::Null },
                GroupChange { id: "g3".into(), value: new },
            ],
            ..Default::default()
        };
        write(&paths, &changes).unwrap();
        let groups = SmartGroups::load(&paths).groups;
        let summary: Vec<(&str, &str, bool)> = groups.iter().map(|g| (g.id.as_str(), g.name.as_str(), g.pinned)).collect();
        assert_eq!(summary, [("g1", "Weekend games", true), ("g3", "Co-op", false)]);
    }

    #[test]
    fn categories_from_other_pcs_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        categories::set_in_category(&paths, "RPG", "legendary", "a", true).unwrap();
        let change = |name: &str, on| CategoryChange { runner: "legendary".into(), app_name: "a".into(), category: name.into(), on };
        write(&paths, &Changes { categories: vec![change("RPG", false), change("Souls", true)], ..Default::default() }).unwrap();
        assert_eq!(Categories::load(&paths).of_game("legendary", "a"), ["Souls"]);
    }
    use crate::format::Stamp;
    use heronite_data::models::LocalGameMeta;
    use heronite_data::xbox::{played, MINUTES_KEY, SOURCE};

    #[test]
    fn a_total_lowered_on_another_pc_holds_against_the_xbox_count() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let mut file = LocalMetaFile::default();
        let mut meta = LocalGameMeta { app_name: "xbox_a".into(), source: SOURCE.into(), ..Default::default() };
        meta.extra.insert(MINUTES_KEY.into(), json!(754));
        file.games.insert("xbox_a".into(), meta);
        json_store::write(&paths.local_meta(), &file).unwrap();

        let stamp = Stamp { clock: 1, at: "2026-11-07T10:00:00Z".into(), device: "other".into() };
        let changes = Changes {
            playtime: BTreeMap::from([("xbox_a".to_string(), Playtime { minutes: 100, lowered: Some(stamp), ..Default::default() })]),
            ..Default::default()
        };
        write(&paths, &changes).unwrap();
        let file: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(played(file.games.get("xbox_a")).0, 100, "not raised back to Xbox's 754");
    }
}
