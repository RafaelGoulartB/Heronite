//! Merging the other PCs' files into this one's Collection. Pure: no I/O.
//!
//! - **Fields** (status, favourite, hidden, rating, notes, and each custom
//!   category a game is in or not): the newest [`Stamp`] wins, field by field. A value changed here since the last
//!   sync gets a stamp past every clock seen; a value seen for the first
//!   time gets a weak one (clock 0), so a PC joining with defaults never
//!   overwrites the history of the others.
//! - **Sessions**: a union. A PC publishes only the sessions played on it;
//!   one taken in is remembered, so deleting it here does not bring it back.
//! - **Play time**: the largest total (see [`Playtime`]), unless a total was
//!   made smaller by hand: the newest such change is followed.
//! - A game the other PC has and this one does not is left alone: its data
//!   stays in that PC's file and comes in once the game is here too.
//! - **Smart groups**: the newest stamp wins, group by group (a deletion is a
//!   stamped `null`). Which games a group holds is never synced: each PC
//!   works it out from its own Collection, and keeps its own pins.
//! - A PC that publishes only its play (`publish_edits` off, for a PC the
//!   user does not keep organised) sends its sessions and play time and none
//!   of its fields; what is changed there loses to any other PC's value.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use heronite_data::models::LocalGameSession;

use serde_json::Value;

use crate::format::{category_of, DeviceFile, DeviceInfo, Field, GameState, Playtime, Stamp, Stamped, CATEGORY_PREFIX, FORMAT};
use crate::ledger::{Ledger, LedgerGame};
use crate::local::{CategoryChange, Changes, FieldChange, GroupChange, LocalGame, LocalLibrary};

/// What a sync did, for Settings → Sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Fields changed here.
    pub fields: usize,
    /// Sessions added here.
    pub sessions: usize,
    /// Play times changed here.
    pub playtime: usize,
    /// Games of the other PCs that are not in this Collection (yet).
    pub waiting: usize,
    /// Smart groups added, changed or deleted here.
    pub groups: usize,
}

