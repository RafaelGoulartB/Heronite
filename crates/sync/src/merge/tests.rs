use serde_json::{json, Value};

use super::*;
use crate::local::LocalGame;

/// A PC: its Collection and its ledger, synced in memory.
struct Pc {
    me: DeviceInfo,
    /// Publishes only its play.
    play_only: bool,
    local: LocalLibrary,
    ledger: Ledger,
    own: Option<DeviceFile>,
}

impl Pc {
    fn new(id: &str) -> Self {
        Pc {
            me: DeviceInfo { id: id.into(), name: id.into(), os: "linux".into(), app_version: String::new() },
            play_only: false,
            local: Default::default(),
            ledger: Default::default(),
            own: None,
        }
    }

    fn add(&mut self, key: &str) -> &mut LocalGame {
        let (runner, app_name) = key.split_once('_').unwrap();
        self.local.games.entry(key.into()).or_insert_with(|| LocalGame {
            runner: runner.into(),
            app_name: app_name.into(),
            title: app_name.into(),
            ..Default::default()
        })
    }

    fn game(&mut self, key: &str) -> &mut LocalGame {
        self.local.games.get_mut(key).unwrap()
    }

    fn set(&mut self, key: &str, field: Field, value: Value) {
        self.game(key).values.insert(field, value);
    }

    fn get(&self, key: &str, field: Field) -> Value {
        self.local.games[key].value(field)
    }

    fn play(&mut self, key: &str, started_at: &str, minutes: u64) {
        let g = self.game(key);
        g.sessions.push(session(started_at, minutes));
        g.playtime.minutes += minutes;
        g.playtime.first_played.get_or_insert_with(|| started_at.into());
        g.playtime.last_played = Some(started_at.into());
    }

    /// One sync against the others' files, written back like `local::write`.
    fn sync(&mut self, others: &[&Pc], now: &str) -> Summary {
        let remotes: Vec<DeviceFile> = others.iter().filter_map(|p| p.own.clone()).collect();
        let out = merge(&self.local, std::mem::take(&mut self.ledger), &remotes, &self.me, now, !self.play_only);
        for c in &out.changes.fields {
            self.local.games.get_mut(&c.key).unwrap().values.insert(c.field, c.value.clone());
        }
        for c in &out.changes.categories {
            let g = self.local.games.values_mut().find(|g| g.app_name == c.app_name).unwrap();
            if c.on {
                g.categories.insert(c.category.clone());
            } else {
                g.categories.remove(&c.category);
            }
        }
        // `local::write` stores only the groups this version reads.
        for c in out.changes.groups.iter().filter(|c| crate::local::storable_group(&c.id, &c.value)) {
            if c.value.is_null() {
                self.local.groups.remove(&c.id);
            } else {
                self.local.groups.insert(c.id.clone(), c.value.clone());
            }
        }
        for g in self.local.games.values_mut() {
            if let Some(new) = out.changes.sessions.get(&g.app_name) {
                g.sessions.extend(new.iter().cloned());
            }
            if let Some(p) = out.changes.playtime.get(&g.app_name) {
                g.playtime = Playtime { lowered: None, ..p.clone() };
            }
        }
        self.ledger = out.ledger;
        self.own = Some(out.own);
        out.summary
    }

    fn sessions(&self, key: &str) -> Vec<String> {
        let mut s: Vec<String> = self.local.games[key].sessions.iter().map(|s| s.started_at.clone()).collect();
        s.sort();
        s
    }
}

fn session(started_at: &str, minutes: u64) -> LocalGameSession {
    serde_json::from_value(json!({ "startedAt": started_at, "endedAt": started_at, "elapsedSeconds": minutes * 60, "source": "heroic" }))
        .unwrap()
}

const ELDEN: &str = "sideload_steam_1245620";
const EPIC: &str = "legendary_Fortnite";

/// Linux with history, Windows new with only Steam.
fn dual_boot() -> (Pc, Pc) {
    let mut linux = Pc::new("linux");
    linux.add(ELDEN);
    linux.set(ELDEN, Field::Status, json!("playing"));
    linux.set(ELDEN, Field::Favourite, json!(true));
    linux.play(ELDEN, "2026-10-01T20:00:00.000Z", 120);
    linux.add(EPIC);
    linux.set(EPIC, Field::Status, json!("beaten"));
    linux.play(EPIC, "2026-09-01T20:00:00.000Z", 600);
    let mut windows = Pc::new("windows");
    windows.add(ELDEN);
    // What the Steam import gives a new game.
    windows.set(ELDEN, Field::Status, json!("not-played"));
    (linux, windows)
}

