use super::*;
use crate::library::tests_support::{game, game_with};
use crate::num::JsNum;

/// A copy from `runner` titled `title`, with an IGDB id and a Steam app id.
fn copy(runner: &str, app: &str, title: &str, igdb: Option<f64>, steam: Option<&str>) -> Arc<CollectionGame> {
    game_with(app, runner, |info, _, md, meta, _| {
        info.title = title.into();
        md.igdb_id = igdb.map(JsNum);
        md.steam_app_id = steam.map(str::to_string);
        meta.source = String::new();
        meta.launch_kind = String::new();
    })
}

fn installed(g: Arc<CollectionGame>) -> Arc<CollectionGame> {
    let mut info = (*g.game).clone();
    info.is_installed = true;
    game_with(&g.app_name, &g.runner, |i, _, md, meta, _| {
        *i = info;
        *md = g.metadata.as_deref().cloned().unwrap_or_default();
        *meta = g.meta.as_deref().cloned().unwrap_or_default();
    })
}

fn keys(groups: &CopyGroups) -> Vec<Vec<&str>> {
    let mut v: Vec<Vec<&str>> = groups.groups.iter().map(|g| g.members.iter().map(String::as_str).collect()).collect();
    v.sort();
    v
}

#[test]
fn a_shared_id_and_title_link_copies_of_different_stores() {
    let games = [
        copy("gog", "1", "The Witcher® 3", Some(1942.0), None),
        copy("legendary", "w3", "the witcher 3", Some(1942.0), None),
        copy("nile", "a", "Hades", None, Some("1145360")),
        copy("gog", "2", "HADES™", None, Some("1145360")),
    ];
    let g = groups(&games, &CopiesFile::default());
    assert_eq!(keys(&g), [vec!["gog_1", "legendary_w3"], vec!["gog_2", "nile_a"]]);
    assert!(!g.get("gog_1").unwrap().chosen && !g.get("gog_1").unwrap().manual);
}

#[test]
fn ids_alone_titles_alone_editions_and_one_store_never_link() {
    let games = [
        // Same IGDB id, other titles: an edition, or a fuzzy IGDB match.
        copy("gog", "1", "Fallout 3", Some(10.0), None),
        copy("legendary", "f3", "Fallout 3 Game of the Year Edition", Some(10.0), None),
        // Same title, no shared id.
        copy("gog", "2", "Portal", Some(1.0), None),
        copy("legendary", "p", "Portal", Some(2.0), None),
        copy("nile", "p", "Portal", None, None),
        // Same store twice: two editions the store sells apart.
        copy("legendary", "x1", "Control", Some(5.0), None),
        copy("legendary", "x2", "Control", Some(5.0), None),
    ];
    assert!(groups(&games, &CopiesFile::default()).is_empty());
}

#[test]
fn editions_with_a_shared_id_are_offered_not_linked() {
    let games = [
        copy("gog", "1", "Fallout 3", Some(10.0), None),
        copy("legendary", "f3", "Fallout 3 Game of the Year Edition", Some(10.0), None),
        copy("nile", "f3", "Fallout 3", Some(10.0), None),
        copy("legendary", "other", "Fallout 3", Some(10.0), None),
    ];
    let g = groups(&games[..3], &CopiesFile::default());
    assert_eq!(keys(&g), [vec!["gog_1", "nile_f3"]]);
    assert_eq!(g.suggested("nile_f3"), ["legendary_f3"]);
    assert_eq!(g.suggested("legendary_f3"), ["gog_1", "nile_f3"]);
    // Ungrouped pairs are not offered again.
    let file = CopiesFile {
        apart: vec![["gog_1".into(), "legendary_f3".into()], ["nile_f3".into(), "legendary_f3".into()]],
        ..Default::default()
    };
    assert!(groups(&games[..3], &file).suggested("gog_1").is_empty());
    // An id that many entries share (a wrong match) offers nothing.
    let many = [
        games[0].clone(),
        games[1].clone(),
        games[3].clone(),
        copy("xbox", "f", "F3", Some(10.0), None),
        copy("amazon", "f", "F", Some(10.0), None),
    ];
    assert!(groups(&many, &CopiesFile::default()).suggested("gog_1").is_empty());
}

