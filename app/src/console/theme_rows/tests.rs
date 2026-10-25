use heronite_data::console_theme::{presets, ConsolePrefs, Layout, MAX_TABS};

use super::*;
use crate::console::Pad;

fn data() -> Data {
    Data {
        facts: Facts {
            statuses: vec![("playing".into(), "Playing".into())],
            categories: vec!["Co-op".into()],
            platforms: vec!["PC".into(), "PlayStation 2".into()],
            stores: vec!["Steam".into()],
            genres: vec!["Adventure".into()],
        },
        saved: Vec::new(),
    }
}

fn change(prefs: &mut ConsolePrefs, id: &str, value: Value) -> bool {
    run(prefs, &Command::changed(id, value), &data())
}

fn choice(index: usize) -> Value {
    Value::Choice { options: Vec::new(), index }
}

/// The prefs of a machine on Classic (two tabs, shelves and a grid).
fn classic() -> ConsolePrefs {
    ConsolePrefs { theme: presets::classic(), based_on: "classic".into(), ..ConsolePrefs::default() }
}

fn rows_of(prefs: &ConsolePrefs, card: Card) -> Vec<Row> {
    let d = data();
    rows(card, &Ctx { prefs, data: &d })
}

#[test]
fn a_new_machine_starts_on_minimal_listed_first() {
    let p = ConsolePrefs::default();
    assert_eq!((p.theme.id.as_str(), p.based_on.as_str()), ("minimal", "minimal"));
    assert_eq!(theme_list(&[])[0].0, "minimal");
    assert!(!is_edited(&p, &[]));
}

#[test]
fn toggles_choices_and_numbers_write_the_theme() {
    let mut p = ConsolePrefs::default();
    assert!(change(&mut p, "nav.wrap", Value::Toggle(true)));
    assert!(p.theme.nav.wrap);
    assert!(change(&mut p, "interface.clock", choice(2)));
    assert_eq!(p.theme.interface.clock, heronite_data::console_theme::Clock::Off);
    let n = Value::Number { value: 200, min: 80, max: 140, step: 5, suffix: "%" };
    assert!(change(&mut p, "interface.scale", n));
    assert_eq!(p.theme.interface.scale, 140, "clamped");
    assert!(!change(&mut p, "interface.clock", choice(2)), "same value changes nothing");
    assert!(!change(&mut p, "interface.clock", choice(99)), "out of range is ignored");
}

#[test]
fn startup_options_live_in_the_prefs_not_the_theme() {
    let mut p = ConsolePrefs::default();
    assert!(change(&mut p, "startup.open_in", choice(1)));
    assert_eq!(p.open_in, heronite_data::console_theme::OpenIn::Console);
    assert!(change(&mut p, "startup.idle", choice(2)));
    assert_eq!(p.idle, heronite_data::console_theme::Idle::Dim);
}

#[test]
fn tabs_stay_between_one_and_four() {
    let mut p = ConsolePrefs::default();
    let add = Command::new("tab.add", Arg::Press);
    for _ in 0..10 {
        run(&mut p, &add, &data());
    }
    assert_eq!(p.theme.tabs.len(), MAX_TABS);
    let layouts: Vec<Layout> = p.theme.tabs.iter().map(|t| t.layout).collect();
    let mut unique = layouts.clone();
    unique.dedup();
    assert_eq!(unique.len(), layouts.len(), "new tabs take layouts not used yet");
    for _ in 0..10 {
        run(&mut p, &Command::new("tab.0.remove", Arg::Press), &data());
    }
    assert_eq!(p.theme.tabs.len(), 1);
    assert!(rows_of(&p, Card::Tabs).iter().all(|r| !r.id.ends_with(".remove")), "the last tab cannot be removed");
}

#[test]
fn a_tab_follows_its_layout_name_until_renamed() {
    let mut p = classic();
    run(&mut p, &Command::new("tab.1.name", Arg::Text("Games".into())), &data());
    change(&mut p, "tab.1.layout", choice(5));
    assert_eq!(p.theme.tabs[1].layout, Layout::Wheel);
    assert_eq!(p.theme.tabs[1].name, "Games");
    run(&mut p, &Command::new("tab.add", Arg::Press), &data());
    let added = p.theme.tabs.len() - 1;
    let name = p.theme.tabs[added].name.clone();
    assert_eq!(name, tr(layout_name(p.theme.tabs[added].layout)));
    change(&mut p, &format!("tab.{added}.layout"), choice(2));
    assert_eq!(p.theme.tabs[added].name, "Cinema", "a tab named after its layout follows it");
}

