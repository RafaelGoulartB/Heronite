//! The built-in console themes. `classic` is console mode as it was before
//! themes (the default); the others start from it and change what makes
//! them different. Their names and descriptions are English keys translated
//! where they are shown (`n_!` marks them for the translation coverage test).

use heronite_i18n::n_;

use super::*;

/// The ☰ menu of today, plus Customize.
pub fn default_menu() -> Vec<MenuItemSpec> {
    [
        "collection",
        "downloads",
        "wine-manager",
        "stores",
        "emulators",
        "statistics",
        "separator",
        "add-game",
        "customize",
        "settings",
        "separator",
        "desktop",
        "power",
    ]
    .into_iter()
    .map(MenuItemSpec::new)
    .collect()
}

/// Continue playing, Favourites, Installed, one per status (the statuses of
/// a new library), then the new kinds switched off.
pub fn default_shelves() -> Vec<ShelfSpec> {
    let on = ["recent", "favourites", "installed", "status:playing", "status:endless", "status:plan-to-play", "status:on-hold", "status:played", "status:beaten"];
    let off = ["added", "backlog-short", "random", "forgotten"];
    on.into_iter().map(|k| ShelfSpec::new(k, true)).chain(off.into_iter().map(|k| ShelfSpec::new(k, false))).collect()
}

fn tab(name: &str, layout: Layout) -> TabSpec {
    TabSpec { name: name.into(), layout, source: "all".into() }
}

pub fn classic() -> ConsoleTheme {
    ConsoleTheme {
        format: FORMAT,
        id: "classic".into(),
        name: n_!("Classic").into(),
        author: "Heronite".into(),
        description: n_!("Console mode as it shipped: shelves and a cover grid.").into(),
        version: "1".into(),
        tabs: vec![tab(n_!("Home"), Layout::Shelves), tab(n_!("Library"), Layout::Grid)],
        shelves: default_shelves(),
        shelf_limit: 20,
        dim_others: true,
        nav: NavOptions::default(),
        hero: HeroOptions::default(),
        tiles: TileOptions::default(),
        background: BackgroundOptions::default(),
        interface: InterfaceOptions::default(),
        colors: ColourOptions::default(),
        controls: ControlOptions::default(),
        game_page: GamePage::Classic,
        menu: MenuOptions { style: MenuStyle::Sidebar, side: Side::Left, items: default_menu() },
        layouts: LayoutOptions::default(),
        extra: Map::new(),
    }
}

fn from_classic(id: &str, name: &str, description: &str, edit: impl FnOnce(&mut ConsoleTheme)) -> ConsoleTheme {
    let mut t = classic();
    t.id = id.into();
    t.name = name.into();
    t.description = description.into();
    edit(&mut t);
    t
}

pub fn spotlight() -> ConsoleTheme {
    from_classic("spotlight", n_!("Spotlight"), n_!("One game at a time over its art, with its actions at hand."), |t| {
        t.tabs = vec![tab(n_!("Home"), Layout::Spotlight), tab(n_!("Library"), Layout::Grid)];
        t.hero.title = HeroTitle::Logo;
        t.interface.accent = "game".into();
        t.layouts.grid_panel = GridPanel::Right;
    })
}

pub fn cinema() -> ConsoleTheme {
    from_classic("cinema", n_!("Cinema"), n_!("Full-screen art that drifts, wide covers, hints that fade."), |t| {
        t.tabs = vec![tab(n_!("Home"), Layout::Cinema), tab(n_!("Library"), Layout::Grid)];
        t.tiles.shape = TileShape::Landscape;
        t.tiles.focus = FocusStyle::Lift;
        t.hero.title = HeroTitle::Logo;
        t.background.motion = BackgroundMotion::Drift;
        t.controls.hints = Hints::Auto;
        t.interface.status_icons = false;
        t.layouts.grid_panel = GridPanel::Right;
    })
}

pub fn dashboard() -> ConsoleTheme {
    from_classic("dashboard", n_!("Dashboard"), n_!("Tiles with the current game, your week and a pick from the backlog."), |t| {
        t.tabs = vec![tab(n_!("Dashboard"), Layout::Dashboard), tab(n_!("Library"), Layout::Grid), tab(n_!("List"), Layout::Wheel)];
        t.interface.accent = "ice".into();
        t.interface.date = true;
        t.layouts.grid_panel = GridPanel::Right;
    })
}

pub fn xbox() -> ConsoleTheme {
    from_classic("xbox", n_!("Xbox"), n_!("A row of tiles over the game's art, the newest one wide, groups below."), |t| {
        t.tabs = vec![tab(n_!("Home"), Layout::Xbox), tab(n_!("Library"), Layout::Grid)];
        t.tiles.corners = Corners::Soft;
        t.tiles.shape = TileShape::Square;
        t.interface.accent = "green".into();
        t.interface.font = ConsoleFont::System;
        t.layouts.grid_panel = GridPanel::Right;
        t.menu.style = MenuStyle::Dock;
    })
}

pub fn retro() -> ConsoleTheme {
    from_classic("retro", n_!("Retro"), n_!("For emulators: a wheel of names, square corners, a glow."), |t| {
        t.tabs = vec![tab(n_!("List"), Layout::Wheel), tab(n_!("Library"), Layout::Grid)];
        t.background.source = BackgroundSource::Cover;
        t.background.blur = 24;
        t.background.dim = 30;
        t.interface.accent = "amber".into();
        t.interface.font = ConsoleFont::Mono;
        t.tiles.corners = Corners::Square;
        t.tiles.focus = FocusStyle::Glow;
        t.controls.glyphs = Glyphs::PlayStation;
    })
}

pub fn handheld() -> ConsoleTheme {
    from_classic("handheld", n_!("Handheld"), n_!("Steam Deck and small screens: everything bigger, the Deck's buttons."), |t| {
        t.interface.scale = 120;
        t.tiles.size = TileSize::Large;
        t.controls.glyphs = Glyphs::Deck;
        t.hero.overview = false;
        t.interface.clock = Clock::H12;
    })
}

pub fn minimal() -> ConsoleTheme {
    from_classic("minimal", n_!("Minimal"), n_!("Only the covers: a carousel, no hints, no badges."), |t| {
        t.tabs = vec![tab(n_!("Games"), Layout::Carousel)];
        t.controls.hints = Hints::Off;
        t.interface.status_icons = false;
        t.tiles.badges = Badges::Off;
        t.background.source = BackgroundSource::Cover;
        t.background.blur = 40;
        t.background.dim = 40;
        t.layouts.carousel_reflect = true;
    })
}

/// Every built-in theme, Classic first.
pub fn all() -> Vec<ConsoleTheme> {
    vec![classic(), spotlight(), cinema(), dashboard(), xbox(), retro(), handheld(), minimal()]
}

pub fn by_id(id: &str) -> Option<ConsoleTheme> {
    all().into_iter().find(|t| t.id == id)
}
