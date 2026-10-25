//! The console theme's options as console rows, shared by Settings → Console
//! mode and the ☰ → Customize panel: [`rows`] reads the prefs, [`activate`]
//! says what a press on an action / text row asks for, and [`run`] applies a
//! [`Command`] (a changed value, a typed text, a move...) to the prefs. All of
//! it is pure; [`host`] runs it against a view (keyboard, pickers, the theme
//! store, `themes::update`).
//!
//! Row ids name what they change: `nav.wrap`, `tab.1.layout`, `shelf.3`,
//! `menu.5`, `theme.save`... The table [`OPTS`] holds the plain options (a
//! switch, a choice, a number, a text); tabs, shelves, the menu, the accent
//! and the theme card are written out.

mod commands;
mod host;
mod labels;
mod options;

pub use commands::*;
pub use host::*;
use options::*;

use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui_kit::SharedString;
use heronite_data::categories::{Categories, UNCATEGORIZED};
use heronite_data::console_theme::{
    parse_hex, presets, slug, AButton, AutoConsole, BackgroundMotion, BackgroundSource, Badges, Clock, ConsoleFont, ConsolePrefs,
    ConsoleTheme, Corners, FactStyle, FocusStyle, GamePage, Glyphs, GridPanel, Headings, HeroAlign, HeroTitle, Hints, Idle, Layout,
    MenuStyle, Motion, OpenIn, ShelfSpec, Side, Size3, SpotActions, StartOn, TabBar, TabSpec, TileShape, TileSize, TileTitles, WheelItems,
    MAX_TABS, MENU_LOCKED,
};
use heronite_data::{CollectionGame, Library};
use heronite_i18n::{n_, tr, trc};

use super::list::{Row, Value};
use super::look::ACCENTS;
use crate::icons::Icon;
pub use labels::layout_name;
use labels::*;

/// What the library has, for the pickers (a tab's games, a shelf's kind).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// `(key, name)`: the key is what `status:<key>` carries (the slug of the
    /// default statuses, the id of a custom one).
    pub statuses: Vec<(String, String)>,
    pub categories: Vec<String>,
    pub platforms: Vec<String>,
    pub stores: Vec<String>,
    pub genres: Vec<String>,
}

impl Facts {
    /// `platform` gives the platform a game is shown under.
    pub fn build(lib: &Library, categories: &Categories, platform: impl Fn(&CollectionGame) -> String) -> Self {
        let (mut stores, mut platforms, mut genres) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for g in &lib.games {
            stores.insert(g.source_label.to_string());
            platforms.insert(platform(g));
            if let Some(m) = &g.metadata {
                genres.extend(m.genres.iter().filter(|x| !x.trim().is_empty()).cloned());
            }
        }
        let statuses = lib
            .statuses
            .iter()
            .map(|s| {
                let key = if s.slug.is_empty() || s.slug == "custom" { s.id.clone() } else { s.slug.clone() };
                (key, heronite_data::status::status_label(&s.name).to_string())
            })
            .collect();
        Self {
            statuses,
            categories: categories.names().map(str::to_owned).collect(),
            platforms: platforms.into_iter().collect(),
            stores: stores.into_iter().collect(),
            genres: genres.into_iter().collect(),
        }
    }

    /// The name of the status `key` (the key itself when the library has no such status).
    pub fn status_name(&self, key: &str) -> String {
        self.statuses.iter().find(|(k, _)| k == key).map_or_else(|| key.to_string(), |(_, n)| n.clone())
    }
}

/// What the rows need besides the prefs.
#[derive(Clone, Debug, Default)]
pub struct Data {
    pub facts: Facts,
    /// The user's saved themes.
    pub saved: Vec<ConsoleTheme>,
}

pub struct Ctx<'a> {
    pub prefs: &'a ConsolePrefs,
    pub data: &'a Data,
}

/// A block of options, a card on the desktop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Card {
    Theme,
    Tabs,
    Navigation,
    Shelves,
    Hero,
    Covers,
    Background,
    Interface,
    Colours,
    Controls,
    GamePage,
    Menu,
    Layouts,
    Startup,
}