#[test]
fn a_new_pc_gets_the_library_without_overwriting_it() {
    let (mut linux, mut windows) = dual_boot();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    let s = windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    assert_eq!(windows.get(ELDEN, Field::Status), json!("playing"));
    assert_eq!(windows.get(ELDEN, Field::Favourite), json!(true));
    assert_eq!(windows.sessions(ELDEN), ["2026-10-01T20:00:00.000Z"]);
    assert_eq!(windows.local.games[ELDEN].playtime.minutes, 120);
    // The Epic game is not on Windows: it waits, and nothing of it is lost.
    assert_eq!(s.waiting, 1);
    assert!(!windows.local.games.contains_key(EPIC));

    // Back on Linux: the default status of Windows did not win.
    let s = linux.sync(&[&windows], "2026-11-01T12:00:00.000Z");
    assert_eq!(s, Summary::default());
    assert_eq!(linux.get(ELDEN, Field::Status), json!("playing"));
    assert_eq!(linux.local.games[EPIC].playtime.minutes, 600);
}

#[test]
fn playing_on_windows_shows_up_on_linux() {
    let (mut linux, mut windows) = dual_boot();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");

    windows.play(ELDEN, "2026-11-01T20:00:00.000Z", 135);
    windows.set(ELDEN, Field::Status, json!("beaten"));
    windows.sync(&[&linux], "2026-11-01T22:10:00.000Z");

    let s = linux.sync(&[&windows], "2026-11-02T09:00:00.000Z");
    assert_eq!(linux.get(ELDEN, Field::Status), json!("beaten"));
    assert_eq!(linux.sessions(ELDEN), ["2026-10-01T20:00:00.000Z", "2026-11-01T20:00:00.000Z"]);
    assert_eq!(linux.local.games[ELDEN].playtime.minutes, 255);
    assert_eq!(linux.local.games[ELDEN].playtime.last_played.as_deref(), Some("2026-11-01T20:00:00.000Z"));
    assert_eq!((s.fields, s.sessions, s.playtime), (1, 1, 1));

    // Linux does not publish Windows' session as its own.
    let own = linux.own.as_ref().unwrap();
    assert_eq!(own.games[ELDEN].sessions.len(), 1);
    // Nothing moves on a second sync.
    assert!(!linux.sync(&[&windows], "2026-11-02T09:05:00.000Z").changed());
    assert!(!windows.sync(&[&linux], "2026-11-02T09:06:00.000Z").changed());
}

#[test]
fn a_change_made_after_reading_the_other_pc_wins_even_with_a_wrong_clock() {
    let (mut linux, mut windows) = dual_boot();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    // Windows' clock is three hours ahead.
    windows.set(ELDEN, Field::Status, json!("on-hold"));
    windows.sync(&[&linux], "2026-11-01T14:00:00.000Z");
    linux.sync(&[&windows], "2026-11-01T11:00:00.000Z");
    assert_eq!(linux.get(ELDEN, Field::Status), json!("on-hold"));
    linux.set(ELDEN, Field::Status, json!("abandoned"));
    linux.sync(&[&windows], "2026-11-01T11:30:00.000Z");
    windows.sync(&[&linux], "2026-11-01T14:40:00.000Z");
    assert_eq!(windows.get(ELDEN, Field::Status), json!("abandoned"));
}

#[test]
fn turning_a_flag_off_syncs_too() {
    let (mut linux, mut windows) = dual_boot();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    windows.set(ELDEN, Field::Favourite, Value::Null);
    windows.set(ELDEN, Field::Rating, json!(4));
    windows.sync(&[&linux], "2026-11-01T12:00:00.000Z");
    linux.sync(&[&windows], "2026-11-01T13:00:00.000Z");
    assert_eq!(linux.get(ELDEN, Field::Favourite), Value::Null);
    assert_eq!(linux.get(ELDEN, Field::Rating), json!(4));
}

