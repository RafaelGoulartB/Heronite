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

mod host;
mod labels;

pub use host::*;

use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui_kit::SharedString;
use heronite_data::categories::Categories;
use heronite_data::console_theme::{
    parse_hex, presets, slug, AButton, AutoConsole, BackgroundMotion, BackgroundSource, Badges, Clock, ConsoleFont, ConsolePrefs,
    ConsoleTheme, Corners, FocusStyle, GamePage, GridPanel, Glyphs, HeroTitle, Hints, Idle, Layout, MenuStyle, Motion, OpenIn,
    ShelfSpec, Side, Size3, StartOn, TabSpec, TileShape, TileSize, TileTitles, WheelItems, MAX_TABS, MENU_LOCKED,
};
use heronite_data::{CollectionGame, Library};
use heronite_i18n::{n_, tr, trc};

use super::list::{Row, Value};
use crate::icons::Icon;
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

    fn status_name(&self, key: &str) -> String {
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
// The table of plain options.

enum Kind {
    Toggle { get: fn(&ConsolePrefs) -> bool, set: fn(&mut ConsolePrefs, bool) },
    Choice { names: fn() -> Vec<&'static str>, get: fn(&ConsolePrefs) -> usize, set: fn(&mut ConsolePrefs, usize) },
    Number { min: i64, max: i64, step: i64, suffix: &'static str, get: fn(&ConsolePrefs) -> i64, set: fn(&mut ConsolePrefs, i64) },
    /// Typed on the keyboard; `set` says whether it took the text.
    Text { get: fn(&ConsolePrefs) -> String, set: fn(&mut ConsolePrefs, &str) -> bool },
    /// A picture file.
    File { get: fn(&ConsolePrefs) -> String, set: fn(&mut ConsolePrefs, &str) -> bool },
}

struct Opt {
    id: &'static str,
    card: Card,
    /// English key; `desc` too ("" = none).
    label: &'static str,
    desc: &'static str,
    /// The layouts it matters for (empty = all).
    on: &'static [Layout],
    kind: Kind,
}

macro_rules! tgl {
    ($id:literal, $card:ident, $label:expr, $desc:expr, $on:expr, $($f:ident).+) => {
        Opt {
            id: $id,
            card: Card::$card,
            label: $label,
            desc: $desc,
            on: $on,
            kind: Kind::Toggle { get: |p| p.$($f).+, set: |p, v| p.$($f).+ = v },
        }
    };
}

macro_rules! sel {
    ($id:literal, $card:ident, $label:expr, $desc:expr, $on:expr, $ty:ident, $name:ident, $($f:ident).+) => {
        Opt {
            id: $id,
            card: Card::$card,
            label: $label,
            desc: $desc,
            on: $on,
            kind: Kind::Choice {
                names: || names($ty::ALL, $name),
                get: |p| index($ty::ALL, p.$($f).+),
                set: |p, i| p.$($f).+ = $ty::ALL[i],
            },
        }
    };
}

macro_rules! num {
    ($id:literal, $card:ident, $label:expr, $desc:expr, $min:expr, $max:expr, $step:expr, $suffix:expr, $ty:ty, $($f:ident).+) => {
        Opt {
            id: $id,
            card: Card::$card,
            label: $label,
            desc: $desc,
            on: &[],
            kind: Kind::Number {
                min: $min,
                max: $max,
                step: $step,
                suffix: $suffix,
                get: |p| i64::from(p.$($f).+),
                set: |p, v| p.$($f).+ = v as $ty,
            },
        }
    };
}

/// A colour of `theme.colors`: `#rrggbb`, or empty for the built-in one.
macro_rules! colour {
    ($id:literal, $label:expr, $($f:ident).+) => {
        Opt {
            id: $id,
            card: Card::Colours,
            label: $label,
            desc: n_!("Empty keeps the built-in colour."),
            on: &[],
            kind: Kind::Text {
                get: |p| p.$($f).+.clone(),
                set: |p, s| match normalize_colour(s) {
                    Some(c) => {
                        p.$($f).+ = c;
                        true
                    }
                    None => false,
                },
            },
        }
    };
}

const SHELF_LAYOUTS: &[Layout] = &[Layout::Shelves, Layout::Cinema, Layout::Spotlight, Layout::Xbox];
const LIMITS: [u32; 4] = [10, 20, 40, 0];

static OPTS: &[Opt] = &[
    sel!("nav.start_on", Navigation, n_!("Open on"), n_!("Which tab the console starts on."), &[], StartOn, start_on_name, theme.nav.start_on),
    sel!("nav.a_button", Navigation, n_!("A on a cover"), n_!("X does the other."), &[], AButton, a_button_name, theme.nav.a_button),
    tgl!("nav.wrap", Navigation, n_!("Wrap around at the ends"), n_!("Right on the last one goes back to the first."), &[], theme.nav.wrap),
    Opt {
        id: "shelf_limit",
        card: Card::Shelves,
        label: n_!("Covers per shelf"),
        desc: "",
        on: SHELF_LAYOUTS,
        kind: Kind::Choice {
            names: || vec!["10", "20", "40", tr!("All")],
            get: |p| LIMITS.iter().position(|l| *l == p.theme.shelf_limit).unwrap_or(1),
            set: |p, i| p.theme.shelf_limit = LIMITS[i],
        },
    },
    tgl!("dim_others", Shelves, n_!("Dim the other shelves"), n_!("The shelves without the focus step back."), &[Layout::Shelves, Layout::Cinema], theme.dim_others),
    sel!("hero.title", Hero, n_!("Game name"), "", &[], HeroTitle, hero_title_name, theme.hero.title),
    tgl!("hero.facts", Hero, n_!("Store, installed, time played and status"), "", &[], theme.hero.facts),
    tgl!("hero.overview", Hero, n_!("Description"), "", &[], theme.hero.overview),
    sel!("hero.size", Hero, n_!("Height of the hero"), "", &[Layout::Shelves], Size3, size3_name, theme.hero.size),
    sel!("tiles.shape", Covers, n_!("Shape"), n_!("Landscape uses the game's wide art."), &[], TileShape, tile_shape_name, theme.tiles.shape),
    sel!("tiles.size", Covers, n_!("Size"), "", &[], TileSize, tile_size_name, theme.tiles.size),
    sel!("tiles.titles", Covers, n_!("Name below the cover"), "", &[], TileTitles, tile_titles_name, theme.tiles.titles),
    sel!("tiles.badges", Covers, n_!("Badges"), n_!("All adds favourite and downloading."), &[], Badges, badges_name, theme.tiles.badges),
    sel!("tiles.corners", Covers, n_!("Corners"), "", &[], Corners, corners_name, theme.tiles.corners),
    sel!("tiles.focus", Covers, n_!("Focus"), "", &[], FocusStyle, focus_name, theme.tiles.focus),
    tgl!("tiles.grey", Covers, n_!("Grey out games that are not installed"), "", &[], theme.tiles.grey_uninstalled),
    sel!("background.source", Background, n_!("Picture"), "", &[], BackgroundSource, background_name, theme.background.source),
    Opt {
        id: "background.image",
        card: Card::Background,
        label: n_!("Your own picture"),
        desc: n_!("Used when the picture is “Your own picture”."),
        on: &[],
        kind: Kind::File {
            get: |p| p.theme.background.image.clone(),
            set: |p, s| {
                p.theme.background.image = s.trim().to_string();
                true
            },
        },
    },
    num!("background.dim", Background, n_!("Darken"), "", 0, 90, 10, "%", u8, theme.background.dim),
    num!("background.blur", Background, n_!("Blur"), "", 0, 40, 4, " px", u8, theme.background.blur),
    sel!("background.motion", Background, n_!("Motion"), "", &[], BackgroundMotion, bg_motion_name, theme.background.motion),
    sel!("interface.font", Interface, n_!("Font"), "", &[], ConsoleFont, font_name, theme.interface.font),
    num!("interface.scale", Interface, n_!("Interface size"), n_!("For the distance from the couch."), 80, 140, 5, "%", u16, theme.interface.scale),
    sel!("interface.clock", Interface, n_!("Clock"), "", &[], Clock, clock_name, theme.interface.clock),
    tgl!("interface.date", Interface, n_!("Date next to the clock"), "", &[], theme.interface.date),
    Opt {
        id: "interface.label",
        card: Card::Interface,
        label: n_!("Text in the top-left corner"),
        desc: n_!("Empty shows nothing."),
        on: &[],
        kind: Kind::Text {
            get: |p| p.theme.interface.label.clone(),
            set: |p, s| {
                p.theme.interface.label = s.trim().chars().take(24).collect();
                true
            },
        },
    },
    tgl!("interface.status_icons", Interface, n_!("Status icons"), n_!("Controller, downloads and battery."), &[], theme.interface.status_icons),
    sel!("interface.motion", Interface, n_!("Animations"), "", &[], Motion, motion_name, theme.interface.motion),
    colour!("colors.background", n_!("Background colour"), theme.colors.background),
    colour!("colors.panel", n_!("Panels"), theme.colors.panel),
    colour!("colors.text", n_!("Text"), theme.colors.text),
    colour!("colors.muted", n_!("Secondary text"), theme.colors.muted),
    colour!("colors.focus_text", n_!("Text on the focus colour"), theme.colors.focus_text),
    colour!("colors.on", n_!("“On” switches"), theme.colors.on),
    sel!("controls.glyphs", Controls, n_!("Buttons drawn as"), "", &[], Glyphs, glyphs_name, theme.controls.glyphs),
    sel!("controls.hints", Controls, n_!("Hint bar"), "", &[], Hints, hints_name, theme.controls.hints),
    tgl!("controls.swap_ab", Controls, n_!("Confirm with the east button"), n_!("The Nintendo way: east confirms, south goes back."), &[], theme.controls.swap_ab),
    tgl!("controls.sounds", Controls, n_!("Navigation sounds"), "", &[], theme.controls.sounds),
    sel!("game_page", GamePage, n_!("Game page"), "", &[], GamePage, game_page_name, theme.game_page),
    sel!("menu.style", GamePage, n_!("Menu"), "", &[], MenuStyle, menu_style_name, theme.menu.style),
    sel!("menu.side", GamePage, n_!("Side of the menu"), "", &[], Side, side_name, theme.menu.side),
    sel!("layouts.grid_panel", Layouts, n_!("Grid: the focused game"), "", &[Layout::Grid], GridPanel, grid_panel_name, theme.layouts.grid_panel),
    tgl!("layouts.jump_letters", Layouts, n_!("Grid: jump by letter"), n_!("LT and RT go to the next and previous letter."), &[Layout::Grid], theme.layouts.jump_letters),
    sel!("layouts.wheel_items", Layouts, n_!("Wheel: items"), "", &[Layout::Wheel], WheelItems, wheel_items_name, theme.layouts.wheel_items),
    tgl!("layouts.spotlight_cards", Layouts, n_!("Spotlight: cards"), n_!("Time played, time to beat and the journal."), &[Layout::Spotlight], theme.layouts.spotlight_cards),
    tgl!("layouts.carousel_reflect", Layouts, n_!("Carousel: reflection"), "", &[Layout::Carousel], theme.layouts.carousel_reflect),
    tgl!("layouts.xbox_groups", Layouts, n_!("Xbox: groups under the tiles"), "", &[Layout::Xbox], theme.layouts.xbox_groups),
    tgl!("layouts.dashboard.week", Layouts, n_!("Dashboard: this week"), "", &[Layout::Dashboard], theme.layouts.dashboard.week),
    tgl!("layouts.dashboard.downloads", Layouts, n_!("Dashboard: downloads"), "", &[Layout::Dashboard], theme.layouts.dashboard.downloads),
    tgl!("layouts.dashboard.random", Layouts, n_!("Dashboard: a pick from the backlog"), "", &[Layout::Dashboard], theme.layouts.dashboard.random),
    tgl!("layouts.dashboard.journal", Layouts, n_!("Dashboard: the journal"), "", &[Layout::Dashboard], theme.layouts.dashboard.journal),
    tgl!("layouts.dashboard.shortcuts", Layouts, n_!("Dashboard: shortcuts"), n_!("Emulators and Stores."), &[Layout::Dashboard], theme.layouts.dashboard.shortcuts),
    sel!("startup.open_in", Startup, n_!("Open the app"), "", &[], OpenIn, open_in_name, open_in),
    sel!("startup.auto_console", Startup, n_!("When a controller connects"), "", &[], AutoConsole, auto_console_name, auto_console),
    sel!("startup.idle", Startup, n_!("After 5 minutes without input"), "", &[], Idle, idle_name, idle),
];

fn opt(id: &str) -> Option<&'static Opt> {
    OPTS.iter().find(|o| o.id == id)
}

/// `#rrggbb` from what was typed; empty stays empty; `None` is not a colour.
pub fn normalize_colour(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return Some(String::new());
    }
    let (r, g, b) = parse_hex(s)?;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

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

/// The theme in use differs from the preset or saved theme it started from.
pub fn is_edited(prefs: &ConsolePrefs, saved: &[ConsoleTheme]) -> bool {
    base_theme(prefs, saved).is_none_or(|b| b != prefs.theme)
}

fn base_theme(prefs: &ConsolePrefs, saved: &[ConsoleTheme]) -> Option<ConsoleTheme> {
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

/// The English key of a layout's name.
pub fn layout_label(l: Layout) -> &'static str {
    layout_name(l)
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

/// The name a shelf kind shows when it has no title of its own.
pub fn kind_label(facts: &Facts, kind: &str) -> String {
    let (k, value) = kind.split_once(':').unwrap_or((kind, ""));
    match k {
        "recent" => tr!("Continue playing").into(),
        "favourites" => tr!("Favourites").into(),
        "installed" => trc!("shelf", "Installed").into(),
        "added" => tr!("Recently added").into(),
        "backlog-short" => tr!("Short ones from the backlog").into(),
        "random" => tr!("Pick of the day").into(),
        "forgotten" => tr!("Installed and forgotten").into(),
        "status" => facts.status_name(value),
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
        "status" => tr!("Status"),
        "platform" => tr!("Platform"),
        "store" => tr!("Store"),
        "genre" => tr!("Genre"),
        "category" => tr!("Category"),
        _ => "",
    }
}

pub fn shelf_title(facts: &Facts, s: &ShelfSpec) -> String {
    if s.title.trim().is_empty() {
        kind_label(facts, &s.kind)
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
        .map(|(i, s)| Row::toggle(format!("shelf.{i}"), shelf_title(&cx.data.facts, s), s.on).desc(kind_group(&s.kind)))
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
    for k in ["recent", "favourites", "installed", "added", "backlog-short", "random", "forgotten"] {
        out.push((tr!("Shelves"), k.to_string(), kind_label(f, k)));
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

const ACCENT_NAMES: [&str; 7] = ["white", "game", "ice", "green", "amber", "pink", "violet"];

fn accent_index(accent: &str) -> usize {
    ACCENT_NAMES.iter().position(|n| *n == accent).unwrap_or(ACCENT_NAMES.len())
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
    if accent_index(accent) == ACCENT_NAMES.len() {
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

// ---------------------------------------------------------------------------
// What a press asks for, and applying it.

/// A theme operation that reads or writes files (run by [`host`]).
#[derive(Clone, Debug, PartialEq)]
pub enum StoreOp {
    /// Overwrite the saved theme in use.
    Save,
    SaveAs { name: String, author: String },
    Rename(String),
    Delete,
    Export,
    Import,
}

/// What a press on an action / text row asks the host for.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Nothing,
    Run(Command),
    /// Type a text, then [`Command`] `id` with [`Arg::Text`] (the theme's
    /// save-as and rename go to [`StoreOp`] instead).
    Text { id: String, title: String, initial: String },
    /// Pick a picture file.
    File { id: String, title: String, start: Option<PathBuf> },
    /// Pick one of `rows`, then [`Command`] `id` with [`Arg::Pick`].
    Pick { id: String, title: String, rows: Vec<Row> },
    /// Ask first; yes runs [`Step::Store`].
    Confirm { title: String, message: String, yes: String, then: StoreOp },
    Store(StoreOp),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Value(Value),
    Text(String),
    Press,
    /// Earlier (`-1`) or later (`1`) in its list.
    Move(i32),
    Remove,
    Pick(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub id: String,
    pub arg: Arg,
}

impl Command {
    pub fn new(id: impl Into<String>, arg: Arg) -> Self {
        Self { id: id.into(), arg }
    }

    /// A row's value changed.
    pub fn changed(id: &str, value: Value) -> Self {
        Self::new(id, Arg::Value(value))
    }
}

fn part<'a>(id: &'a str, prefix: &str) -> Option<&'a str> {
    id.strip_prefix(prefix)
}

/// `tab.2.name` with `("tab.", ".name")` gives 2.
fn indexed(id: &str, prefix: &str, suffix: &str) -> Option<usize> {
    part(id, prefix)?.strip_suffix(suffix)?.parse().ok()
}

fn tab_index(id: &str, suffix: &str) -> Option<usize> {
    indexed(id, "tab.", suffix)
}

fn flat_index(id: &str, prefix: &str) -> Option<usize> {
    part(id, prefix)?.parse().ok()
}

/// What A on `id` does.
pub fn activate(id: &str, cx: &Ctx) -> Step {
    let p = cx.prefs;
    let press = || Step::Run(Command::new(id, Arg::Press));
    match id {
        "theme.save" => return Step::Store(StoreOp::Save),
        "theme.save-as" => {
            return Step::Text { id: id.into(), title: tr!("Name of the theme").into(), initial: String::new() };
        }
        "theme.rename" => {
            let initial = based_on_saved(p, &cx.data.saved).map(|t| t.name.clone()).unwrap_or_default();
            return Step::Text { id: id.into(), title: tr!("New name").into(), initial };
        }
        "theme.delete" => {
            let name = based_on_saved(p, &cx.data.saved).map(|t| t.name.clone()).unwrap_or_default();
            return Step::Confirm {
                title: tr!("Delete “{name}”?", name).into(),
                message: tr!("The theme file is deleted. The look on screen stays as it is.").into(),
                yes: tr!("Delete").into(),
                then: StoreOp::Delete,
            };
        }
        "theme.export" => return Step::Store(StoreOp::Export),
        "theme.import" => return Step::Store(StoreOp::Import),
        "theme.reset" | "tab.add" => return press(),
        "shelf.add" => {
            return Step::Pick { id: id.into(), title: tr!("Add a shelf").into(), rows: pick_rows(cx) };
        }
        "interface.accent_hex" => {
            return Step::Text { id: id.into(), title: tr!("Custom focus colour").into(), initial: p.theme.interface.accent.clone() };
        }
        _ => {}
    }
    if let Some(i) = tab_index(id, ".name") {
        return Step::Text { id: id.into(), title: tr!("Name of the tab").into(), initial: p.theme.tabs.get(i).map(|t| t.name.clone()).unwrap_or_default() };
    }
    if tab_index(id, ".up").is_some() || tab_index(id, ".remove").is_some() {
        return press();
    }
    match opt(id).map(|o| &o.kind) {
        Some(Kind::Text { get, .. }) => {
            let o = opt(id).expect("found above");
            Step::Text { id: id.into(), title: tr(o.label).into(), initial: get(p) }
        }
        Some(Kind::File { get, .. }) => {
            let current = PathBuf::from(get(p));
            Step::File { id: id.into(), title: tr!("Choose a picture").into(), start: current.parent().map(PathBuf::from) }
        }
        _ => Step::Nothing,
    }
}

/// Whether the row can be moved with X and ↑ ↓.
pub fn movable(id: &str) -> bool {
    flat_index(id, "shelf.").is_some() || flat_index(id, "menu.").is_some()
}

/// Whether LT removes the row.
pub fn removable(id: &str) -> bool {
    flat_index(id, "shelf.").is_some()
}

/// Moves `v[i]` one place; gives its new index.
pub fn shift<T>(v: &mut [T], i: usize, dir: i32) -> Option<usize> {
    let to = if dir < 0 { i.checked_sub(1)? } else { i + 1 };
    (to < v.len() && i < v.len()).then(|| {
        v.swap(i, to);
        to
    })
}

fn first_unused_layout(tabs: &[TabSpec]) -> Layout {
    Layout::ALL.iter().copied().find(|l| tabs.iter().all(|t| t.layout != *l)).unwrap_or(Layout::Shelves)
}

/// Applies `cmd` to the prefs; whether anything changed.
pub fn run(prefs: &mut ConsolePrefs, cmd: &Command, data: &Data) -> bool {
    let before = prefs.clone();
    apply(prefs, cmd, data);
    *prefs != before
}

fn apply(prefs: &mut ConsolePrefs, cmd: &Command, data: &Data) {
    let id = cmd.id.as_str();
    let t = &mut prefs.theme;
    match (id, &cmd.arg) {
        ("theme.use", Arg::Value(Value::Choice { index, .. })) => {
            let list = theme_list(&data.saved);
            if let Some((tid, _)) = list.get(*index) {
                if let Some(next) = presets::by_id(tid).or_else(|| data.saved.iter().find(|s| s.id == *tid).cloned()) {
                    prefs.theme = next;
                    prefs.based_on = tid.clone();
                }
            }
        }
        ("theme.reset", Arg::Press) => {
            prefs.theme = presets::classic();
            prefs.based_on = "classic".into();
        }
        ("tab.add", Arg::Press) => {
            if t.tabs.len() < MAX_TABS {
                let layout = first_unused_layout(&t.tabs);
                t.tabs.push(TabSpec { name: tr(layout_name(layout)).to_string(), layout, source: "all".into() });
            }
        }
        ("interface.accent", Arg::Value(Value::Choice { index, .. })) => {
            t.interface.accent = match ACCENT_NAMES.get(*index) {
                Some(n) => (*n).to_string(),
                None if accent_index(&t.interface.accent) == ACCENT_NAMES.len() => t.interface.accent.clone(),
                None => "#a5dceb".into(),
            };
        }
        ("interface.accent_hex", Arg::Text(s)) => {
            if let Some(c) = normalize_colour(s).filter(|c| !c.is_empty()) {
                t.interface.accent = c;
            }
        }
        ("shelf.add", Arg::Pick(kind)) => {
            if !t.shelves.iter().any(|s| s.kind == *kind) {
                t.shelves.push(ShelfSpec::new(kind, true));
            }
        }
        _ => apply_indexed(prefs, cmd, data),
    }
}

fn apply_indexed(prefs: &mut ConsolePrefs, cmd: &Command, data: &Data) {
    let id = cmd.id.as_str();
    let t = &mut prefs.theme;
    if let Some(i) = tab_index(id, ".name") {
        if let (Arg::Text(s), Some(tab)) = (&cmd.arg, t.tabs.get_mut(i)) {
            tab.name = s.trim().chars().take(20).collect();
        }
    } else if let Some(i) = tab_index(id, ".layout") {
        if let (Arg::Value(Value::Choice { index, .. }), Some(tab)) = (&cmd.arg, t.tabs.get_mut(i)) {
            if let Some(l) = Layout::ALL.get(*index) {
                // A tab still called by its layout's name follows it.
                if tab.name == tr(layout_name(tab.layout)) {
                    tab.name = tr(layout_name(*l)).to_string();
                }
                tab.layout = *l;
            }
        }
    } else if let Some(i) = tab_index(id, ".source") {
        if let (Arg::Value(Value::Choice { index, .. }), Some(tab)) = (&cmd.arg, t.tabs.get_mut(i)) {
            if let Some((source, _)) = source_options(&data.facts, &tab.source).get(*index) {
                tab.source = source.clone();
            }
        }
    } else if let Some(i) = tab_index(id, ".up") {
        shift(&mut t.tabs, i, -1);
    } else if let Some(i) = tab_index(id, ".remove") {
        if t.tabs.len() > 1 && i < t.tabs.len() {
            t.tabs.remove(i);
        }
    } else if let Some(i) = flat_index(id, "shelf.") {
        match &cmd.arg {
            Arg::Value(Value::Toggle(on)) => {
                if let Some(s) = t.shelves.get_mut(i) {
                    s.on = *on;
                }
            }
            Arg::Text(title) => {
                if let Some(s) = t.shelves.get_mut(i) {
                    s.title = title.trim().chars().take(32).collect();
                }
            }
            Arg::Move(dir) => {
                shift(&mut t.shelves, i, *dir);
            }
            Arg::Remove if i < t.shelves.len() => {
                t.shelves.remove(i);
            }
            _ => {}
        }
    } else if let Some(i) = flat_index(id, "menu.") {
        match &cmd.arg {
            Arg::Value(Value::Toggle(on)) => {
                if let Some(m) = t.menu.items.get_mut(i).filter(|m| !MENU_LOCKED.contains(&m.item.as_str()) && m.item != "separator") {
                    m.on = *on;
                }
            }
            Arg::Move(dir) => {
                shift(&mut t.menu.items, i, *dir);
            }
            _ => {}
        }
    } else if let Some(o) = opt(id) {
        apply_opt(prefs, o, &cmd.arg);
    }
}

fn apply_opt(prefs: &mut ConsolePrefs, o: &Opt, arg: &Arg) {
    match (&o.kind, arg) {
        (Kind::Toggle { set, .. }, Arg::Value(Value::Toggle(on))) => set(prefs, *on),
        (Kind::Choice { names, set, .. }, Arg::Value(Value::Choice { index, .. })) if *index < names().len() => set(prefs, *index),
        (Kind::Number { min, max, set, .. }, Arg::Value(Value::Number { value, .. })) => set(prefs, (*value).clamp(*min, *max)),
        (Kind::Text { set, .. } | Kind::File { set, .. }, Arg::Text(s)) => {
            set(prefs, s);
        }
        _ => {}
    }
}

/// The file name an export suggests: `<id>.heronite-theme.json`.
pub fn export_name(theme: &ConsoleTheme) -> String {
    let id = if theme.id.is_empty() { slug(&theme.name) } else { theme.id.clone() };
    format!("{id}.{}", heronite_data::console_theme::store::EXTENSION)
}

#[cfg(test)]
mod tests;