impl Summary {
    pub fn changed(&self) -> bool {
        self.fields + self.sessions + self.playtime + self.groups > 0
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    /// What to write here.
    pub changes: Changes,
    /// What to remember until the next sync.
    pub ledger: Ledger,
    /// This PC's new file.
    pub own: DeviceFile,
    pub summary: Summary,
}

/// Merges `remotes` (the other PCs' files; this PC's own is skipped) into
/// `local`. `now`: RFC 3339, UTC. `publish_edits` off: this PC's file holds
/// only its play.
pub fn merge(local: &LocalLibrary, mut ledger: Ledger, remotes: &[DeviceFile], me: &DeviceInfo, now: &str, publish_edits: bool) -> Outcome {
    stamp_local_changes(local, &mut ledger, me, now, publish_edits);
    let remotes: Vec<&DeviceFile> = remotes.iter().filter(|r| r.device.id != me.id && r.format <= FORMAT).collect();
    for r in &remotes {
        ledger.clock = ledger.clock.max(r.clock);
        take_newer(&mut ledger.groups, &r.groups);
    }

    let mut changes = Changes::default();
    let mut summary = Summary::default();
    let mut waiting = BTreeSet::new();
    let mut theirs: BTreeMap<&str, Vec<&Playtime>> = BTreeMap::new();
    for r in &remotes {
        for (key, rg) in &r.games {
            let Some(g) = local.games.get(key) else {
                waiting.insert(key.as_str());
                continue;
            };
            let lg = ledger.games.entry(key.clone()).or_default();
            take_fields(lg, rg);
            let new = take_sessions(lg, g, rg);
            if !new.is_empty() {
                summary.sessions += new.len();
                changes.sessions.entry(g.app_name.clone()).or_default().extend(new);
            }
            if let Some(p) = &rg.playtime {
                theirs.entry(key.as_str()).or_default().push(p);
            }
        }
    }
    summary.waiting = waiting.len();

    let mut playtimes = BTreeMap::new();
    for (key, g) in &local.games {
        let lg = ledger.games.entry(key.clone()).or_default();
        for (field_key, stamped) in &lg.fields {
            if let Some(name) = category_of(field_key) {
                let on = stamped.value.as_bool() == Some(true);
                if on != g.categories.contains(name) {
                    changes.categories.push(CategoryChange {
                        runner: g.runner.clone(),
                        app_name: g.app_name.clone(),
                        category: name.to_owned(),
                        on,
                    });
                }
                continue;
            }
            let Some(field) = Field::parse(field_key) else { continue };
            if stamped.value != g.value(field) {
                changes.fields.push(FieldChange {
                    key: key.clone(),
                    runner: g.runner.clone(),
                    app_name: g.app_name.clone(),
                    title: g.title.clone(),
                    field,
                    value: stamped.value.clone(),
                    at: stamped.stamp.at.clone(),
                });
            }
        }
        let merged = merge_playtime(lg, &g.playtime, theirs.get(key.as_str()).map(Vec::as_slice).unwrap_or_default());
        if merged.minutes != g.playtime.minutes
            || merged.first_played != g.playtime.first_played
            || merged.last_played != g.playtime.last_played
        {
            changes.playtime.insert(g.app_name.clone(), merged.clone());
        }
        lg.published_minutes = Some(merged.minutes);
        playtimes.insert(key.clone(), merged);
    }
    for (id, stamped) in &ledger.groups {
        // A group this PC can't read (from a newer version) is never stored
        // here, so it would count as a change on every sync.
        if local.groups.get(id).unwrap_or(&Value::Null) != &stamped.value && crate::local::storable_group(id, &stamped.value) {
            changes.groups.push(GroupChange { id: id.clone(), value: stamped.value.clone() });
        }
    }
    summary.fields = changes.fields.len() + changes.categories.len();
    summary.playtime = changes.playtime.len();
    summary.groups = changes.groups.len();

    let known: HashSet<&str> = local.custom_statuses.iter().map(|s| s.id.as_str()).collect();
    let mut added = HashSet::new();
    for r in &remotes {
        for s in &r.statuses {
            if !known.contains(s.id.as_str()) && added.insert(s.id.clone()) {
                changes.statuses.push(s.clone());
            }
        }
    }

    ledger.games.retain(|_, g| !g.is_empty());
    let mut own = own_file(local, &ledger, &playtimes, me, now);
    if !publish_edits {
        own.statuses.clear();
        own.groups.clear();
        for g in own.games.values_mut() {
            g.fields.clear();
            if let Some(p) = &mut g.playtime {
                p.lowered = None;
            }
        }
        own.games.retain(|_, g| g.playtime.is_some() || !g.sessions.is_empty());
    }
    Outcome { changes, ledger, own, summary }
}

/// Gives the fields changed here since the last sync a new stamp (the
/// weakest of all when this PC does not publish them).
fn stamp_local_changes(local: &LocalLibrary, ledger: &mut Ledger, me: &DeviceInfo, now: &str, publish_edits: bool) {
    let Ledger { clock, games, groups } = ledger;
    let fresh = |clock: &mut u64| {
        *clock += 1;
        Stamp { clock: *clock, at: now.to_owned(), device: me.id.clone() }
    };
    let ids: BTreeSet<String> = local.groups.keys().chain(groups.keys()).cloned().collect();
    for id in ids {
        let held = groups.get(&id);
        // A group this PC can't read (from a newer version) is never stored
        // here: the Collection's copy, or its absence, is no edit, and
        // stamping it would overwrite or delete the group on every PC.
        if held.is_some_and(|s| !crate::local::storable_group(&id, &s.value)) {
            continue;
        }
        let value = local.groups.get(&id).cloned().unwrap_or(Value::Null);
        if let Some(stamp) = restamp(held, &value, publish_edits, || fresh(clock), &me.id) {
            groups.insert(id, Stamped { value, stamp });
        }
    }
    for (key, g) in &local.games {
        let lg = games.entry(key.clone()).or_default();
        let names: BTreeSet<String> =
            g.categories.iter().cloned().chain(lg.fields.keys().filter_map(|k| category_of(k)).map(str::to_owned)).collect();
        for name in names {
            let value = if g.categories.contains(&name) { Value::Bool(true) } else { Value::Null };
            let key = format!("{CATEGORY_PREFIX}{name}");
            if let Some(stamp) = restamp(lg.fields.get(&key), &value, publish_edits, || fresh(clock), &me.id) {
                lg.fields.insert(key, Stamped { value, stamp });
            }
        }
        lg.runner.clone_from(&g.runner);
        lg.app_name.clone_from(&g.app_name);
        lg.title.clone_from(&g.title);
        for field in Field::ALL {
            let value = g.value(field);
            let stamp = match lg.fields.get(field.key()) {
                Some(s) if s.value == value => continue,
                _ if !publish_edits => Stamp::default(),
                Some(_) => {
                    *clock += 1;
                    Stamp { clock: *clock, at: now.to_owned(), device: me.id.clone() }
                }
                None => {
                    if g.initial_value(field).is_null() {
                        continue;
                    }
                    // Weak: older than any change made while syncing. The
                    // status keeps the time it was set, when known.
                    let at = if field == Field::Status { g.status_at.clone().unwrap_or_default() } else { String::new() };
                    Stamp { clock: 0, at, device: me.id.clone() }
                }
            };
            lg.fields.insert(field.key().to_owned(), Stamped { value, stamp });
        }
        if publish_edits && lg.published_minutes.is_some_and(|before| g.playtime.minutes < before) {
            *clock += 1;
            lg.lowered = Some(Stamp { clock: *clock, at: now.to_owned(), device: me.id.clone() });
        }
    }
}

/// The stamp for `value` against what the ledger `held` (a custom category
/// or a smart group): a new one for a change made here, a weak one for a
/// value the ledger never saw (the weakest of all when this PC does not
/// publish its edits); `None` when nothing changed or there is nothing to say.
fn restamp(held: Option<&Stamped>, value: &Value, publish_edits: bool, fresh: impl FnOnce() -> Stamp, device: &str) -> Option<Stamp> {
    match held {
        Some(s) if s.value == *value => None,
        _ if !publish_edits => Some(Stamp::default()),
        Some(_) => Some(fresh()),
        None if value.is_null() => None,
        None => Some(Stamp { clock: 0, at: String::new(), device: device.to_owned() }),
    }
}

/// Takes the other PC's entries that are newer than the ledger's.
fn take_newer(mine: &mut BTreeMap<String, Stamped>, theirs: &BTreeMap<String, Stamped>) {
    for (key, t) in theirs {
        if mine.get(key).is_none_or(|m| t.stamp > m.stamp) {
            mine.insert(key.clone(), t.clone());
        }
    }
}

fn take_fields(lg: &mut LedgerGame, rg: &GameState) {
    for (key, theirs) in &rg.fields {
        if Field::parse(key).is_none() && category_of(key).is_none() {
            continue;
        }
        let newer = lg.fields.get(key).is_none_or(|mine| theirs.stamp > mine.stamp);
        if newer {
            lg.fields.insert(key.clone(), theirs.clone());
        }
    }
}

/// The other PC's sessions this one has not seen yet.
fn take_sessions(lg: &mut LedgerGame, g: &LocalGame, rg: &GameState) -> Vec<LocalGameSession> {
    let have: HashSet<&str> = g.sessions.iter().map(|s| s.started_at.as_str()).collect();
    let mut new = Vec::new();
    for s in &rg.sessions {
        if s.started_at.is_empty() || have.contains(s.started_at.as_str()) {
            continue;
        }
        if lg.imported.insert(s.started_at.clone()) {
            new.push(s.clone());
        }
    }
    new
}

/// The total: the largest among the PCs that know of the newest total made
/// smaller by hand (all of them when there is none). The first and last
/// play: the earliest and the latest.
fn merge_playtime(lg: &mut LedgerGame, mine: &Playtime, theirs: &[&Playtime]) -> Playtime {
    let newest = theirs.iter().filter_map(|p| p.lowered.as_ref()).chain(lg.lowered.as_ref()).max().cloned();
    let minutes = std::iter::once((lg.lowered.as_ref(), mine.minutes))
        .chain(theirs.iter().map(|p| (p.lowered.as_ref(), p.minutes)))
        .filter(|(lowered, _)| *lowered == newest.as_ref())
        .map(|(_, m)| m)
        .max()
        .unwrap_or(mine.minutes);
    let dates = |pick: fn(&Playtime) -> &Option<String>| {
        std::iter::once(mine).chain(theirs.iter().copied()).filter_map(move |p| pick(p).as_deref()).filter(|s| !s.is_empty())
    };
    let first_played = dates(|p| &p.first_played).min().map(str::to_owned).or_else(|| mine.first_played.clone());
    let last_played = dates(|p| &p.last_played).max().map(str::to_owned).or_else(|| mine.last_played.clone());
    lg.lowered = newest.clone();
    Playtime { minutes, first_played, last_played, lowered: newest }
}

fn own_file(local: &LocalLibrary, ledger: &Ledger, playtimes: &BTreeMap<String, Playtime>, me: &DeviceInfo, now: &str) -> DeviceFile {
    let keys: BTreeSet<&String> = ledger.games.keys().chain(local.games.keys()).collect();
    let mut games = BTreeMap::new();
    for key in keys {
        let (g, lg) = (local.games.get(key), ledger.games.get(key));
        let fields = lg.map(|lg| lg.fields.clone()).unwrap_or_default();
        let playtime = playtimes.get(key).filter(|p| p.minutes > 0 || p.first_played.is_some() || p.lowered.is_some()).cloned();
        let imported = lg.map(|lg| &lg.imported);
        let sessions: Vec<LocalGameSession> = g
            .map(|g| g.sessions.iter().filter(|s| imported.is_none_or(|i| !i.contains(&s.started_at))).cloned().collect())
            .unwrap_or_default();
        if fields.is_empty() && playtime.is_none() && sessions.is_empty() {
            continue;
        }
        let (runner, app_name, title) = match (g, lg) {
            (Some(g), _) => (g.runner.clone(), g.app_name.clone(), g.title.clone()),
            (None, Some(lg)) => (lg.runner.clone(), lg.app_name.clone(), lg.title.clone()),
            (None, None) => continue,
        };
        games.insert(key.clone(), GameState { runner, app_name, title, fields, playtime, sessions });
    }
    DeviceFile {
        format: FORMAT,
        device: me.clone(),
        written_at: now.to_owned(),
        clock: ledger.clock,
        games,
        statuses: local.custom_statuses.clone(),
        groups: ledger.groups.clone(),
    }
}

#[cfg(test)]
mod tests;