#[test]
fn a_session_deleted_here_does_not_come_back() {
    let (mut linux, mut windows) = dual_boot();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    windows.game(ELDEN).sessions.clear();
    assert_eq!(windows.sync(&[&linux], "2026-11-01T12:00:00.000Z").sessions, 0);
    assert!(windows.sessions(ELDEN).is_empty());
}

#[test]
fn the_same_sessions_on_both_pcs_are_not_doubled() {
    // A data folder copied from one PC to the other.
    let (mut linux, _) = dual_boot();
    let mut copy = Pc::new("copy");
    copy.local = linux.local.clone();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    copy.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    linux.sync(&[&copy], "2026-11-01T12:00:00.000Z");
    assert_eq!(linux.sessions(ELDEN).len(), 1);
    assert_eq!(copy.sessions(ELDEN).len(), 1);
    assert_eq!(linux.local.games[ELDEN].playtime.minutes, 120);
}

#[test]
fn store_play_time_on_both_pcs_is_not_counted_twice() {
    let (mut linux, mut windows) = dual_boot();
    // Steam's own total (every PC's play) taken in on both.
    linux.game(ELDEN).playtime.minutes = 6000;
    windows.game(ELDEN).playtime.minutes = 6000;
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    assert_eq!(windows.local.games[ELDEN].playtime.minutes, 6000);
    windows.play(ELDEN, "2026-11-01T20:00:00.000Z", 60);
    windows.sync(&[&linux], "2026-11-01T21:30:00.000Z");
    linux.sync(&[&windows], "2026-11-02T09:00:00.000Z");
    assert_eq!(linux.local.games[ELDEN].playtime.minutes, 6060);
}

#[test]
fn play_time_made_smaller_by_hand_is_followed() {
    let (mut linux, mut windows) = dual_boot();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    linux.sync(&[&windows], "2026-11-01T11:30:00.000Z");
    // Fixed by hand on Linux: 120 → 30.
    linux.game(ELDEN).playtime.minutes = 30;
    linux.sync(&[&windows], "2026-11-01T12:00:00.000Z");
    assert_eq!(linux.local.games[ELDEN].playtime.minutes, 30, "Windows' older total must not undo it");
    windows.sync(&[&linux], "2026-11-01T13:00:00.000Z");
    assert_eq!(windows.local.games[ELDEN].playtime.minutes, 30);
    // Playing afterwards adds up again.
    windows.play(ELDEN, "2026-11-01T20:00:00.000Z", 10);
    windows.sync(&[&linux], "2026-11-01T21:00:00.000Z");
    linux.sync(&[&windows], "2026-11-02T09:00:00.000Z");
    assert_eq!(linux.local.games[ELDEN].playtime.minutes, 40);
}

#[test]
fn a_status_set_on_each_pc_before_syncing_keeps_the_latest_known_one() {
    let mut linux = Pc::new("linux");
    linux.add(ELDEN);
    linux.set(ELDEN, Field::Status, json!("playing"));
    linux.game(ELDEN).status_at = Some("2026-01-01T00:00:00.000Z".into());
    let mut windows = Pc::new("windows");
    windows.add(ELDEN);
    windows.set(ELDEN, Field::Status, json!("beaten"));
    windows.game(ELDEN).status_at = Some("2026-06-01T00:00:00.000Z".into());
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    linux.sync(&[&windows], "2026-11-01T12:00:00.000Z");
    assert_eq!(linux.get(ELDEN, Field::Status), json!("beaten"));
    assert_eq!(windows.get(ELDEN, Field::Status), json!("beaten"));
}

#[test]
fn a_game_that_arrives_later_gets_its_data() {
    let (mut linux, mut windows) = dual_boot();
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    // Epic signed in on Windows a week later.
    windows.add(EPIC);
    let s = windows.sync(&[&linux], "2026-11-08T11:00:00.000Z");
    assert_eq!(s.waiting, 0);
    assert_eq!(windows.get(EPIC, Field::Status), json!("beaten"));
    assert_eq!(windows.local.games[EPIC].playtime.minutes, 600);
}

