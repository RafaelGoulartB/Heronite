//! The built-in console themes. `classic` is the default; the others start
//! from it and change what makes them different. Their names and descriptions are English keys translated
//! where they are shown (`n_!` marks them for the translation coverage test).

use heronite_i18n::n_;

use super::*;

/// Classic's ☰ menu (Customize included).
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

/// The default: shelves on Home, a cover grid in Library, the sidebar menu.
pub fn classic() -> ConsoleTheme {
    ConsoleTheme {
        format: FORMAT,
        id: "classic".into(),
        name: n_!("Classic").into(),
        author: "Heronite".into(),
        description: n_!("Console mode as it shipped: shelves and a cover grid.").into(),
        version: "1".into(),
        tabs: vec![TabSpec::new(n_!("Home"), Layout::Shelves, "all"), TabSpec::new(n_!("Library"), Layout::Grid, "all")],
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

/// The game's art full screen with its logo large, chips of facts, one row
/// of portrait covers and the tabs along the bottom.
pub fn cinema() -> ConsoleTheme {
    from_classic("cinema", n_!("Cinema"), n_!("The game's art full screen, its logo large, a row of covers and the tabs below."), |t| {
        t.tabs = vec![
            TabSpec::new(n_!("All games"), Layout::Cinema, "all"),
            TabSpec::new(n_!("Installed games"), Layout::Cinema, "installed"),
            TabSpec::new(n_!("Favourites"), Layout::Cinema, "favourites"),
            TabSpec::new(n_!("Library"), Layout::Grid, "all"),
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

/// A console's home screen: a row of covers on top (the focused one
/// grows, its name beside it), the game's art and logo, a wide Play button
/// and cards; the tabs as plain names.
pub fn pulse() -> ConsoleTheme {
    from_classic("pulse", n_!("Pulse"), n_!("A console home: icons on top, the game's art and logo, a wide Play button and cards."), |t| {
        t.tabs = vec![TabSpec::new(n_!("Games"), Layout::Spotlight, "all"), TabSpec::new(n_!("Library"), Layout::Grid, "all")];
        t.hero.title = HeroTitle::Logo;
        t.hero.facts = false;
        t.hero.overview = false;
        t.hero.size = Size3::Large;
        t.tiles.size = TileSize::Large;
        t.tiles.corners = Corners::Soft;
        t.interface.tab_bar = TabBar::Names;
        t.interface.font = ConsoleFont::System;
        t.controls.glyphs = Glyphs::PlayStation;
        t.controls.hints = Hints::Auto;
        t.layouts.spotlight_actions = SpotActions::Play;
        t.layouts.grid_panel = GridPanel::Right;
    })
}

/// A row of square tiles over the game's art (the newest one wide, groups
/// below), a green focus and the system font.
pub fn series() -> ConsoleTheme {
    from_classic("series", n_!("Series X"), n_!("A row of tiles over the game's art, the newest one wide, groups below."), |t| {
        t.tabs = vec![TabSpec::new(n_!("Home"), Layout::Xbox, "all"), TabSpec::new(n_!("Library"), Layout::Grid, "all")];
        t.tiles.corners = Corners::Soft;
        t.tiles.shape = TileShape::Square;
        t.interface.accent = "green".into();
        t.interface.font = ConsoleFont::System;
        t.layouts.grid_panel = GridPanel::Right;
    })
}

/// Only covers: a carousel over the blurred cover, no hints or badges.
pub fn minimal() -> ConsoleTheme {
    from_classic("minimal", n_!("Minimal"), n_!("Only the covers: a carousel, no hints, no badges."), |t| {
        t.tabs = vec![TabSpec::new(n_!("Games"), Layout::Carousel, "all")];
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
    vec![classic(), minimal(), cinema(), pulse(), series()]
}

pub fn by_id(id: &str) -> Option<ConsoleTheme> {
    all().into_iter().find(|t| t.id == id)
}
