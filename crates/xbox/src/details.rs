//! What the Xbox account says about the Collection's Xbox games, written
//! where the Collection reads it:
//!
//! * the minutes Xbox network counted and when the title was last played
//!   (the game's metadata, [`MINUTES_KEY`] and [`LAST_PLAYED_KEY`]).
//!   Heronite's own `totalPlayed` / `lastPlayed` (`store/timestamp.json`)
//!   stay as they are: the Collection shows the larger total and the later
//!   date, since Xbox's count already holds the sessions started here;
//! * achievements unlocked and gamerscore (the game's metadata,
//!   [`ACHIEVEMENTS_KEY`]);
//! * the title's image, when the entry has no art;
//! * the PC games played on the account that are not installed here, as
//!   entries not installed (never one the user took out);
//! * "Played" for the games with play time still in the automatic "Not
//!   played", as Epic's and GOG's play time does
//!   ([`heronite_data::status::promote_played`]).
//!
//! Titles are matched to entries by package family name, else by title id
//! (`xboxTitleId`), else by Store product id (`xboxStoreId`).

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile};
use heronite_data::status::{PlayedGame, SetStatusExtras};
use heronite_data::xbox::{XboxPrefs, ACHIEVEMENTS_KEY, LAST_PLAYED_KEY, LAUNCH_KIND, MINUTES_KEY, SOURCE, STORE_ID_KEY, TITLE_ID_KEY};
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Map, Value};

use crate::collection::app_name;
use crate::live::titlehub::Title;

/// What applying changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Applied {
    /// Entries whose play time, date, achievements or art changed.
    pub updated: usize,
    /// Games of the account added as not installed.
    pub added: usize,
    /// Games moved from the automatic "Not played" to "Played".
    pub promoted: usize,
}

impl Applied {
    /// Whether the Collection has anything new to show.
    pub fn any(&self) -> bool {
        self.updated + self.added + self.promoted > 0
    }
}

/// One Collection entry of the Xbox app, as matching needs it.
#[derive(Debug, Clone)]
struct Known {
    app: String,
    family: String,
    title_id: Option<String>,
    store_id: Option<String>,
}

fn known(metas: &LocalMetaFile) -> Vec<Known> {
    let text = |m: &LocalGameMeta, k: &str| m.extra.get(k).and_then(Value::as_str).map(str::to_string);
    metas
        .games
        .values()
        .filter(|m| m.source == SOURCE && m.launch_kind == LAUNCH_KIND)
        .map(|m| Known {
            app: m.app_name.clone(),
            family: m.store_game_id.clone().unwrap_or_default(),
            title_id: text(m, TITLE_ID_KEY),
            store_id: text(m, STORE_ID_KEY),
        })
        .collect()
}

/// The entry of `title`, if the Collection has one.
fn matched<'a>(title: &Title, known: &'a [Known]) -> Option<&'a Known> {
    let eq = |a: &str, b: &str| !a.is_empty() && a.eq_ignore_ascii_case(b);
    if let Some(pfn) = &title.pfn {
        if let Some(k) = known.iter().find(|k| eq(&k.family, pfn)) {
            return Some(k);
        }
    }
    if let Some(hex) = title.title_id_hex() {
        if let Some(k) = known.iter().find(|k| k.title_id.as_deref().is_some_and(|t| eq(t, &hex))) {
            return Some(k);
        }
    }
    let product = title.product_id.as_deref()?;
    known.iter().find(|k| k.store_id.as_deref().is_some_and(|s| eq(s, product)))
}

/// `{"unlocked", "total", "gamerscore", "totalGamerscore"}`.
fn achievements_value(title: &Title) -> Option<Value> {
    let a = title.achievements?;
    Some(json!({"unlocked": a.unlocked, "total": a.total, "gamerscore": a.gamerscore, "totalGamerscore": a.total_gamerscore}))
}

