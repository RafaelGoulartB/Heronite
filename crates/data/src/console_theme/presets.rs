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

/// Continue playing, Favourites, Installed, one per status of the library
/// (`statuses`), then the new kinds switched off.
pub fn default_shelves() -> Vec<ShelfSpec> {
    let on = ["recent", "favourites", "installed", "statuses"];
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

fn tab_of(name: &str, layout: Layout, source: &str) -> TabSpec {
    TabSpec { name: name.into(), layout, source: source.into() }
}

/// The game's art full screen with its logo large, chips of facts, one row
/// of portrait covers and the tabs along the bottom.
pub fn cinema() -> ConsoleTheme {
    from_classic("cinema", n_!("Cinema"), n_!("The game's art full screen, its logo large, a row of covers and the tabs below."), |t| {
        t.tabs = vec![
            tab_of(n_!("All games"), Layout::Cinema, "all"),
            tab_of(n_!("Installed games"), Layout::Cinema, "installed"),
            tab_of(n_!("Favourites"), Layout::Cinema, "favourites"),
            tab_of(n_!("Library"), Layout::Grid, "all"),
        ];
        t.hero.title = HeroTitle::Logo;
        t.hero.overview = false;
        t.hero.fact_style = FactStyle::Chips;
        t.tiles.corners = Corners::Soft;
        t.interface.tab_bar = TabBar::Bottom;
        t.interface.status_icons = false;
        t.controls.hints = Hints::Auto;
    })
}

/// Art and logo up front, a panel of numbers at the edge, square covers.
pub fn showcase() -> ConsoleTheme {
    from_classic("showcase", n_!("Showcase"), n_!("Art and logo up front, your numbers at the edge, a row of square covers."), |t| {
        t.tabs = vec![tab(n_!("Home"), Layout::Cinema), tab(n_!("Library"), Layout::Grid)];
        t.hero.title = HeroTitle::Logo;
        t.hero.facts = false;
        t.hero.overview = false;
        t.hero.stats = true;
        t.tiles.shape = TileShape::Square;
        t.tiles.corners = Corners::Soft;
        t.tiles.focus = FocusStyle::Glow;
        t.interface.accent = "ice".into();
        t.background.motion = BackgroundMotion::Drift;
        t.layouts.grid_panel = GridPanel::Right;
    })
}

/// A store front: a rail of icons, the game's details and tags, shelves of
/// square covers named under them.
pub fn storefront() -> ConsoleTheme {
    from_classic(
        "storefront",
        n_!("Storefront"),
        n_!("Like a store: a rail of icons, the game's details and shelves of square covers."),
        |t| {
            t.tabs = vec![tab(n_!("Home"), Layout::Shelves), tab(n_!("Library"), Layout::Grid)];
            t.shelves = ["added", "recent", "favourites", "installed", "statuses"]
                .into_iter()
                .map(|k| ShelfSpec::new(k, true))
                .chain(["backlog-short", "random", "forgotten"].into_iter().map(|k| ShelfSpec::new(k, false)))
                .collect();
            t.hero.title = HeroTitle::Logo;
            t.hero.fact_style = FactStyle::Chips;
            t.tiles.shape = TileShape::Square;
            t.tiles.corners = Corners::Square;
            t.tiles.titles = TileTitles::Bar;
            t.tiles.badges = Badges::All;
            t.interface.headings = Headings::Spaced;
            t.interface.tab_bar = TabBar::Title;
            t.interface.font = ConsoleFont::Cabin;
            t.controls.hints = Hints::Auto;
            t.menu.style = MenuStyle::Rail;
        },
    )
}

/// A light theme: the art washed out, the logo centred at the top, square
/// covers under their store's name and a dock for the menu.
pub fn daylight() -> ConsoleTheme {
    from_classic(
        "daylight",
        n_!("Daylight"),
        n_!("A light theme: the art washed out, the logo at the top, covers under their store."),
        |t| {
            t.tabs = vec![tab(n_!("Home"), Layout::Cinema), tab(n_!("Library"), Layout::Grid)];
            t.hero.title = HeroTitle::Logo;
            t.hero.facts = false;
            t.hero.overview = false;
            t.hero.align = HeroAlign::Center;
            t.tiles.shape = TileShape::Square;
            t.tiles.size = TileSize::Large;
            t.tiles.corners = Corners::Square;
            t.tiles.header = true;
            t.dim_others = false;
            t.colors.background = "#eceef2".into();
            t.interface.font = ConsoleFont::Rubik;
            t.menu.style = MenuStyle::Dock;
        },
    )
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

/// Handhelds: bigger, the tab's name at the top, portrait covers with a
/// sharp ring, the connected controller's buttons.
pub fn handheld() -> ConsoleTheme {
    from_classic("handheld", n_!("Handheld"), n_!("For handhelds: bigger, the tab's name at the top, covers with a sharp ring."), |t| {
        t.tabs = vec![
            tab_of(n_!("All games"), Layout::Cinema, "all"),
            tab_of(n_!("Installed games"), Layout::Cinema, "installed"),
            tab_of(n_!("List"), Layout::Wheel, "all"),
        ];
        t.interface.scale = 115;
        t.interface.tab_bar = TabBar::Title;
        t.interface.accent = "ice".into();
        t.hero.title = HeroTitle::Logo;
        t.hero.overview = false;
        t.tiles.corners = Corners::Square;
        t.controls.glyphs = Glyphs::Auto;
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
    vec![classic(), cinema(), showcase(), storefront(), daylight(), xbox(), handheld(), minimal()]
}

pub fn by_id(id: &str) -> Option<ConsoleTheme> {
    all().into_iter().find(|t| t.id == id)
}