#[test]
fn emulated_and_browser_games_never_link_by_themselves() {
    let emu = game_with("rom", "sideload", |info, _, md, meta, _| {
        info.title = "Celeste".into();
        md.igdb_id = Some(JsNum(3.0));
        meta.launch_kind = "emulator".into();
        meta.source = "emulator".into();
    });
    let web = game_with("web", "sideload", |info, _, md, _, _| {
        info.title = "Celeste".into();
        info.install.platform = Some("Browser".into());
        md.igdb_id = Some(JsNum(3.0));
    });
    let games = [emu, web, copy("gog", "c", "Celeste", Some(3.0), None)];
    assert!(groups(&games, &CopiesFile::default()).is_empty());
}

#[test]
fn an_ungrouped_pair_stays_apart_and_a_manual_link_joins_anything() {
    let games = [
        copy("gog", "1", "Fallout 3", Some(10.0), None),
        copy("legendary", "f3", "Fallout 3", Some(10.0), None),
        copy("nile", "goty", "Fallout 3 GOTY", None, None),
    ];
    let file = CopiesFile { apart: vec![["legendary_f3".into(), "gog_1".into()]], ..Default::default() };
    assert!(groups(&games, &file).is_empty());

    let file = CopiesFile { links: vec![vec!["gog_1".into(), "nile_goty".into(), "gone_key".into()]], ..Default::default() };
    let g = groups(&games, &file);
    assert_eq!(keys(&g), [vec!["gog_1", "legendary_f3", "nile_goty"]]);
    assert!(g.get("nile_goty").unwrap().manual);
}

#[test]
fn the_default_copy_is_the_chosen_one_else_installed_else_by_store() {
    let gog = copy("gog", "1", "Hades", Some(7.0), None);
    let epic = copy("legendary", "h", "Hades", Some(7.0), None);
    let amazon = copy("nile", "h", "Hades", Some(7.0), None);
    let games = [amazon.clone(), epic.clone(), gog.clone()];
    let first = |games: &[Arc<CollectionGame>], file: &CopiesFile| groups(games, file).groups[0].members[0].clone();
    assert_eq!(first(&games, &CopiesFile::default()), "gog_1", "store order");
    let games = [installed(amazon), epic, gog];
    assert_eq!(first(&games, &CopiesFile::default()), "nile_h", "installed first");
    let file = CopiesFile { preferred: vec!["legendary_h".into()], ..Default::default() };
    assert_eq!(first(&games, &file), "legendary_h", "the user's choice wins");
    assert!(groups(&games, &file).groups[0].chosen);
}

#[test]
fn a_copy_the_steam_family_lends_is_never_the_default_over_an_owned_one() {
    // The Steam entry of a game another member shares (Steam ranks first by store).
    let family = |is_installed: bool| {
        let mut g = Arc::try_unwrap(game_with("steam_1145360", "sideload", |info, _, md, meta, _| {
            info.title = "Hades".into();
            info.is_installed = is_installed;
            md.steam_app_id = Some("1145360".into());
            meta.steam_app_id = Some("1145360".into());
            meta.launch_kind = "steam-uri".into();
        }))
        .unwrap();
        g.family = Some(crate::steam_owned::FamilyShare { owner_id: crate::steam_owned::steamid64(1001), ..Default::default() });
        Arc::new(g)
    };
    let gog = copy("gog", "1", "Hades", None, Some("1145360"));
    let first = |games: &[Arc<CollectionGame>], file: &CopiesFile| groups(games, file).groups[0].members[0].clone();
    assert_eq!(first(&[family(false), gog.clone()], &CopiesFile::default()), "gog_1", "the user's own copy plays");
    // Installed only through the Family, or chosen: it plays.
    let file = CopiesFile { preferred: vec!["sideload_steam_1145360".into()], ..Default::default() };
    assert_eq!(first(&[family(false), gog.clone()], &file), "sideload_steam_1145360");
    assert_eq!(first(&[family(true), gog], &CopiesFile::default()), "sideload_steam_1145360");
}

