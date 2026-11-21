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
fn classic_is_the_unlisted_base_and_cinema_is_listed() {
    let ids: Vec<_> = presets::all().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, ["minimal", "pulse", "cinema", "arcade", "hub", "living-room"]);
    assert!(presets::by_id("classic").is_none());
    let cinema = presets::cinema();
    assert_eq!(cinema.tabs.iter().map(|t| t.source.as_str()).collect::<Vec<_>>(), ["installed", "all", "recent", "favourites", "all"]);
    assert!(cinema.tabs[..4].iter().all(|t| t.layout == Layout::Cinema));
    assert_eq!(cinema.tabs[4].layout, Layout::Grid);
    assert_eq!(cinema.nav.a_button, AButton::default(), "A and X are the user's choice");
    assert!(!cinema.layouts.showcase_glass && presets::pulse().layouts.showcase_glass);
}

#[test]
fn the_built_in_classic_and_first_cinema_become_cinema_unless_changed() {
    for old in [presets::classic(), presets::first_cinema(), presets::second_cinema(), presets::five_tab_cinema()] {
        // Saved and read back, as a console.json is.
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        ConsolePrefs { theme: old.clone(), based_on: old.id.clone(), ..ConsolePrefs::default() }.save(&paths).unwrap();
        let p = ConsolePrefs::load(&paths);
        assert_eq!((p.theme, p.based_on.as_str()), (presets::cinema(), "cinema"), "{}", old.id);
        // Changed by the user: its values stay.
        let changed = ConsoleTheme { shelf_limit: 7, ..old.clone() };
        let mut p = ConsolePrefs { theme: changed.clone(), based_on: old.id.clone(), ..ConsolePrefs::default() };
        upgrade_cinema(&mut p);
        assert_eq!(p.theme, changed);
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
    let kept = KeptFilters { show: 1, sort: 2, favourites: true, carousel: 1 };
    ConsolePrefs { kept_filters: kept, ..p.clone() }.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).kept_filters, kept);

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

#[test]
fn kept_filters_read_leniently() {
    // A file from before them, and one with a broken value: the rest stays.
    let old: ConsolePrefs = lenient(json!({ "lastTab": 2 })).unwrap();
    assert_eq!((old.last_tab, old.kept_filters), (2, KeptFilters::default()));
    let odd: ConsolePrefs = lenient(json!({ "lastTab": 1, "keptFilters": { "show": "x", "sort": 1, "carousel": 1 } })).unwrap();
    assert_eq!((odd.last_tab, odd.kept_filters), (1, KeptFilters { sort: 1, carousel: 1, ..KeptFilters::default() }));
}

#[test]
fn the_carousel_starts_on_installed_and_a_saved_pill_keeps_its_games() {
    // New: Installed. A pill saved before Installed came first keeps what it
    // showed (the number names the filter, not its place in the row).
    assert_eq!(KeptFilters::default().carousel, CAROUSEL_INSTALLED);
    let fresh: ConsolePrefs = lenient(json!({ "keptFilters": { "sort": 0 } })).unwrap();
    assert_eq!(fresh.kept_filters.carousel, CAROUSEL_INSTALLED);
    for saved in 0..4 {
        let p: ConsolePrefs = lenient(json!({ "keptFilters": { "carousel": saved } })).unwrap();
        assert_eq!(p.kept_filters.carousel, saved);
    }
}