impl Card {
    pub fn title(self) -> &'static str {
        tr(match self {
            Card::Theme => n_!("Theme"),
            Card::Tabs => n_!("Tabs (LB / RB)"),
            Card::Navigation => n_!("Navigation"),
            Card::Shelves => n_!("Shelves"),
            Card::Hero => n_!("Hero"),
            Card::Covers => n_!("Covers"),
            Card::Background => n_!("Background"),
            Card::Interface => n_!("Interface"),
            Card::Colours => n_!("Colours"),
            Card::Controls => n_!("Controls"),
            Card::GamePage => n_!("Game page and menu"),
            Card::Menu => n_!("Menu items"),
            Card::Layouts => n_!("Layout options"),
            Card::Startup => n_!("Startup"),
        })
    }
}

/// The Customize panel's groups (LB / RB) and the cards each one holds.
pub const PANEL: [(&str, &[Card]); 6] = [
    (n_!("Screens"), &[Card::Theme, Card::Tabs, Card::Navigation, Card::Shelves, Card::Hero]),
    (n_!("Covers"), &[Card::Covers]),
    (n_!("Background"), &[Card::Background]),
    (n_!("Interface"), &[Card::Interface, Card::Colours]),
    (n_!("Controls"), &[Card::Controls]),
    (n_!("More"), &[Card::GamePage, Card::Menu, Card::Layouts, Card::Startup]),
];

/// Every card, in the order of the desktop page.
pub const CARDS: [Card; 14] = [
    Card::Theme,
    Card::Tabs,
    Card::Shelves,
    Card::Navigation,
    Card::Hero,
    Card::Covers,
    Card::Background,
    Card::Interface,
    Card::Colours,
    Card::Controls,
    Card::GamePage,
    Card::Menu,
    Card::Layouts,
    Card::Startup,
];

// ---------------------------------------------------------------------------
// Rows.

/// The rows of one card.
pub fn rows(card: Card, cx: &Ctx) -> Vec<Row> {
    let mut v = match card {
        Card::Theme => theme_rows(cx),
        Card::Tabs => tab_rows(cx),
        Card::Shelves => shelf_rows(cx),
        Card::Interface => accent_rows(cx),
        Card::Menu => menu_rows(cx),
        _ => Vec::new(),
    };
    v.extend(OPTS.iter().filter(|o| o.card == card).map(|o| opt_row(o, cx)));
    v
}

/// The rows of a group of the Customize panel: its cards under their titles
/// (a group of one card has no title).
pub fn group_rows(group: usize, cx: &Ctx) -> Vec<Row> {
    cards_rows(PANEL[group.min(PANEL.len() - 1)].1, cx)
}

/// Every card, for the Settings page in console mode.
pub fn all_rows(cx: &Ctx) -> Vec<Row> {
    cards_rows(&CARDS, cx)
}

fn cards_rows(cards: &[Card], cx: &Ctx) -> Vec<Row> {
    let titled = cards.len() > 1;
    let mut v = Vec::new();
    for card in cards {
        if titled {
            v.push(Row::header(card.title()));
        }
        v.extend(rows(*card, cx));
    }
    v
}

fn opt_row(o: &Opt, cx: &Ctx) -> Row {
    let p = cx.prefs;
    let label = tr(o.label);
    let mut row = match &o.kind {
        Kind::Toggle { get, .. } => Row::toggle(o.id, label, get(p)),
        Kind::Choice { names, get, .. } => {
            Row::choice(o.id, label, names().into_iter().map(SharedString::from).collect(), get(p)).compact()
        }
        Kind::Number { min, max, step, suffix, get, .. } => Row::number(o.id, label, get(p), *min, *max, *step, suffix),
        Kind::Text { get, .. } | Kind::File { get, .. } => Row::text(o.id, label, get(p)),
    };
    if !o.desc.is_empty() {
        row = row.desc(tr(o.desc));
    }
    if o.id == "background.image" {
        let own = p.theme.background.source == BackgroundSource::Image;
        row = row.enabled(own);
    }
    if !applies(o.id, &p.theme) {
        row = row.enabled(false).desc(tr!("No tab uses a layout this applies to."));
    }
    row
}

/// Whether the option `id` matters for the tabs the theme has.
pub fn applies(id: &str, theme: &ConsoleTheme) -> bool {
    opt(id).is_none_or(|o| o.on.is_empty() || o.on.iter().any(|l| theme.uses(*l)))
}