#[test]
fn collapse_shows_one_copy_per_group_the_best_one_the_filter_kept() {
    let games = [
        copy("gog", "1", "Hades", Some(7.0), None),
        copy("legendary", "h", "Hades", Some(7.0), None),
        copy("gog", "2", "Celeste", None, None),
    ];
    let g = groups(&games, &CopiesFile::default());
    let shown = |list: Vec<Arc<CollectionGame>>| g.collapse(list).iter().map(|g| g.key.clone()).collect::<Vec<_>>();
    assert_eq!(shown(games.to_vec()), ["gog_1", "gog_2"]);
    // A filter that keeps only the Epic copy shows it.
    assert_eq!(shown(vec![games[1].clone(), games[2].clone()]), ["legendary_h", "gog_2"]);
    // The representative keeps its place in the sorted list.
    assert_eq!(shown(vec![games[2].clone(), games[1].clone(), games[0].clone()]), ["gog_2", "gog_1"]);
}

#[test]
fn totals_sum_the_copies_once_each() {
    let a = game("1", "Hades", |s| {
        s.runner = Some("gog".into());
        s.minutes = 60;
        s.last_played = Some("2026-01-02T00:00:00.000Z".into());
        s.first_played = Some("2025-01-01T00:00:00.000Z".into());
    });
    let b = game("h", "Hades", |s| {
        s.runner = Some("legendary".into());
        s.minutes = 30;
        s.last_played = Some("2026-03-01T00:00:00.000Z".into());
    });
    // Two entries that start the same Steam install: one play, counted once.
    let c = game("steam_9", "Hades", |s| {
        s.steam_launch = Some("9".into());
        s.minutes = 100;
    });
    let d = game("playnite_9", "Hades", |s| {
        s.steam_launch = Some("9".into());
        s.minutes = 90;
    });
    assert_eq!(c.steam_launch_id.as_deref(), Some("9"));
    let t = totals([&*a, &*b, &*c, &*d]);
    assert_eq!(t.playtime_minutes, 190);
    assert_eq!(t.last_played.as_deref(), Some("2026-03-01T00:00:00.000Z"));
    assert_eq!(t.first_played.as_deref(), Some("2025-01-01T00:00:00.000Z"));
}

fn paths() -> (tempfile::TempDir, DataPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    (dir, paths)
}

#[test]
fn no_file_means_no_decisions_and_a_broken_one_too() {
    let (_dir, paths) = paths();
    assert_eq!(CopiesFile::read(&paths), CopiesFile::default());
    std::fs::create_dir_all(paths.local_library()).unwrap();
    std::fs::write(paths.copies(), "{ not json").unwrap();
    assert_eq!(CopiesFile::read(&paths), CopiesFile::default());
}

