use super::*;
use serde_json::json;

#[test]
fn default_is_classic_and_round_trips() {
    let t = ConsoleTheme::default();
    assert_eq!(t.id, "classic");
    assert_eq!(t.tabs.len(), 2);
    let back = ConsoleTheme::from_json(&t.to_json().unwrap()).unwrap();
    assert_eq!(back, t.clone().normalized());
}

#[test]
fn unknown_keys_and_values_are_kept_or_defaulted() {
    let text = json!({
        "format": 1,
        "name": "Mine",
        "futureThing": {"a": 1},
        "tiles": {"shape": "hexagon", "size": "l"},
        "tabs": [{"name": "A", "layout": "holodeck"}],
    })
    .to_string();
    let t = ConsoleTheme::from_json(&text).unwrap();
    assert_eq!(t.tiles.shape, TileShape::Portrait);
    assert_eq!(t.tiles.size, TileSize::Large);
    assert_eq!(t.tabs[0].layout, Layout::Shelves);
    assert_eq!(t.extra["futureThing"]["a"], 1);
    let again: Value = serde_json::from_str(&t.to_json().unwrap()).unwrap();
    assert_eq!(again["futureThing"]["a"], 1);
}

#[test]
fn newer_format_is_refused() {
    assert!(ConsoleTheme::from_json(&json!({"format": FORMAT + 1}).to_string()).is_err());
    assert!(ConsoleTheme::from_json("[1, 2]").is_err());
}

#[test]
fn normalized_clamps_and_repairs() {
    let mut t = ConsoleTheme { tabs: (0..9).map(|_| TabSpec::default()).collect(), ..ConsoleTheme::default() };
    t.interface.scale = 400;
    t.background.dim = 255;
    t.menu.items.retain(|m| m.item != "settings");
    t.menu.items.iter_mut().for_each(|m| m.on = false);
    let t = t.normalized();
    assert_eq!(t.tabs.len(), MAX_TABS);
    assert!(t.tabs.iter().all(|t| !t.name.is_empty()));
    assert_eq!(t.interface.scale, SCALE_RANGE.1);
    assert_eq!(t.background.dim, DIM_MAX);
    for locked in MENU_LOCKED {
        assert!(t.menu.items.iter().any(|m| m.item == locked && m.on));
    }
    let empty = ConsoleTheme { tabs: vec![], ..ConsoleTheme::default() }.normalized();
    assert_eq!(empty.tabs.len(), 2);
}

#[test]
fn presets_are_valid_and_unique() {
    let all = presets::all();
    let mut ids: Vec<_> = all.iter().map(|t| t.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), all.len());
    for t in all {
        assert_eq!(t.clone().normalized().tabs, t.tabs, "{}", t.id);
    }
}

#[test]
fn prefs_and_store_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    assert_eq!(ConsolePrefs::load(&paths), ConsolePrefs::default());
    let p = ConsolePrefs { theme: presets::cinema(), based_on: "cinema".into(), ..ConsolePrefs::default() };
    p.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.tabs[0].layout, Layout::Cinema);

    let mut mine = presets::pulse();
    mine.name = "Meu Tema Ção".into();
    let saved = store::save(&paths, &mine).unwrap();
    assert_eq!(saved.id, "meu-tema-cao");
    let again = store::save(&paths, &ConsoleTheme { id: String::new(), ..mine.clone() }).unwrap();
    assert_eq!(again.id, "meu-tema-cao-2");
    assert_eq!(store::list(&paths).len(), 2);
    let file = dir.path().join(format!("x.{}", store::EXTENSION));
    store::export(&saved, &file).unwrap();
    let imported = store::import(&paths, &file).unwrap();
    assert_eq!(imported.name, "Meu Tema Ção");
    store::delete(&paths, &saved.id).unwrap();
    assert!(store::find(&paths, "meu-tema-cao").is_none());
    assert!(store::find(&paths, "pulse").is_some());
}

#[test]
fn hex_and_slug() {
    assert_eq!(parse_hex("#ff8000"), Some((255, 128, 0)));
    assert_eq!(parse_hex("nope"), None);
    assert_eq!(slug("  !!"), "theme");
}

#[test]
fn a_light_background_makes_the_theme_light() {
    let with = |bg: &str| ColourOptions { background: bg.into(), ..ColourOptions::default() };
    assert!(with("#eef1f5").is_light());
    assert!(!with("#0b0e13").is_light());
    // Mid greys stay dark: the threshold is relative luminance 0.5.
    assert!(!with("#808080").is_light());
    assert!(!ColourOptions::default().is_light());
}