/// Whether a tab of the theme shows shelves (a layout with them).
pub fn uses_shelves(theme: &ConsoleTheme) -> bool {
    SHELF_LAYOUTS.iter().any(|l| theme.uses(*l))
}

/// The theme in use differs from the preset or saved theme it started from.
pub fn is_edited(prefs: &ConsolePrefs, saved: &[ConsoleTheme]) -> bool {
    base_theme(prefs, saved).is_none_or(|b| b != prefs.theme)
}

/// The preset or saved theme the prefs started from (`None` once it was deleted).
pub fn base_theme(prefs: &ConsolePrefs, saved: &[ConsoleTheme]) -> Option<ConsoleTheme> {
    presets::by_id(&prefs.based_on).or_else(|| saved.iter().find(|t| t.id == prefs.based_on).cloned())
}

/// The saved theme the prefs are based on, when there is one.
pub fn based_on_saved<'a>(prefs: &ConsolePrefs, saved: &'a [ConsoleTheme]) -> Option<&'a ConsoleTheme> {
    saved.iter().find(|t| t.id == prefs.based_on)
}

/// Built-in themes, then the saved ones, as `(id, name)`.
pub fn theme_list(saved: &[ConsoleTheme]) -> Vec<(String, String)> {
    presets::all()
        .into_iter()
        .map(|t| (t.id.clone(), tr(&t.name).to_string()))
        .chain(saved.iter().map(|t| (t.id.clone(), t.name.clone())))
        .collect()
}

fn theme_rows(cx: &Ctx) -> Vec<Row> {
    let (p, saved) = (cx.prefs, &cx.data.saved);
    let list = theme_list(saved);
    let edited = is_edited(p, saved);
    let current = list.iter().position(|(id, _)| *id == p.based_on).filter(|_| !edited);
    let mut names: Vec<SharedString> = list.iter().map(|(_, n)| SharedString::from(n.clone())).collect();
    if current.is_none() {
        names.push(tr!("Customised").into());
    }
    let index = current.unwrap_or(names.len() - 1);
    let mut v = vec![Row::choice("theme.use", tr!("Theme"), names, index).desc(theme_note(p, saved, edited))];
    let own = based_on_saved(p, saved);
    if let Some(t) = own.filter(|_| edited) {
        v.push(Row::action("theme.save", tr!("Save changes to “{name}”", name = t.name)).icon(Icon::Save));
    }
    v.push(Row::action("theme.save-as", tr!("Save as a new theme…")).icon(Icon::Add));
    if let Some(t) = own {
        v.push(Row::action("theme.rename", tr!("Rename “{name}”…", name = t.name)).icon(Icon::Edit));
        v.push(Row::action("theme.delete", tr!("Delete “{name}”", name = t.name)).icon(Icon::DeleteOutline).danger());
    }
    v.push(Row::action("theme.export", tr!("Export to a file…")).icon(Icon::OpenInNew));
    v.push(Row::action("theme.import", tr!("Import from a file…")).icon(Icon::Download));
    v.push(Row::action("theme.reset", tr!("Back to Classic")).icon(Icon::History));
    v
}

/// The line under the theme row: what it is, and whether it was changed.
pub fn theme_note(p: &ConsolePrefs, saved: &[ConsoleTheme], edited: bool) -> String {
    let base = base_theme(p, saved);
    let text = base.as_ref().map(|b| if b.description.is_empty() { String::new() } else { tr(&b.description).to_string() });
    match (edited, text) {
        (true, Some(t)) if !t.is_empty() => tr!("Changed from the theme it started from. {description}", description = t),
        (true, _) => tr!("Changed from the theme it started from.").into(),
        (false, Some(t)) => t,
        _ => String::new(),
    }
}

fn layout_names() -> Vec<&'static str> {
    names(Layout::ALL, layout_name)
}