#[test]
fn custom_statuses_travel_with_the_games() {
    let (mut linux, mut windows) = dual_boot();
    let custom = heronite_data::models::CompletionStatus {
        id: "custom-1".into(),
        name: "Replaying".into(),
        slug: "custom".into(),
        ..Default::default()
    };
    linux.local.custom_statuses.push(custom);
    linux.set(ELDEN, Field::Status, json!("custom-1"));
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    let out = merge(&windows.local, Ledger::default(), &[linux.own.clone().unwrap()], &windows.me, "2026-11-01T11:00:00.000Z", true);
    assert_eq!(out.changes.statuses.len(), 1);
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    assert_eq!(windows.get(ELDEN, Field::Status), json!("custom-1"));
}

#[test]
fn a_play_only_pc_sends_its_play_and_none_of_its_edits() {
    let (mut linux, mut windows) = dual_boot();
    windows.play_only = true;
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    assert_eq!(windows.get(ELDEN, Field::Status), json!("playing"));

    // Careless on Windows: a status, a hidden game, the play time cut down.
    windows.set(ELDEN, Field::Status, json!("abandoned"));
    windows.set(ELDEN, Field::Hidden, json!(true));
    windows.game(ELDEN).playtime.minutes = 5;
    windows.play(ELDEN, "2026-11-01T20:00:00.000Z", 60);
    windows.sync(&[&linux], "2026-11-01T21:30:00.000Z");
    let own = windows.own.as_ref().unwrap();
    assert!(own.games[ELDEN].fields.is_empty());

    linux.sync(&[&windows], "2026-11-02T09:00:00.000Z");
    assert_eq!(linux.get(ELDEN, Field::Status), json!("playing"));
    assert_eq!(linux.get(ELDEN, Field::Hidden), Value::Null);
    assert_eq!(linux.sessions(ELDEN), ["2026-10-01T20:00:00.000Z", "2026-11-01T20:00:00.000Z"]);
    assert_eq!(linux.local.games[ELDEN].playtime.minutes, 120, "Linux' total is larger");

    // Windows takes Linux' values back.
    windows.sync(&[&linux], "2026-11-02T10:00:00.000Z");
    assert_eq!(windows.get(ELDEN, Field::Status), json!("playing"));
}

fn categories(pc: &Pc, key: &str) -> Vec<String> {
    pc.local.games[key].categories.iter().cloned().collect()
}

#[test]
fn categories_added_on_two_pcs_both_stay() {
    let (mut linux, mut windows) = dual_boot();
    linux.game(ELDEN).categories.insert("Souls".into());
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    assert_eq!(categories(&windows, ELDEN), ["Souls"], "a new PC takes the categories in");

    // Both add the game to another category before syncing again.
    linux.game(ELDEN).categories.insert("RPG".into());
    linux.sync(&[&windows], "2026-11-02T10:00:00.000Z");
    windows.game(ELDEN).categories.insert("Open world".into());
    let s = windows.sync(&[&linux], "2026-11-02T11:00:00.000Z");
    assert_eq!(s.fields, 1);
    linux.sync(&[&windows], "2026-11-02T12:00:00.000Z");
    for pc in [&linux, &windows] {
        assert_eq!(categories(pc, ELDEN), ["Open world", "RPG", "Souls"]);
    }

    // Taken out on Windows, out on Linux too, and it stays out.
    windows.game(ELDEN).categories.remove("Souls");
    windows.sync(&[&linux], "2026-11-03T10:00:00.000Z");
    linux.sync(&[&windows], "2026-11-03T11:00:00.000Z");
    windows.sync(&[&linux], "2026-11-03T12:00:00.000Z");
    for pc in [&linux, &windows] {
        assert_eq!(categories(pc, ELDEN), ["Open world", "RPG"]);
    }
}

fn group(name: &str) -> Value {
    json!({ "id": "g1", "name": name, "colour": null, "rule": { "all": [ { "field": "installed", "op": "is", "value": true } ] } })
}

