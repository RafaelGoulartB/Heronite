//! The table of plain options: a switch, a choice, a number or a text each,
//! found by row id.

use super::*;

pub(super) enum Kind {
    Toggle {
        get: fn(&ConsolePrefs) -> bool,
        set: fn(&mut ConsolePrefs, bool),
    },
    Choice {
        names: fn() -> Vec<&'static str>,
        get: fn(&ConsolePrefs) -> usize,
        set: fn(&mut ConsolePrefs, usize),
    },
    Number {
        min: i64,
        max: i64,
        step: i64,
        suffix: &'static str,
        get: fn(&ConsolePrefs) -> i64,
        set: fn(&mut ConsolePrefs, i64),
    },
    /// Typed on the keyboard; `set` says whether it took the text.
    Text {
        get: fn(&ConsolePrefs) -> String,
        set: fn(&mut ConsolePrefs, &str) -> bool,
    },
    /// A picture file.
    File {
        get: fn(&ConsolePrefs) -> String,
        set: fn(&mut ConsolePrefs, &str) -> bool,
    },
}

pub(super) struct Opt {
    pub(super) id: &'static str,
    pub(super) card: Card,
    /// English key; `desc` too ("" = none).
    pub(super) label: &'static str,
    pub(super) desc: &'static str,
    /// The layouts it matters for (empty = all).
    pub(super) on: &'static [Layout],
    pub(super) kind: Kind,
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

pub(super) const SHELF_LAYOUTS: &[Layout] = &[Layout::Shelves, Layout::Cinema, Layout::Spotlight, Layout::Xbox];
pub(super) const LIMITS: [u32; 4] = [10, 20, 40, 0];

pub(super) static OPTS: &[Opt] = &[
    sel!(
        "nav.start_on",
        Navigation,
        n_!("Open on"),
        n_!("Which tab the console starts on."),
        &[],
        StartOn,
        start_on_name,
        theme.nav.start_on
    ),
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
    tgl!(
        "dim_others",
        Shelves,
        n_!("Dim the other shelves"),
        n_!("The shelves without the focus step back."),
        &[Layout::Shelves, Layout::Cinema],
        theme.dim_others
    ),
    sel!("hero.title", Hero, n_!("Game name"), "", &[], HeroTitle, hero_title_name, theme.hero.title),
    tgl!("hero.facts", Hero, n_!("Store, installed, time played and status"), "", &[], theme.hero.facts),
    tgl!("hero.overview", Hero, n_!("Description"), "", &[], theme.hero.overview),
    sel!("hero.size", Hero, n_!("Height of the hero"), "", &[Layout::Shelves], Size3, size3_name, theme.hero.size),
    sel!("hero.fact_style", Hero, n_!("Facts as"), "", &[], FactStyle, fact_style_name, theme.hero.fact_style),
    sel!("hero.align", Hero, n_!("Position of the game"), "", &[Layout::Cinema], HeroAlign, hero_align_name, theme.hero.align),
    tgl!(
        "hero.stats",
        Hero,
        n_!("Numbers at the right edge"),
        n_!("Time played, last played, size on disk and score."),
        &[Layout::Shelves, Layout::Cinema],
        theme.hero.stats
    ),
    sel!(
        "tiles.shape",
        Covers,
        n_!("Shape"),
        n_!("Landscape uses the game's wide art."),
        &[],
        TileShape,
        tile_shape_name,
        theme.tiles.shape
    ),
    sel!("tiles.size", Covers, n_!("Size"), "", &[], TileSize, tile_size_name, theme.tiles.size),
    sel!("tiles.titles", Covers, n_!("Name below the cover"), "", &[], TileTitles, tile_titles_name, theme.tiles.titles),
    sel!(
        "tiles.badges",
        Covers,
        n_!("Badges"),
        n_!("All adds favourite, downloading and new games."),
        &[],
        Badges,
        badges_name,
        theme.tiles.badges
    ),
    sel!("tiles.corners", Covers, n_!("Corners"), "", &[], Corners, corners_name, theme.tiles.corners),
    sel!("tiles.focus", Covers, n_!("Focus"), "", &[], FocusStyle, focus_name, theme.tiles.focus),
    tgl!("tiles.grey", Covers, n_!("Grey out games that are not installed"), "", &[], theme.tiles.grey_uninstalled),
    tgl!("tiles.header", Covers, n_!("Store above the cover"), "", &[], theme.tiles.header),
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
    num!(
        "interface.scale",
        Interface,
        n_!("Interface size"),
        n_!("For the distance from the couch."),
        80,
        140,
        5,
        "%",
        u16,
        theme.interface.scale
    ),
    sel!("interface.tab_bar", Interface, n_!("Tabs"), "", &[], TabBar, tab_bar_name, theme.interface.tab_bar),
    sel!(
        "interface.headings",
        Interface,
        n_!("Shelf titles"),
        "",
        &[Layout::Shelves, Layout::Cinema, Layout::Xbox],
        Headings,
        headings_name,
        theme.interface.headings
    ),
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
    tgl!(
        "interface.status_icons",
        Interface,
        n_!("Status icons"),
        n_!("Controller, downloads and battery."),
        &[],
        theme.interface.status_icons
    ),
    sel!("interface.motion", Interface, n_!("Animations"), "", &[], Motion, motion_name, theme.interface.motion),
    colour!("colors.background", n_!("Background colour"), theme.colors.background),
    colour!("colors.panel", n_!("Panels"), theme.colors.panel),
    colour!("colors.text", n_!("Text"), theme.colors.text),
    colour!("colors.muted", n_!("Secondary text"), theme.colors.muted),
    colour!("colors.focus_text", n_!("Text on the focus colour"), theme.colors.focus_text),
    colour!("colors.on", n_!("“On” switches"), theme.colors.on),
    sel!("controls.glyphs", Controls, n_!("Buttons drawn as"), "", &[], Glyphs, glyphs_name, theme.controls.glyphs),
    sel!("controls.hints", Controls, n_!("Hint bar"), "", &[], Hints, hints_name, theme.controls.hints),
    tgl!(
        "controls.swap_ab",
        Controls,
        n_!("Confirm with the east button"),
        n_!("The Nintendo way: east confirms, south goes back."),
        &[],
        theme.controls.swap_ab
    ),
    tgl!("controls.sounds", Controls, n_!("Navigation sounds"), "", &[], theme.controls.sounds),
    sel!("game_page", GamePage, n_!("Game page"), "", &[], GamePage, game_page_name, theme.game_page),
    sel!("menu.style", GamePage, n_!("Menu"), "", &[], MenuStyle, menu_style_name, theme.menu.style),
    sel!("menu.side", GamePage, n_!("Side of the menu"), "", &[], Side, side_name, theme.menu.side),
    sel!(
        "layouts.grid_panel",
        Layouts,
        n_!("Grid: the focused game"),
        "",
        &[Layout::Grid],
        GridPanel,
        grid_panel_name,
        theme.layouts.grid_panel
    ),
    tgl!(
        "layouts.jump_letters",
        Layouts,
        n_!("Grid: jump by letter"),
        n_!("LT and RT go to the next and previous letter."),
        &[Layout::Grid],
        theme.layouts.jump_letters
    ),
    sel!(
        "layouts.wheel_items",
        Layouts,
        n_!("Wheel: items"),
        "",
        &[Layout::Wheel],
        WheelItems,
        wheel_items_name,
        theme.layouts.wheel_items
    ),
    tgl!(
        "layouts.spotlight_cards",
        Layouts,
        n_!("Spotlight: cards"),
        n_!("Time played, time to beat and the journal."),
        &[Layout::Spotlight],
        theme.layouts.spotlight_cards
    ),
    sel!(
        "layouts.spotlight_actions",
        Layouts,
        n_!("Spotlight: buttons"),
        "",
        &[Layout::Spotlight],
        SpotActions,
        spot_actions_name,
        theme.layouts.spotlight_actions
    ),
    tgl!("layouts.carousel_reflect", Layouts, n_!("Carousel: reflection"), "", &[Layout::Carousel], theme.layouts.carousel_reflect),
    tgl!("layouts.xbox_groups", Layouts, n_!("Series X: groups under the tiles"), "", &[Layout::Xbox], theme.layouts.xbox_groups),
    tgl!("layouts.dashboard.week", Layouts, n_!("Dashboard: this week"), "", &[Layout::Dashboard], theme.layouts.dashboard.week),
    tgl!("layouts.dashboard.downloads", Layouts, n_!("Dashboard: downloads"), "", &[Layout::Dashboard], theme.layouts.dashboard.downloads),
    tgl!(
        "layouts.dashboard.random",
        Layouts,
        n_!("Dashboard: a pick from the backlog"),
        "",
        &[Layout::Dashboard],
        theme.layouts.dashboard.random
    ),
    tgl!("layouts.dashboard.journal", Layouts, n_!("Dashboard: the journal"), "", &[Layout::Dashboard], theme.layouts.dashboard.journal),
    tgl!(
        "layouts.dashboard.shortcuts",
        Layouts,
        n_!("Dashboard: shortcuts"),
        n_!("Emulators and Stores."),
        &[Layout::Dashboard],
        theme.layouts.dashboard.shortcuts
    ),
    sel!("startup.open_in", Startup, n_!("Open the app"), "", &[], OpenIn, open_in_name, open_in),
    sel!("startup.auto_console", Startup, n_!("When a controller connects"), "", &[], AutoConsole, auto_console_name, auto_console),
    sel!("startup.idle", Startup, n_!("After 5 minutes without input"), "", &[], Idle, idle_name, idle),
];

pub(super) fn opt(id: &str) -> Option<&'static Opt> {
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