#[test]
fn the_controller_options_leave_the_theme_and_keep_what_was_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    let write = |v: Value| {
        std::fs::create_dir_all(paths.console_prefs().parent().unwrap()).unwrap();
        std::fs::write(paths.console_prefs(), v.to_string()).unwrap();
    };
    // New: automatic both.
    assert_eq!(ConsolePrefs::default().controller, ControllerPrefs { glyphs: Glyphs::Auto, confirm: ConfirmButton::Auto });
    // A file from before: the east button stays the confirm button; a
    // theme's Xbox or PlayStation drawing becomes automatic, the keyboard's stays.
    write(json!({ "theme": { "controls": { "glyphs": "playstation", "swapAb": true, "hints": "off" } } }));
    let p = ConsolePrefs::load(&paths);
    assert_eq!(p.controller, ControllerPrefs { glyphs: Glyphs::Auto, confirm: ConfirmButton::East });
    assert_eq!(p.theme.controls.hints, Hints::Off);
    write(json!({ "theme": { "controls": { "glyphs": "keyboard" } } }));
    assert_eq!(ConsolePrefs::load(&paths).controller, ControllerPrefs { glyphs: Glyphs::Keyboard, confirm: ConfirmButton::Auto });
    // Once saved, its own key wins over what the theme had.
    write(json!({ "controller": { "glyphs": "nintendo", "confirm": "south" }, "theme": { "controls": { "swapAb": true } } }));
    assert_eq!(ConsolePrefs::load(&paths).controller, ControllerPrefs { glyphs: Glyphs::Nintendo, confirm: ConfirmButton::South });
    // A theme no longer carries them.
    let text = presets::pulse().to_json().unwrap();
    assert!(!text.contains("glyphs") && !text.contains("swapAb"), "{text}");
}

#[test]
fn a_theme_from_before_the_sound_options_gets_the_soft_sound_at_full_volume() {
    let text = json!({ "format": 1, "controls": { "hints": "off", "sounds": true } }).to_string();
    let c = ConsoleTheme::from_json(&text).unwrap().controls;
    assert!(c.sounds);
    assert_eq!((c.sound_style, c.sound_volume), (SoundStyle::Soft, 100));
    let text = json!({ "format": 1, "controls": { "soundStyle": "classic", "soundVolume": 40 } }).to_string();
    let c = ConsoleTheme::from_json(&text).unwrap().controls;
    assert_eq!((c.sound_style, c.sound_volume), (SoundStyle::Classic, 40));
}

#[test]
fn the_sound_style_and_volume_are_repaired() {
    let text = json!({ "format": 1, "controls": { "soundStyle": "loud", "soundVolume": 0 } }).to_string();
    let c = ConsoleTheme::from_json(&text).unwrap().normalized().controls;
    assert_eq!((c.sound_style, c.sound_volume), (SoundStyle::Soft, SOUND_VOLUME.0));
    let text = json!({ "format": 1, "controls": { "soundVolume": 200 } }).to_string();
    assert_eq!(ConsoleTheme::from_json(&text).unwrap().normalized().controls.sound_volume, SOUND_VOLUME.1);
}

#[test]
fn pulse_with_its_first_tabs_takes_its_new_home() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    let mut old = presets::pulse();
    old.tabs = vec![TabSpec::new("Games", Layout::Spotlight, "all"), TabSpec::new("Library", Layout::Grid, "all")];
    old.interface.tab_bar = TabBar::Names;
    old.interface.accent = "pink".into();
    ConsolePrefs { theme: old, ..ConsolePrefs::default() }.save(&paths).unwrap();
    let p = ConsolePrefs::load(&paths);
    assert_eq!(p.theme.tabs, presets::pulse().tabs);
    assert_eq!(p.theme.interface.tab_bar, TabBar::Top);
    assert_eq!(p.theme.interface.accent, "pink", "the user's own changes stay");

    assert_eq!(p.theme.tabs[0].source, "installed", "Installed comes first");
    assert_eq!(p.kept_filters.sort, SORT_LAST_PLAYED, "played last first");

    // The four filters it had for a while take the same home.
    let mut filters = presets::pulse();
    filters.tabs = vec![
        TabSpec::new("All games", Layout::Spotlight, "all"),
        TabSpec::new("Installed", Layout::Spotlight, "installed"),
        TabSpec::new("Favourites", Layout::Spotlight, "favourites"),
        TabSpec::new("Not played yet", Layout::Spotlight, "status:not-played"),
    ];
    ConsolePrefs { theme: filters, ..ConsolePrefs::default() }.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.tabs, presets::pulse().tabs);

    // Tabs the user changed are theirs.
    let mut mine = presets::pulse();
    mine.tabs = vec![TabSpec::new("Games", Layout::Spotlight, "all")];
    ConsolePrefs { theme: mine.clone(), ..ConsolePrefs::default() }.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.tabs, mine.tabs);
}