#[test]
fn decisions_round_trip_and_keep_unknown_fields() {
    let (_dir, paths) = paths();
    std::fs::create_dir_all(paths.local_library()).unwrap();
    std::fs::write(paths.copies(), r#"{"future":{"x":1},"links":[["a","b"]]}"#).unwrap();
    link(&paths, &["b".into(), "c".into()]).unwrap();
    set_preferred(&paths, "c", &["a".into(), "b".into(), "c".into()]).unwrap();
    let file = CopiesFile::read(&paths);
    assert_eq!(file.links, [vec!["a", "b", "c"]]);
    assert_eq!(file.preferred, ["c"]);
    assert_eq!(file.extra["future"], serde_json::json!({"x": 1}));

    set_preferred(&paths, "a", &["a".into(), "b".into(), "c".into()]).unwrap();
    assert_eq!(CopiesFile::read(&paths).preferred, ["a"]);

    unlink(&paths, &["a".into(), "b".into(), "c".into()]).unwrap();
    let file = CopiesFile::read(&paths);
    assert!(file.links.is_empty() && file.preferred.is_empty());
    assert_eq!(file.apart.len(), 3);
    assert_eq!(file.extra["future"], serde_json::json!({"x": 1}));

    // Linking again by hand lifts the ban for those pairs.
    link(&paths, &["a".into(), "b".into()]).unwrap();
    let file = CopiesFile::read(&paths);
    assert_eq!(file.apart, [["a".to_string(), "c".to_string()], ["b".to_string(), "c".to_string()]]);
}

#[test]
fn groups_survive_reloads_and_refreshes_and_every_record_stays() {
    use crate::library::Library;
    let (_dir, paths) = paths();
    let w = |p: std::path::PathBuf, s: &str| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    };
    let gog = r#"{"games":[{"runner":"gog","app_name":"1","title":"Hades","install":{}}],"__timestamp":{"games":"x"}}"#;
    w(paths.gog_library(), gog);
    w(
        paths.legendary_library(),
        r#"{"library":[{"runner":"legendary","app_name":"h","title":"HADES","install":{}},
                       {"runner":"legendary","app_name":"c","title":"Celeste","install":{}}],
            "__timestamp":{"library":"x"}}"#,
    );
    w(paths.gog_installed(), r#"{"installed":[{"appName":"1","install_path":"/games/hades","platform":"windows"}]}"#);
    w(
        paths.metadata(),
        r#"{"games":{"gog:1":{"appName":"1","runner":"gog","igdbId":113},
                     "legendary:h":{"appName":"h","runner":"legendary","igdbId":113}}}"#,
    );
    w(paths.timestamps(), r#"{"1":{"totalPlayed":60},"h":{"totalPlayed":45,"lastPlayed":"2026-05-01T00:00:00.000Z"}}"#);
    let sessions = r#"{"sessions":{"h":[{"startedAt":"2026-05-01T00:00:00.000Z","endedAt":"2026-05-01T00:45:00.000Z","elapsedSeconds":2700,"source":"heroic"}]}}"#;
    w(paths.sessions(), sessions);

    // No copies.json (a library from before this feature): found by id and title.
    let lib = Library::load(&paths).unwrap();
    assert_eq!(lib.games.len(), 3, "both copies stay entries of their own");
    let order: Vec<&str> = lib.copies_of("legendary_h").iter().map(|g| g.key.as_str()).collect();
    assert_eq!(order, ["gog_1", "legendary_h"], "the installed copy first");
    assert_eq!(lib.launch_copy("legendary_h").unwrap().key, "gog_1");
    assert_eq!(lib.copy_totals("gog_1").playtime_minutes, 105);
    assert_eq!(lib.copies_of("legendary_c").len(), 1);
    assert_eq!(lib.launch_copy("legendary_c").unwrap().key, "legendary_c");

    // The user's choice, then a store refresh that rewrites the libraries.
    set_preferred(&paths, "legendary_h", &["gog_1".into(), "legendary_h".into()]).unwrap();
    w(paths.gog_library(), gog);
    let lib = Library::load(&paths).unwrap();
    assert!(lib.copies.get("gog_1").unwrap().chosen);
    assert_eq!(lib.copies_of("gog_1")[0].key, "legendary_h");
    // The default copy is not installed: Play starts the installed one.
    assert_eq!(lib.launch_copy("legendary_h").unwrap().key, "gog_1");

    // A game that leaves the library (and comes back) keeps its place.
    w(paths.gog_library(), r#"{"games":[],"__timestamp":{"games":"x"}}"#);
    let lib = Library::load(&paths).unwrap();
    assert!(lib.copies.is_empty());
    w(paths.gog_library(), gog);
    let lib = Library::load(&paths).unwrap();
    assert_eq!(lib.copies_of("gog_1")[0].key, "legendary_h");

    // Ungrouping keeps the entries and never relinks them.
    unlink(&paths, &["gog_1".into(), "legendary_h".into()]).unwrap();
    let lib = Library::load(&paths).unwrap();
    assert!(lib.copies.is_empty());
    assert_eq!(lib.games.len(), 3);
    assert_eq!(lib.get("gog_1").unwrap().playtime_minutes, 60);
    assert_eq!(lib.get("legendary_h").unwrap().playtime_minutes, 45);
    assert_eq!(lib.get("legendary_h").unwrap().sessions.as_ref().map(|s| s.len()), Some(1));
    // No other file was written.
    assert_eq!(std::fs::read_to_string(paths.sessions()).unwrap(), sessions);
}

/// A ROM entry of `emulator` from `file`.
fn rom(app: &str, title: &str, emulator: &str, file: &str) -> Arc<CollectionGame> {
    game_with(app, "sideload", |info, _, _, meta, _| {
        info.title = title.into();
        meta.source = "emulator".into();
        meta.launch_kind = "emulator".into();
        meta.emulator_name = Some(emulator.into());
        meta.roms = Some(vec![crate::models::LocalGameRom { name: None, path: file.into(), extra: Default::default() }]);
    })
}

fn details(copies: &[&Arc<CollectionGame>]) -> Vec<Option<String>> {
    let copies: Vec<&CollectionGame> = copies.iter().map(|g| &***g).collect();
    copy_details(&copies)
}

#[test]
fn copies_of_one_title_need_only_their_store() {
    let (gog, epic) = (copy("gog", "1", "Hades", None, None), copy("legendary", "h", "HADES™", None, None));
    assert_eq!(details(&[&gog, &epic]), [None, None]);
    assert_eq!(store_label(&gog), "GOG");
}

#[test]
fn an_edition_or_a_region_shows_on_every_copy() {
    let (gog, epic) =
        (copy("gog", "1", "Fallout 3", None, None), copy("legendary", "f", "Fallout 3: Game of the Year Edition", None, None));
    assert_eq!(details(&[&gog, &epic]), [Some("Fallout 3".into()), Some("Fallout 3: Game of the Year Edition".into())]);

    let usa = rom("usa", "Advance Wars (USA)", "mGBA", "/roms/gba/Advance Wars (USA).gba");
    let eur = rom("eur", "Advance Wars (Europe)", "mGBA", "/roms/gba/Advance Wars (Europe).gba");
    assert_eq!(details(&[&usa, &eur]), [Some("Advance Wars (USA)".into()), Some("Advance Wars (Europe)".into())]);
    assert_eq!(store_label(&usa), "mGBA");
}

#[test]
fn roms_of_one_title_show_their_files() {
    let a = rom("a", "Advance Wars", "mGBA", "/roms/Advance Wars (USA).gba");
    let b = rom("b", "Advance Wars", "mGBA", "/roms/Advance Wars (PT-BR patch).gba");
    let gog = copy("gog", "1", "Advance Wars", None, None);
    assert_eq!(details(&[&a, &b, &gog]), [Some("Advance Wars (USA).gba".into()), Some("Advance Wars (PT-BR patch).gba".into()), None]);
    // The ROM the user picked wins over the first one.
    let picked = game_with("p", "sideload", |info, _, _, meta, _| {
        info.title = "Advance Wars".into();
        meta.launch_kind = "emulator".into();
        meta.roms = Some(vec![crate::models::LocalGameRom { name: None, path: "/roms/disc1.cue".into(), extra: Default::default() }]);
        meta.selected_rom_path = Some("/roms/disc2.cue".into());
    });
    assert_eq!(details(&[&a, &picked])[1].as_deref(), Some("disc2.cue"));
}

#[test]
fn a_link_to_a_steam_page_does_not_make_copies_but_an_id_a_source_reports_does() {
    // Copies need the same title and a shared id: the id of a link (by hand, or found by name)
    // does not count, one a source reported does.
    let linked = |runner: &str, link: Option<serde_json::Value>| {
        game_with("1", runner, |info, _, md, meta, _| {
            info.title = "Skyrim".into();
            md.steam_app_id = Some("72850".into());
            if let Some(entry) = link {
                md.extra.insert("sourceMatches".into(), serde_json::json!({ "steam": entry }));
            }
            meta.source = String::new();
            meta.launch_kind = String::new();
        })
    };
    let pinned = linked("gog", Some(serde_json::json!({"id": "72850", "pinned": true})));
    let by_name = linked("nile", Some(serde_json::json!({"id": "72850", "matchedBy": "exactTitle"})));
    let a = linked("legendary", None);
    let b = linked("sideload", None);
    let g = groups(&[pinned, by_name, a, b], &CopiesFile::default());
    assert_eq!(keys(&g), [["legendary_1", "sideload_1"]]);
}