#[test]
fn an_export_carries_its_picture_to_another_machine() {
    let here = tempfile::tempdir().unwrap();
    let there = tempfile::tempdir().unwrap();
    let paths = |d: &tempfile::TempDir| AppPaths { config: d.path().join("config"), cache: d.path().join("cache") };
    let picture = here.path().join("wall.png");
    std::fs::write(&picture, b"\x89PNG not really").unwrap();
    let mut t = presets::classic();
    t.name = "Wall".into();
    t.background.source = BackgroundSource::Image;
    t.background.image = picture.display().to_string();

    // Saving copies the picture into the theme folder: it survives the original moving.
    let saved = store::save(&paths(&here), &t).unwrap();
    let own = std::path::PathBuf::from(&saved.background.image);
    assert!(own.starts_with(paths(&here).console_themes_dir()));
    std::fs::remove_file(&picture).unwrap();
    assert!(own.exists());

    let file = here.path().join(format!("wall.{}", store::EXTENSION));
    store::export(&saved, &file).unwrap();
    let shared = store::read(&file).unwrap();
    assert!(shared.background.image.starts_with("data:image/png;base64,"));

    let imported = store::import(&paths(&there), &file).unwrap();
    let back = std::path::PathBuf::from(&imported.background.image);
    assert!(back.starts_with(paths(&there).console_themes_dir()));
    assert_eq!(std::fs::read(&back).unwrap(), b"\x89PNG not really");

    // Deleting the theme takes its picture with it.
    store::delete(&paths(&there), &imported.id).unwrap();
    assert!(!back.exists());
}

#[test]
fn an_unused_local_picture_stays_out_of_an_export() {
    let mut t = presets::classic();
    t.background.image = "/home/someone/wall.png".into();
    assert_eq!(store::portable(&t).background.image, "");
    t.background.image = "https://example.com/wall.png".into();
    assert_eq!(store::portable(&t).background.image, "https://example.com/wall.png");
}

#[test]
fn an_import_is_compared_with_the_saved_themes() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    let mut mine = presets::pulse();
    mine.name = "Mine".into();
    let saved = store::save(&paths, &mine).unwrap();
    let list = store::list(&paths);
    assert_eq!(store::compare(&saved, &list), store::Match::Same(saved.id.clone()));
    let mut newer = saved.clone();
    newer.tiles.size = TileSize::Huge;
    assert_eq!(store::compare(&newer, &list), store::Match::SameId(saved.id.clone()));
    assert_eq!(store::compare(&presets::cinema(), &list), store::Match::New);

    // Kept apart unless asked to replace.
    let both = store::add(&paths, &newer, false).unwrap();
    assert_ne!(both.id, saved.id);
    let replaced = store::add(&paths, &newer, true).unwrap();
    assert_eq!(replaced.id, saved.id);
    assert_eq!(store::list(&paths).len(), 2);
}

#[test]
fn a_wrong_value_loses_only_itself() {
    let text = json!({
        "name": "Odd",
        "shelfLimit": "lots",
        "tiles": {"size": "xl", "greyUninstalled": "yes"},
        "background": {"dim": 400, "blur": 8},
        "interface": {"accent": "not a colour"},
    })
    .to_string();
    let t = ConsoleTheme::from_json(&text).unwrap();
    assert_eq!(t.name, "Odd");
    assert_eq!(t.shelf_limit, presets::classic().shelf_limit);
    assert_eq!(t.tiles.size, TileSize::Huge);
    assert!(!t.tiles.grey_uninstalled);
    assert_eq!(t.background.blur, 8);
    assert_eq!(t.interface.accent, "white");
}

#[test]
fn an_id_cannot_leave_the_themes_folder() {
    let t = ConsoleTheme { id: "../../evil".into(), ..ConsoleTheme::default() }.normalized();
    assert_eq!(t.id, "evil");
    let dup = ConsoleTheme { shelves: vec![ShelfSpec::new("recent", true), ShelfSpec::new("recent", false)], ..ConsoleTheme::default() };
    assert_eq!(dup.normalized().shelves.len(), 1);
}

#[test]
fn deleting_the_theme_in_use_keeps_the_picture_on_screen() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    let picture = dir.path().join("wall.png");
    std::fs::write(&picture, b"png").unwrap();
    let mut t = presets::classic();
    t.name = "Wall".into();
    t.background.source = BackgroundSource::Image;
    t.background.image = picture.display().to_string();
    let saved = store::save(&paths, &t).unwrap();
    ConsolePrefs { theme: saved.clone(), based_on: saved.id.clone(), ..ConsolePrefs::default() }.save(&paths).unwrap();
    store::delete(&paths, &saved.id).unwrap();
    assert!(std::path::Path::new(&saved.background.image).exists());
}