#[test]
fn smart_groups_travel_change_and_stay_deleted() {
    let (mut linux, mut windows) = dual_boot();
    linux.local.groups.insert("g1".into(), group("Weekend"));
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    let s = windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    assert_eq!(s.groups, 1);
    assert_eq!(windows.local.groups["g1"], group("Weekend"));

    // Renamed on Windows: the newest change wins on Linux.
    windows.local.groups.insert("g1".into(), group("Weekend games"));
    windows.sync(&[&linux], "2026-11-02T10:00:00.000Z");
    linux.sync(&[&windows], "2026-11-02T11:00:00.000Z");
    assert_eq!(linux.local.groups["g1"], group("Weekend games"));

    // Deleted on Linux: gone on Windows, and Windows' old copy does not bring it back.
    linux.local.groups.remove("g1");
    linux.sync(&[&windows], "2026-11-03T10:00:00.000Z");
    let s = windows.sync(&[&linux], "2026-11-03T11:00:00.000Z");
    assert_eq!(s.groups, 1);
    assert!(windows.local.groups.is_empty());
    linux.sync(&[&windows], "2026-11-03T12:00:00.000Z");
    assert!(linux.local.groups.is_empty());
    assert_eq!(linux.own.as_ref().unwrap().groups["g1"].value, Value::Null, "published as deleted");
}

#[test]
fn a_play_only_pc_publishes_no_groups_or_categories() {
    let (mut linux, mut windows) = dual_boot();
    windows.play_only = true;
    windows.local.groups.insert("g1".into(), group("Weekend"));
    windows.game(ELDEN).categories.insert("RPG".into());
    windows.sync(&[], "2026-11-01T10:00:00.000Z");
    let own = windows.own.as_ref().unwrap();
    assert!(own.groups.is_empty());
    assert!(own.games.get(ELDEN).is_none_or(|g| g.fields.is_empty()));
    linux.sync(&[&windows], "2026-11-01T11:00:00.000Z");
    assert!(linux.local.groups.is_empty());
    assert!(categories(&linux, ELDEN).is_empty());
}

#[test]
fn a_group_this_pc_cant_read_is_not_a_change_on_every_sync() {
    let (mut linux, mut windows) = dual_boot();
    // A blank name: a newer version's group this one rejects.
    linux.local.groups.insert("g1".into(), json!({ "id": "g1", "name": "", "rule": { "all": [] } }));
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    for at in ["2026-11-01T11:00:00.000Z", "2026-11-01T12:00:00.000Z"] {
        assert_eq!(windows.sync(&[&linux], at).groups, 0);
    }
    assert!(windows.local.groups.is_empty());
    // Nor does this PC publish it as deleted.
    linux.sync(&[&windows], "2026-11-01T13:00:00.000Z");
    assert_eq!(linux.local.groups["g1"]["name"], "");
}

#[test]
fn a_group_this_pc_cant_read_does_not_lose_to_its_older_copy() {
    let (mut linux, mut windows) = dual_boot();
    let older = json!({ "id": "g1", "name": "W", "colour": null, "rule": { "all": [] } });
    linux.local.groups.insert("g1".into(), older.clone());
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    assert_eq!(windows.local.groups["g1"], older);
    // A newer version changes it to something this one rejects.
    let newer = json!({ "id": "g1", "name": "", "rule": { "all": [] } });
    linux.local.groups.insert("g1".into(), newer.clone());
    linux.sync(&[&windows], "2026-11-01T12:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T13:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T14:00:00.000Z");
    assert_eq!(windows.local.groups["g1"], older, "kept as this version read it");
    linux.sync(&[&windows], "2026-11-01T15:00:00.000Z");
    assert_eq!(linux.local.groups["g1"], newer);
}

#[test]
fn a_newer_conditions_key_survives_a_round_trip_through_an_older_pc() {
    use heronite_data::smart_groups::SmartGroup;
    let (mut linux, mut windows) = dual_boot();
    let newer = json!({ "id": "g1", "name": "W", "colour": null,
        "rule": { "all": [ { "field": "playtime", "op": "gt", "value": 5.0, "unit": "hours" } ] } });
    linux.local.groups.insert("g1".into(), newer.clone());
    linux.sync(&[], "2026-11-01T10:00:00.000Z");
    windows.sync(&[&linux], "2026-11-01T11:00:00.000Z");
    // What `local::read` gives after `local::write`.
    let stored = crate::local::shared_group(&SmartGroup::from_value(&windows.local.groups["g1"]).unwrap());
    windows.local.groups.insert("g1".into(), stored);
    windows.sync(&[&linux], "2026-11-01T12:00:00.000Z");
    linux.sync(&[&windows], "2026-11-01T13:00:00.000Z");
    assert_eq!(linux.local.groups["g1"], newer);
}