fn tab_rows(cx: &Ctx) -> Vec<Row> {
    let tabs = &cx.prefs.theme.tabs;
    let mut v = Vec::new();
    for (i, t) in tabs.iter().enumerate() {
        v.push(Row::header(tr!("Tab {n}", n = i + 1)));
        v.push(Row::text(format!("tab.{i}.name"), tr!("Name"), t.name.clone()));
        v.push(Row::choice(
            format!("tab.{i}.layout"),
            tr!("Layout"),
            layout_names().into_iter().map(SharedString::from).collect(),
            index(Layout::ALL, t.layout),
        ));
        let sources = source_options(&cx.data.facts, &t.source);
        let at = sources.iter().position(|(v, _)| *v == t.source).unwrap_or(0);
        v.push(Row::choice(format!("tab.{i}.source"), tr!("Games"), sources.into_iter().map(|(_, l)| l.into()).collect(), at));
        if i > 0 {
            v.push(Row::action(format!("tab.{i}.up"), tr!("Move earlier")).icon(Icon::ArrowUpward));
        }
        if tabs.len() > 1 {
            v.push(Row::action(format!("tab.{i}.remove"), tr!("Remove this tab")).icon(Icon::Close).danger());
        }
    }
    if tabs.len() < MAX_TABS {
        v.push(Row::action("tab.add", tr!("Add a tab")).icon(Icon::Add));
    }
    v
}

/// What a tab can show, as `(source, label)`: the fixed choices, then the
/// library's statuses, categories, stores and platforms; the current value
/// is kept even when the library has no such thing now.
pub fn source_options(facts: &Facts, current: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = vec![
        ("all".into(), tr!("All games").into()),
        ("installed".into(), tr!("Installed").into()),
        ("not-installed".into(), tr!("Not installed").into()),
        ("favourites".into(), tr!("Favourites").into()),
        ("pc".into(), tr!("PC games").into()),
        ("emulated".into(), tr!("Emulated games").into()),
    ];
    v.extend(facts.statuses.iter().map(|(k, n)| (format!("status:{k}"), tr!("Status: {name}", name = n))));
    v.extend(facts.categories.iter().map(|c| (format!("category:{c}"), tr!("Category: {name}", name = c))));
    v.extend(facts.stores.iter().map(|s| (format!("store:{s}"), tr!("Store: {name}", name = s))));
    v.extend(facts.platforms.iter().map(|s| (format!("platform:{s}"), tr!("Platform: {name}", name = s))));
    if !v.iter().any(|(k, _)| k == current) {
        v.push((current.to_string(), current.to_string()));
    }
    v
}

/// The name a shelf kind shows when it has no title of its own;
/// `status_name` gives that of a status (by its slug or id).
pub fn kind_label(kind: &str, status_name: impl Fn(&str) -> String) -> String {
    let (k, value) = kind.split_once(':').unwrap_or((kind, ""));
    match k {
        "recent" => tr!("Continue playing").into(),
        "favourites" => tr!("Favourites").into(),
        "installed" => trc!("shelf", "Installed").into(),
        "added" => tr!("Recently added").into(),
        "backlog-short" => tr!("Short ones from the backlog").into(),
        "random" => tr!("Pick of the day").into(),
        "forgotten" => tr!("Installed and forgotten").into(),
        "statuses" => tr!("One per status").into(),
        "status" => status_name(value),
        "category" if value == UNCATEGORIZED => tr!("Uncategorized").into(),
        _ => value.to_string(),
    }
}

/// What a shelf kind is, under its name.
pub fn kind_group(kind: &str) -> &'static str {
    match kind.split_once(':').map_or(kind, |(k, _)| k) {
        "recent" => tr!("The ones played last"),
        "added" => tr!("The newest in the library"),
        "backlog-short" => tr!("Plan to play, up to 15 hours"),
        "random" => tr!("Six from the backlog, new every day"),
        "forgotten" => tr!("Installed, not played for 3 months"),
        "statuses" => tr!("A shelf for each status of the library"),
        "status" => tr!("Status"),
        "platform" => tr!("Platform"),
        "store" => tr!("Store"),
        "genre" => tr!("Genre"),
        "category" => tr!("Category"),
        _ => "",
    }
}

/// The name a shelf shows: its own title, else its kind's.
pub fn shelf_title(s: &ShelfSpec, status_name: impl Fn(&str) -> String) -> String {
    if s.title.trim().is_empty() {
        kind_label(&s.kind, status_name)
    } else {
        s.title.clone()
    }
}