#[test]
fn a_saved_minimal_opens_the_showcase_unless_its_game_page_was_changed() {
    let as_shipped = |page: GamePage| {
        let mut theme = presets::minimal();
        theme.game_page = page;
        let mut p = ConsolePrefs { theme, ..ConsolePrefs::default() };
        upgrade_minimal(&mut p);
        p.theme.game_page
    };
    assert_eq!(as_shipped(GamePage::Sheet), GamePage::Showcase);
    assert_eq!(as_shipped(GamePage::Hero), GamePage::Hero);
    assert_eq!(presets::minimal().game_page, GamePage::Showcase);
}

#[test]
fn a_saved_five_tab_cinema_that_set_a_to_play_moves_on_with_the_default_buttons() {
    // As a console.json read: id "cinema", the five tabs with "Not played
    // yet", `nav.aButton: play` written by an earlier copy of the preset.
    let mut old = presets::five_tab_cinema();
    old.nav.a_button = AButton::Play;
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    ConsolePrefs { theme: old, based_on: "cinema".into(), ..ConsolePrefs::default() }.save(&paths).unwrap();
    let text = std::fs::read_to_string(paths.console_prefs()).unwrap();
    assert!(text.contains("\"aButton\": \"play\"") && text.contains("status:not-played"));
    let p = ConsolePrefs::load(&paths);
    assert_eq!(p.theme, presets::cinema());
    assert_eq!(p.theme.nav.a_button, AButton::Details);
    assert!(!p.theme.tabs.iter().any(|t| t.source == "status:not-played"));
}

#[test]
fn arcade_shows_covers_beside_the_names_instead_of_logos() {
    assert_eq!(presets::arcade().layouts.wheel_items, WheelItems::Name);
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    let mut old = presets::arcade();
    old.layouts.wheel_items = WheelItems::Logo;
    ConsolePrefs { theme: old, ..ConsolePrefs::default() }.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.layouts.wheel_items, WheelItems::Name);
}

#[test]
fn living_room_drops_the_new_badges() {
    assert_eq!(presets::living_room().tiles.badges, Badges::Running);
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    let mut old = presets::living_room();
    old.tiles.badges = Badges::All;
    ConsolePrefs { theme: old, ..ConsolePrefs::default() }.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.tiles.badges, Badges::Running);
}

#[test]
fn a_migration_runs_once_so_the_user_can_pick_the_old_value_again() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
    let mut old = presets::living_room();
    old.tiles.badges = Badges::All;
    ConsolePrefs { theme: old, ..ConsolePrefs::default() }.save(&paths).unwrap();
    // The first load moves the built-in theme on.
    let mut p = ConsolePrefs::load(&paths);
    assert_eq!(p.theme.tiles.badges, Badges::Running);
    // The user turns every badge back on: it stays through the next start.
    p.theme.tiles.badges = Badges::All;
    p.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.tiles.badges, Badges::All);
    // The same for Minimal's page and Arcade's logos.
    let mut p = ConsolePrefs::load(&paths);
    p.theme = presets::minimal();
    p.theme.game_page = GamePage::Sheet;
    p.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.game_page, GamePage::Sheet);
    let mut p = ConsolePrefs::load(&paths);
    p.theme = presets::arcade();
    p.theme.layouts.wheel_items = WheelItems::Logo;
    p.save(&paths).unwrap();
    assert_eq!(ConsolePrefs::load(&paths).theme.layouts.wheel_items, WheelItems::Logo);
}