/// The name Xbox gives a PC title, without the platform it adds.
fn clean_name(name: &str) -> String {
    let mut out = name.to_string();
    for tag in ["(PC)", "(Windows)", "for Windows 10", "- Windows 10", "Windows 10 Edition"] {
        out = out.replace(tag, "");
    }
    out.replace(['™', '®'], "").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn not_installed_entry(app: &str, title: &Title) -> GameInfo {
    let mut info = GameInfo {
        runner: "sideload".into(),
        app_name: app.to_string(),
        title: clean_name(&title.name),
        is_installed: false,
        is_linux_native: Some(false),
        install: InstallInfo { platform: Some("windows".into()), is_dlc: Some(false), ..Default::default() },
        ..Default::default()
    };
    info.developer = title.developer.clone();
    info.description = title.description.clone();
    info.art_square = title.image.clone();
    info
}

/// Writes what `titles` (the account's, minutes filled in) say about the
/// Collection's Xbox games. Writes nothing that did not change.
pub fn apply(paths: &DataPaths, titles: &[Title]) -> Result<Applied> {
    let mut applied = Applied::default();
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let mut known = known(&metas);
    let prefs = XboxPrefs::load(paths);

    // The account's PC games the Collection does not have: added, not installed.
    let mut new: Vec<(String, &Title)> = Vec::new();
    for title in titles.iter().filter(|t| t.is_pc_game()) {
        let Some(pfn) = title.pfn.as_deref().filter(|p| p.contains('_')) else { continue };
        if matched(title, &known).is_some() || new.iter().any(|(_, t)| t.pfn.as_deref() == Some(pfn)) {
            continue;
        }
        let app = app_name(pfn);
        if prefs.removed.contains(&app) || prefs.removed_titles.contains(&app) {
            continue;
        }
        new.push((app, title));
    }
    if !new.is_empty() {
        let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
        json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
            for (app, title) in &new {
                if file.games.contains_key(app) {
                    continue;
                }
                let status = heronite_data::status::inferred_status_id(title.minutes.unwrap_or(0) as f64, &statuses);
                let mut meta = LocalGameMeta {
                    app_name: app.clone(),
                    runner: "sideload".into(),
                    source: SOURCE.into(),
                    launch_kind: LAUNCH_KIND.into(),
                    store_game_id: title.pfn.clone(),
                    title: clean_name(&title.name),
                    completion_status_id: Some(status),
                    ..Default::default()
                };
                if let Some(hex) = title.title_id_hex() {
                    meta.extra.insert(TITLE_ID_KEY.into(), json!(hex));
                }
                if let Some(p) = &title.product_id {
                    meta.extra.insert(STORE_ID_KEY.into(), json!(p));
                }
                file.games.insert(app.clone(), meta);
            }
            Ok(())
        })?;
        json_store::update_value_if_changed(&paths.sideload_library(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            let games = doc.as_object_mut().expect("object").entry("games").or_insert_with(|| json!([]));
            if !games.is_array() {
                *games = json!([]);
            }
            let list = games.as_array_mut().expect("array");
            for (app, title) in &new {
                if list.iter().any(|g| g.get("app_name").and_then(Value::as_str) == Some(app)) {
                    continue;
                }
                list.push(serde_json::to_value(not_installed_entry(app, title))?);
                applied.added += 1;
            }
            Ok(())
        })?;
        known.extend(new.iter().map(|(app, t)| Known {
            app: app.clone(),
            family: t.pfn.clone().unwrap_or_default(),
            title_id: None,
            store_id: None,
        }));
    }

    // Each title's entry.
    let pairs: Vec<(&Title, String)> = titles.iter().filter_map(|t| matched(t, &known).map(|k| (t, k.app.clone()))).collect();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for (title, app) in &pairs {
            let Some(meta) = file.games.get_mut(app) else { continue };
            let before = meta.extra.clone();
            set_known(&mut meta.extra, MINUTES_KEY, title.minutes.map(|m| json!(m)));
            set_known(&mut meta.extra, LAST_PLAYED_KEY, title.last_played.as_ref().map(|d| json!(d)));
            set_known(&mut meta.extra, ACHIEVEMENTS_KEY, achievements_value(title));
            if meta.extra.get(TITLE_ID_KEY).is_none() {
                if let Some(hex) = title.title_id_hex() {
                    meta.extra.insert(TITLE_ID_KEY.into(), json!(hex));
                }
            }
            if meta.extra != before {
                changed.insert(app.clone());
            }
        }
        Ok(())
    })?;
    // Art only where the entry has none.
    let images: HashMap<&str, &str> = pairs.iter().filter_map(|(t, app)| Some((app.as_str(), t.image.as_deref()?))).collect();
    if !images.is_empty() {
        json_store::update_value_if_changed(&paths.sideload_library(), |doc| {
            let Some(list) = doc.get_mut("games").and_then(Value::as_array_mut) else { return Ok(()) };
            for entry in list.iter_mut().filter_map(Value::as_object_mut) {
                let Some(app) = entry.get("app_name").and_then(Value::as_str).map(str::to_string) else { continue };
                let Some(image) = images.get(app.as_str()) else { continue };
                let empty = |k: &str| entry.get(k).and_then(Value::as_str).is_none_or(|s| s.trim().is_empty());
                if empty("art_square") && empty("art_cover") {
                    entry.insert("art_square".into(), json!(image));
                    changed.insert(app);
                }
            }
            Ok(())
        })?;
    }
    applied.updated = changed.len();
    applied.promoted = promote_played(paths)?;
    Ok(applied)
}