#[test]
fn a_tab_source_comes_from_the_library() {
    let mut p = ConsolePrefs::default();
    let options = source_options(&data().facts, "all");
    let at = options.iter().position(|(v, _)| v == "status:playing").unwrap();
    change(&mut p, "tab.0.source", choice(at));
    assert_eq!(p.theme.tabs[0].source, "status:playing");
    assert!(options.iter().any(|(v, _)| v == "category:Co-op" && v != "all"));
    let odd = source_options(&data().facts, "store:Itch");
    assert_eq!(odd.last().unwrap().0, "store:Itch", "an unknown source is kept");
}

#[test]
fn shelves_toggle_move_remove_and_add() {
    let mut p = ConsolePrefs::default();
    let first = p.theme.shelves[0].kind.clone();
    let second = p.theme.shelves[1].kind.clone();
    run(&mut p, &Command::new("shelf.0", Arg::Move(1)), &data());
    assert_eq!((p.theme.shelves[0].kind.as_str(), p.theme.shelves[1].kind.as_str()), (second.as_str(), first.as_str()));
    assert!(!run(&mut p, &Command::new("shelf.0", Arg::Move(-1)), &data()), "already first");
    change(&mut p, "shelf.0", Value::Toggle(false));
    assert!(!p.theme.shelves[0].on);
    let n = p.theme.shelves.len();
    run(&mut p, &Command::new("shelf.0", Arg::Remove), &data());
    assert_eq!(p.theme.shelves.len(), n - 1);
    run(&mut p, &Command::new("shelf.add", Arg::Pick("genre:Adventure".into())), &data());
    run(&mut p, &Command::new("shelf.add", Arg::Pick("genre:Adventure".into())), &data());
    assert_eq!(p.theme.shelves.iter().filter(|s| s.kind == "genre:Adventure").count(), 1);
    run(&mut p, &Command::new("shelf.0", Arg::Text("  Mine  ".into())), &data());
    assert_eq!(p.theme.shelves[0].title, "Mine");
}

#[test]
fn only_missing_shelf_kinds_are_offered() {
    let p = ConsolePrefs::default();
    let d = data();
    let kinds: Vec<String> = addable_kinds(&Ctx { prefs: &p, data: &d }).into_iter().map(|(_, k, _)| k).collect();
    assert!(!kinds.contains(&"recent".to_string()), "already in the theme");
    assert!(!kinds.contains(&"random".to_string()), "switched off is still in the theme");
    let mut fewer = ConsolePrefs::default();
    fewer.theme.shelves.retain(|s| s.kind != "random");
    let again: Vec<String> = addable_kinds(&Ctx { prefs: &fewer, data: &d }).into_iter().map(|(_, k, _)| k).collect();
    assert!(again.contains(&"random".to_string()));
    assert!(kinds.contains(&"platform:PlayStation 2".to_string()));
    assert!(kinds.contains(&"genre:Adventure".to_string()));
    assert!(!kinds.contains(&"statuses".to_string()), "the default shelves have it");
    assert!(kinds.contains(&"status:playing".to_string()), "a single status can still be added on its own");
}

#[test]
fn locked_menu_items_cannot_be_hidden() {
    let mut p = ConsolePrefs::default();
    let at = |p: &ConsolePrefs, item: &str| p.theme.menu.items.iter().position(|m| m.item == item).unwrap();
    let settings = at(&p, "settings");
    change(&mut p, &format!("menu.{settings}"), Value::Toggle(false));
    assert!(p.theme.menu.items[settings].on);
    let stats = at(&p, "statistics");
    change(&mut p, &format!("menu.{stats}"), Value::Toggle(false));
    assert!(!p.theme.menu.items[stats].on);
    let row = rows_of(&p, Card::Menu).into_iter().find(|r| r.id.as_ref() == format!("menu.{settings}")).unwrap();
    assert!(!row.enabled);
}