fn shelf_rows(cx: &Ctx) -> Vec<Row> {
    let mut v: Vec<Row> = cx
        .prefs
        .theme
        .shelves
        .iter()
        .enumerate()
        .map(|(i, s)| Row::toggle(format!("shelf.{i}"), shelf_title(s, |k| cx.data.facts.status_name(k)), s.on).desc(kind_group(&s.kind)))
        .collect();
    v.push(Row::action("shelf.add", tr!("Add a shelf…")).icon(Icon::Add));
    v
}

/// The shelf kinds that are not in the theme yet, grouped, for the picker:
/// `(group, kind, name)`.
pub fn addable_kinds(cx: &Ctx) -> Vec<(&'static str, String, String)> {
    let f = &cx.data.facts;
    let have: BTreeSet<&str> = cx.prefs.theme.shelves.iter().map(|s| s.kind.as_str()).collect();
    let mut out = Vec::new();
    for k in ["recent", "favourites", "installed", "statuses", "added", "backlog-short", "random", "forgotten"] {
        out.push((tr!("Shelves"), k.to_string(), kind_label(k, |k| f.status_name(k))));
    }
    out.extend(f.statuses.iter().map(|(k, n)| (tr!("Status"), format!("status:{k}"), n.clone())));
    out.extend(f.platforms.iter().map(|n| (tr!("Platform"), format!("platform:{n}"), n.clone())));
    out.extend(f.stores.iter().map(|n| (tr!("Store"), format!("store:{n}"), n.clone())));
    out.extend(f.genres.iter().map(|n| (tr!("Genre"), format!("genre:{n}"), n.clone())));
    out.extend(f.categories.iter().map(|n| (tr!("Category"), format!("category:{n}"), n.clone())));
    out.retain(|(_, k, _)| !have.contains(k.as_str()));
    out
}

fn pick_rows(cx: &Ctx) -> Vec<Row> {
    let mut v = Vec::new();
    let mut group = "";
    for (g, kind, name) in addable_kinds(cx) {
        if g != group {
            v.push(Row::header(g));
            group = g;
        }
        v.push(Row::action(kind, name));
    }
    v
}

/// The place of a named focus colour among [`ACCENTS`] (their number for a custom one).
fn accent_index(accent: &str) -> usize {
    ACCENTS.iter().position(|(n, _)| *n == accent).unwrap_or(ACCENTS.len())
}

fn accent_rows(cx: &Ctx) -> Vec<Row> {
    let accent = &cx.prefs.theme.interface.accent;
    let names: Vec<SharedString> = vec![
        tr!("White").into(),
        tr!("Colour of the game").into(),
        tr!("Ice").into(),
        tr!("Green").into(),
        tr!("Amber").into(),
        tr!("Pink").into(),
        tr!("Violet").into(),
        tr!("Custom").into(),
    ];
    let mut v = vec![Row::choice("interface.accent", tr!("Focus colour"), names, accent_index(accent)).compact()];
    if accent_index(accent) == ACCENTS.len() {
        v.push(Row::text("interface.accent_hex", tr!("Custom focus colour"), accent.clone()).desc(tr!("A colour as #rrggbb.")));
    }
    v
}

pub fn menu_item_name(item: &str) -> &'static str {
    match item {
        "collection" => tr!("Collection"),
        "downloads" => tr!("Downloads"),
        "wine-manager" => tr!("Wine Manager"),
        "stores" => tr!("Stores"),
        "emulators" => tr!("Emulators"),
        "statistics" => tr!("Statistics"),
        "add-game" => tr!("Add game"),
        "customize" => tr!("Customize"),
        "settings" => tr!("Settings"),
        "desktop" => tr!("Desktop mode"),
        "power" => tr!("Power"),
        "separator" => tr!("Separator"),
        _ => "",
    }
}

fn menu_rows(cx: &Ctx) -> Vec<Row> {
    cx.prefs
        .theme
        .menu
        .items
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let id = format!("menu.{i}");
            let name = menu_item_name(&m.item);
            let name = if name.is_empty() { m.item.as_str() } else { name };
            if m.item == "separator" {
                Row::info(id, name, "")
            } else if MENU_LOCKED.contains(&m.item.as_str()) {
                Row::toggle(id, name, true).enabled(false).desc(tr!("Always in the menu: it is the way back."))
            } else {
                Row::toggle(id, name, m.on)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