/// The Xbox games with play time (Heronite's or Xbox's, whichever is
/// larger) move from the automatic "Not played" to "Played"; a status the
/// user chose stays.
fn promote_played(paths: &DataPaths) -> Result<usize> {
    let library = heronite_data::Library::load_with(paths, true)?;
    let games: Vec<PlayedGame> = library
        .games
        .iter()
        .filter(|g| g.is_xbox() && g.playtime_minutes > 0)
        .map(|g| PlayedGame {
            app_name: g.app_name.clone(),
            minutes: g.playtime_minutes,
            extras: SetStatusExtras { runner: Some(g.runner.clone()), title: Some(g.title.clone()), playnite_id: None },
        })
        .collect();
    heronite_data::status::promote_played(paths, &games)
}

/// Sets `key` to what the account says now; a value it did not send this
/// time (a request that failed, a stat Xbox left out) keeps the last one.
fn set_known(obj: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value.filter(|v| obj.get(key) != Some(v)) {
        obj.insert(key.into(), v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::{mark_removed, sync};
    use crate::live::titlehub::{parse_titles, tests::HISTORY};
    use crate::XboxGame;
    use heronite_data::Library;

    fn installed(family: &str, title: &str, title_id: Option<&str>) -> XboxGame {
        XboxGame {
            family: family.into(),
            app_id: "Game".into(),
            title: title.into(),
            folder: std::env::temp_dir().join("XboxGames").join(title).join("Content"),
            store_id: None,
            title_id: title_id.map(str::to_string),
            package: format!("{family}_1.0.0.0_x64__"),
        }
    }

    /// The fixture's titles with the minutes `userstats` would give.
    fn titles() -> Vec<Title> {
        let mut t = parse_titles(&serde_json::from_str(HISTORY).unwrap());
        t[0].minutes = Some(754);
        t[1].minutes = Some(30);
        t
    }

    fn mtimes(paths: &DataPaths) -> [Option<std::time::SystemTime>; 3] {
        [json_store::mtime(&paths.local_meta()), json_store::mtime(&paths.sideload_library()), json_store::mtime(&paths.timestamps())]
    }

    #[test]
    fn matches_titles_and_shows_the_larger_play_time_and_the_later_play() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        // Skyward Lantern by package family; Tide Pool (no pfn in the history) by title id.
        let games = [
            installed("Fable.SkywardLantern_abc123", "Skyward Lantern", None),
            installed("Fable.TidePool_abc123", "Tide Pool", Some("1AB6124F")),
        ];
        sync(&paths, &games).unwrap();
        // Played here before: less than Xbox counted for Skyward Lantern; Tide Pool more, and more recently.
        let ts = json!({
            "xbox_fable_skywardlantern_abc123": {"firstPlayed": "2026-06-01T10:00:00.000Z", "lastPlayed": "2026-07-01T10:00:00.000Z", "totalPlayed": 60},
            "xbox_fable_tidepool_abc123": {"firstPlayed": "2026-06-01T10:00:00.000Z", "lastPlayed": "2026-11-06T10:00:00.000Z", "totalPlayed": 45}
        });
        json_store::write(&paths.timestamps(), &ts).unwrap();

        let applied = apply(&paths, &titles()).unwrap();
        assert_eq!(applied, Applied { updated: 3, added: 1, promoted: 2 }, "Cloud Harbor added; the console title and the app are not");

        let lib = Library::load(&paths).unwrap();
        let sky = lib.get("sideload_xbox_fable_skywardlantern_abc123").unwrap();
        assert_eq!(sky.playtime_minutes, 754, "Xbox counted more (its count holds Heronite's sessions)");
        assert_eq!(sky.last_played.as_deref(), Some("2026-11-05T21:14:03.1200000Z"));
        let m = sky.meta.as_ref().unwrap();
        assert_eq!(m.extra[ACHIEVEMENTS_KEY], json!({"unlocked": 12, "total": 40, "gamerscore": 210, "totalGamerscore": 1000}));
        assert_eq!(m.extra[TITLE_ID_KEY], json!("6A63F4E1"), "learnt from the account");
        assert_eq!(sky.game.art_square.as_deref(), Some("https://store-images.s-microsoft.com/image/apps.1.skyward"));

        let status = |key: &str| lib.get(key).unwrap().meta.as_ref().unwrap().completion_status_id.clone();
        assert_eq!(status("sideload_xbox_fable_skywardlantern_abc123").as_deref(), Some("played"), "played on Xbox: no longer Not played");
        assert_eq!(status("sideload_xbox_fable_tidepool_abc123").as_deref(), Some("played"));

        let tide = lib.get("sideload_xbox_fable_tidepool_abc123").unwrap();
        assert_eq!(tide.playtime_minutes, 45, "Heronite counted more");
        assert_eq!(tide.last_played.as_deref(), Some("2026-11-06T10:00:00.000Z"), "played here later");
        assert!(!tide.meta.as_ref().unwrap().extra.contains_key(ACHIEVEMENTS_KEY));

        // Heronite's own numbers are untouched.
        let ts = json_store::read_value(&paths.timestamps()).unwrap();
        assert_eq!(ts["xbox_fable_skywardlantern_abc123"]["totalPlayed"], 60);
        assert_eq!(ts["xbox_fable_skywardlantern_abc123"]["lastPlayed"], "2026-07-01T10:00:00.000Z");

        let harbor = lib.get("sideload_xbox_fable_cloudharbor_abc123").unwrap();
        assert!(!harbor.installed);
        assert_eq!(harbor.title, "Cloud Harbor", "without the platform tag");
        assert!(harbor.is_xbox());
        let hm = harbor.meta.as_ref().unwrap();
        assert_eq!(hm.store_game_id.as_deref(), Some("Fable.CloudHarbor_abc123"));
        assert_eq!(hm.extra[STORE_ID_KEY], json!("9CLOUDHARBOR"));
        assert_eq!(hm.launcher_args, None, "nothing to start until it is installed");
        assert_eq!(harbor.last_played.as_deref(), Some("2026-09-12T18:00:00Z"));
        assert_ne!(status("sideload_xbox_fable_cloudharbor_abc123").as_deref(), Some("played"), "no minutes known yet");
        assert!(lib.games.iter().all(|g| !g.title.contains("Console") && g.title != "Music Player"));

        // Again: nothing written.
        let before = mtimes(&paths);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(apply(&paths, &titles()).unwrap(), Applied::default());
        assert_eq!(mtimes(&paths), before);
    }

    #[test]
    fn a_later_package_scan_installs_the_same_entry() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        apply(&paths, &titles()).unwrap();
        // Cloud Harbor gets installed: the same entry, now installed and startable.
        sync(&paths, &[installed("Fable.CloudHarbor_abc123", "Cloud Harbor", None)]).unwrap();
        let lib = Library::load(&paths).unwrap();
        let harbor = lib.get("sideload_xbox_fable_cloudharbor_abc123").unwrap();
        assert!(harbor.installed);
        assert_eq!(harbor.meta.as_ref().unwrap().launcher_args.as_deref(), Some("Fable.CloudHarbor_abc123!Game"));
        assert_eq!(harbor.last_played.as_deref(), Some("2026-09-12T18:00:00Z"));
        assert_eq!(lib.games.iter().filter(|g| g.title.starts_with("Cloud Harbor")).count(), 1);
    }

    #[test]
    fn a_game_the_user_took_out_stays_out() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        mark_removed(&paths, "xbox_fable_cloudharbor_abc123").unwrap();
        // A package scan with nothing installed drops it from `removed`, not from `removedTitles`.
        sync(&paths, &[]).unwrap();
        assert_eq!(apply(&paths, &titles()).unwrap().added, 1, "Skyward Lantern only");
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_xbox_fable_cloudharbor_abc123").is_none());
        assert!(!lib.get("sideload_xbox_fable_skywardlantern_abc123").unwrap().installed);

        // Installed again and kept: the account may list it later.
        sync(&paths, &[installed("Fable.CloudHarbor_abc123", "Cloud Harbor", None)]).unwrap();
        assert!(XboxPrefs::load(&paths).removed_titles.is_empty());
    }

    #[test]
    fn missing_values_keep_the_last_ones_and_art_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        sync(&paths, &[installed("Fable.SkywardLantern_abc123", "Skyward Lantern", None)]).unwrap();
        json_store::update_value(&paths.sideload_library(), |doc| {
            doc["games"][0]["art_cover"] = json!("C:/covers/mine.png");
            Ok(())
        })
        .unwrap();
        apply(&paths, &titles()).unwrap();
        let mut later = titles();
        later[0].minutes = None;
        later[0].achievements = None;
        apply(&paths, &later).unwrap();
        let lib = Library::load(&paths).unwrap();
        let sky = lib.get("sideload_xbox_fable_skywardlantern_abc123").unwrap();
        assert_eq!(sky.playtime_minutes, 754);
        assert!(sky.meta.as_ref().unwrap().extra.contains_key(ACHIEVEMENTS_KEY));
        assert_eq!(sky.game.art_square, None, "the user's cover stays the only art");
        assert_eq!(sky.game.art_cover.as_deref(), Some("C:/covers/mine.png"));
    }

    #[test]
    fn names_lose_the_platform_tags() {
        assert_eq!(clean_name("Cloud Harbor (PC)"), "Cloud Harbor");
        assert_eq!(clean_name("Lantern\u{2122} Saga for Windows 10"), "Lantern Saga");
        assert_eq!(clean_name("Tide Pool - Windows 10"), "Tide Pool");
    }
}