#[test]
fn the_accent_is_named_or_custom() {
    let mut p = ConsolePrefs::default();
    change(&mut p, "interface.accent", choice(3));
    assert_eq!(p.theme.interface.accent, "green");
    assert!(rows_of(&p, Card::Interface).iter().all(|r| r.id.as_ref() != "interface.accent_hex"));
    change(&mut p, "interface.accent", choice(7));
    assert_eq!(p.theme.interface.accent, "#a5dceb");
    assert!(rows_of(&p, Card::Interface).iter().any(|r| r.id.as_ref() == "interface.accent_hex"));
    run(&mut p, &Command::new("interface.accent_hex", Arg::Text("FF8800".into())), &data());
    assert_eq!(p.theme.interface.accent, "#ff8800");
    run(&mut p, &Command::new("interface.accent_hex", Arg::Text("nope".into())), &data());
    assert_eq!(p.theme.interface.accent, "#ff8800");
}

#[test]
fn colours_take_hex_or_empty() {
    assert_eq!(normalize_colour(" #ABCDEF "), Some("#abcdef".into()));
    assert_eq!(normalize_colour(""), Some(String::new()));
    assert_eq!(normalize_colour("12345"), None);
    let mut p = ConsolePrefs::default();
    run(&mut p, &Command::new("colors.panel", Arg::Text("101820".into())), &data());
    assert_eq!(p.theme.colors.panel, "#101820");
    run(&mut p, &Command::new("colors.panel", Arg::Text("blue".into())), &data());
    assert_eq!(p.theme.colors.panel, "#101820", "not a colour: kept");
    run(&mut p, &Command::new("colors.panel", Arg::Text("".into())), &data());
    assert_eq!(p.theme.colors.panel, "");
}

#[test]
fn layout_options_wait_for_a_tab_that_uses_them() {
    let mut p = classic();
    let row = |p: &ConsolePrefs, id: &str| rows_of(p, Card::Layouts).into_iter().find(|r| r.id.as_ref() == id).unwrap();
    assert!(row(&p, "layouts.jump_letters").enabled, "Classic has a grid");
    assert!(!row(&p, "layouts.dashboard.week").enabled);
    change(&mut p, "tab.0.layout", choice(3));
    assert!(row(&p, "layouts.dashboard.week").enabled);
    assert!(applies("layouts.dashboard.week", &p.theme));
    assert!(!applies("layouts.carousel_reflect", &p.theme));
    assert!(applies("nav.wrap", &p.theme));
}

#[test]
fn a_theme_is_edited_when_it_differs_from_its_base() {
    let mut p = ConsolePrefs::default();
    assert!(!is_edited(&p, &[]));
    change(&mut p, "nav.wrap", Value::Toggle(true));
    assert!(is_edited(&p, &[]));
    let row = rows_of(&p, Card::Theme).into_iter().find(|r| r.id.as_ref() == "theme.use").unwrap();
    match row.value {
        Value::Choice { options, index } => assert_eq!(index, options.len() - 1, "shows Customised"),
        v => panic!("{v:?}"),
    }
}

#[test]
fn picking_a_theme_replaces_the_theme_and_its_base() {
    let mut p = ConsolePrefs::default();
    let list = theme_list(&[]);
    let at = list.iter().position(|(id, _)| id == "cinema").unwrap();
    assert!(change(&mut p, "theme.use", choice(at)));
    assert_eq!(p.based_on, "cinema");
    assert_eq!(p.theme, presets::cinema());
    assert!(!is_edited(&p, &[]));
    assert!(run(&mut p, &Command::new("theme.reset", Arg::Press), &data()));
    assert_eq!(p.theme, presets::minimal());
    assert_eq!(p.based_on, "minimal");
}

#[test]
fn saved_themes_get_save_rename_and_delete_rows() {
    let mut saved = presets::pulse();
    saved.id = "mine".into();
    saved.name = "Mine".into();
    let d = Data { saved: vec![saved.clone()], ..Default::default() };
    let mut p = ConsolePrefs { theme: saved.clone(), based_on: "mine".into(), ..Default::default() };
    let ids = |p: &ConsolePrefs| rows(Card::Theme, &Ctx { prefs: p, data: &d }).into_iter().map(|r| r.id.to_string()).collect::<Vec<_>>();
    let has = |ids: &[String], id: &str| ids.iter().any(|i| i == id);
    let now = ids(&p);
    assert!(has(&now, "theme.rename") && has(&now, "theme.delete"));
    assert!(!has(&now, "theme.save"), "nothing to save yet");
    p.theme.nav.wrap = true;
    assert!(has(&ids(&p), "theme.save"));
    let plain = ids(&ConsolePrefs::default());
    assert!(!has(&plain, "theme.rename") && !has(&plain, "theme.delete"));
}

#[test]
fn activating_rows_asks_for_the_right_thing() {
    let p = classic();
    let d = data();
    let cx = Ctx { prefs: &p, data: &d };
    assert!(matches!(activate("tab.0.name", &cx), Step::Text { initial, .. } if initial == "Home"));
    assert!(matches!(activate("colors.text", &cx), Step::Text { .. }));
    assert!(matches!(activate("background.image", &cx), Step::File { .. }));
    assert!(matches!(activate("shelf.add", &cx), Step::Pick { .. }));
    assert!(matches!(activate("tab.add", &cx), Step::Run(_)));
    assert!(matches!(activate("theme.delete", &cx), Step::Confirm { .. }));
    assert!(matches!(activate("theme.export", &cx), Step::Store(StoreOp::Export)));
    assert!(matches!(activate("nav.wrap", &cx), Step::Nothing));
}

#[test]
fn rows_have_unique_ids() {
    let p = ConsolePrefs::default();
    let d = data();
    let ids: Vec<String> = all_rows(&Ctx { prefs: &p, data: &d }).into_iter().map(|r| r.id.to_string()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len());
    for g in 0..PANEL.len() {
        assert!(!group_rows(g, &Ctx { prefs: &p, data: &d }).is_empty());
    }
}

#[test]
fn shifting_stops_at_the_ends() {
    let mut v = vec![1, 2, 3];
    assert_eq!(shift(&mut v, 0, -1), None);
    assert_eq!(shift(&mut v, 2, 1), None);
    assert_eq!(shift(&mut v, 0, 1), Some(1));
    assert_eq!(v, [2, 1, 3]);
}

#[test]
fn extra_pads_move_and_remove() {
    let mut st = ThemeState { prefs: ConsolePrefs::default(), data: data(), notice: None, moving: false };
    assert!(matches!(extra_pad(&mut st, Pad::X, Some("nav.wrap")), Extra::Pass));
    assert!(matches!(extra_pad(&mut st, Pad::X, Some("shelf.2")), Extra::Handled));
    assert!(st.moving);
    assert!(matches!(extra_pad(&mut st, Pad::Down, Some("shelf.2")), Extra::Run(c, 1) if c.arg == Arg::Move(1)));
    assert!(matches!(extra_pad(&mut st, Pad::B, Some("shelf.3")), Extra::Handled));
    assert!(!st.moving);
    assert!(matches!(extra_pad(&mut st, Pad::LT, Some("shelf.3")), Extra::Run(c, 0) if c.arg == Arg::Remove));
    assert!(matches!(extra_pad(&mut st, Pad::LT, Some("menu.3")), Extra::Pass));
}

#[test]
fn export_names_follow_the_id() {
    assert_eq!(export_name(&presets::cinema()), "cinema.heronite-theme.json");
    let mut t = presets::cinema();
    t.id.clear();
    t.name = "My Look!".into();
    assert_eq!(export_name(&t), "my-look.heronite-theme.json");
}

#[test]
fn a_theme_is_picked_by_id_and_changes_go_back_to_its_base() {
    let mut p = classic();
    assert!(run(&mut p, &Command::new("theme.use", Arg::Pick("pulse".into())), &data()));
    assert_eq!((p.theme.id.as_str(), p.based_on.as_str()), ("pulse", "pulse"));
    assert!(!run(&mut p, &Command::new("theme.use", Arg::Pick("nope".into())), &data()), "an unknown id changes nothing");
    assert!(change(&mut p, "nav.wrap", Value::Toggle(true)));
    assert!(is_edited(&p, &[]));
    assert!(run(&mut p, &Command::new("theme.revert", Arg::Press), &data()));
    assert!(!is_edited(&p, &[]));
    assert_eq!(p.based_on, "pulse");
    p.based_on.clear();
    assert!(!run(&mut p, &Command::new("theme.revert", Arg::Press), &data()), "nothing to go back to");
}
