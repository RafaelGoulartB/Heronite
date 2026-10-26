//! Console mode themes: everything console mode's look and layout can change,
//! as one JSON document that can be saved, exported, imported and shared
//! (a theme file is self-contained and holds no machine-specific state).
//!
//! - [`ConsoleTheme`]: the theme. Every field has a default (the Classic theme,
//!   see [`presets::classic`]); unknown keys are
//!   kept in `extra` so a theme made by a newer app survives a round trip;
//!   an unknown enum value reads as that field's default.
//! - [`ConsolePrefs`] (`AppPaths::console_prefs()`, `console.json`): the theme
//!   in use on this machine plus the options that are not part of a theme
//!   (what to do when a controller connects, how the app opens, idle).
//! - [`presets`]: the built-in themes; [`store`]: saved themes, export and
//!   import.
//!
//! Nothing here is read unless console mode is on or its Settings section is
//! shown, so the desktop pays nothing for it.

pub mod presets;
pub mod store;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::json_store;
use crate::paths::AppPaths;

/// Version of the theme format this app writes. A theme with a higher
/// `format` is refused on import (it may rely on things this app lacks).
pub const FORMAT: u32 = 1;
/// At most this many tabs (LB / RB).
pub const MAX_TABS: usize = 4;
/// Interface scale, percent.
pub const SCALE_RANGE: (u16, u16) = (80, 140);
pub const DIM_MAX: u8 = 90;
pub const BLUR_MAX: u8 = 40;

/// The named focus colours (`interface.accent`); anything else is `#rrggbb`.
pub const ACCENT_NAMES: [&str; 7] = ["white", "game", "ice", "green", "amber", "pink", "violet"];

/// A string-backed enum: written as its string, and read leniently (an
/// unknown or missing value gives the default) so themes from other app
/// versions always load.
macro_rules! str_enum {
    ($(#[$m:meta])* pub enum $name:ident { $($(#[$vm:meta])* $var:ident = $s:literal),+ $(,)? } default $def:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vm])* $var),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub fn as_str(self) -> &'static str {
                match self { $($name::$var => $s),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($name::$var),)+ _ => None }
            }
        }
        impl Default for $name {
            fn default() -> Self { $name::$def }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let v = Value::deserialize(d)?;
                Ok(v.as_str().and_then(Self::parse).unwrap_or_default())
            }
        }
    };
}

str_enum! {
    /// How a tab shows its games.
    pub enum Layout {
        /// The focused game large, shelves of covers below.
        Shelves = "shelves",
        /// One game at a time over its art: a row of icons, actions, cards.
        Spotlight = "spotlight",
        /// Full-screen art that drifts, one row of wide covers, little text.
        Cinema = "cinema",
        /// Tiles: the current game, recent ones, week, downloads, backlog pick, journal.
        Dashboard = "dashboard",
        /// Every game in a cover grid with filters (optionally a game panel).
        Grid = "grid",
        /// A wheel of names, the art and facts beside it.
        Wheel = "wheel",
        /// One large cover in the middle, the neighbours smaller.
        Carousel = "carousel",
        /// A row of tiles over the art, the newest one wide, groups below.
        /// Shown as "Series X" (the id stays `xbox` in theme files).
        Xbox = "xbox",
    } default Shelves
}

str_enum! {
    pub enum StartOn { First = "first", Last = "last" } default First
}
str_enum! {
    /// What A does on a cover (X does the other).
    pub enum AButton { Details = "details", Play = "play" } default Details
}
str_enum! {
    pub enum HeroTitle { Text = "text", Logo = "logo" } default Text
}
str_enum! {
    /// How the hero shows the game's facts: one line of text, or chips (the
    /// store, the year, the score) with the genres.
    pub enum FactStyle { Line = "line", Chips = "chips" } default Line
}
str_enum! {
    /// Where the hero sits: at the left, or centred at the top (Cinema).
    pub enum HeroAlign { Left = "left", Center = "center" } default Left
}
str_enum! {
    pub enum Size3 { Small = "small", Medium = "medium", Large = "large" } default Medium
}
str_enum! {
    pub enum TileShape { Portrait = "portrait", Landscape = "landscape", Square = "square" } default Portrait
}
str_enum! {
    pub enum TileSize { Small = "s", Medium = "m", Large = "l", Huge = "xl" } default Medium
}
str_enum! {
    /// The name under the covers: on the page, or (`bar`) in a strip joined
    /// to the cover's bottom edge.
    pub enum TileTitles { Never = "never", Focused = "focused", Always = "always", Bar = "bar" } default Never
}
str_enum! {
    /// Badges on the covers: none, only "Playing" / "Preparing", or also
    /// favourite, downloading and "New" (added in the last two weeks).
    pub enum Badges { Off = "off", Running = "running", All = "all" } default Running
}
str_enum! {
    pub enum Corners { Square = "square", Soft = "soft", Round = "round" } default Round
}
str_enum! {
    /// How the focused cover stands out.
    pub enum FocusStyle { Ring = "ring", Glow = "glow", Lift = "lift", Underline = "underline" } default Ring
}
str_enum! {
    pub enum BackgroundSource {
        /// The focused game's wide art.
        Art = "art",
        /// Its cover, heavily blurred.
        Cover = "cover",
        /// Its screenshots.
        Screenshots = "screenshots",
        /// A gradient of the colour of its cover.
        GameColour = "game-colour",
        /// The background colour alone.
        Solid = "solid",
        /// The theme's own picture (`background.image`).
        Image = "image",
    } default Art
}
str_enum! {
    pub enum BackgroundMotion { Still = "still", Drift = "drift" } default Still
}
str_enum! {
    pub enum ConsoleFont { Heronite = "heronite", Cabin = "cabin", Rubik = "rubik", System = "system", Mono = "mono" } default Heronite
}
str_enum! {
    pub enum Clock { H24 = "24h", H12 = "12h", Off = "off" } default H24
}
str_enum! {
    /// Where the LB / RB tabs show.
    pub enum TabBar {
        /// Their names along the top bar.
        Top = "top",
        /// An icon and a name each, in a bar along the bottom.
        Bottom = "bottom",
        /// Only the current tab's name, centred in the top bar.
        Title = "title",
        /// Their names alone, larger, without the LB / RB buttons.
        Names = "names",
    } default Top
}
str_enum! {
    /// Shelf titles as written, or in spaced capitals.
    pub enum Headings { Normal = "normal", Spaced = "spaced" } default Normal
}
str_enum! {
    pub enum Motion { Full = "full", Reduced = "reduced", Off = "off" } default Full
}
str_enum! {
    /// How the controller buttons are drawn (Auto follows the connected controller).
    pub enum Glyphs { Auto = "auto", Xbox = "xbox", PlayStation = "playstation", Nintendo = "nintendo", Deck = "deck", Keyboard = "keyboard" } default Xbox
}
str_enum! {
    pub enum Hints { Always = "always", Auto = "auto", Off = "off" } default Always
}
str_enum! {
    pub enum GamePage { Classic = "classic", Hero = "hero", Tabs = "tabs", Sheet = "sheet" } default Classic
}
str_enum! {
    /// `rail`: a column of icons always on screen that opens into the sidebar.
    pub enum MenuStyle { Sidebar = "sidebar", Grid = "grid", Dock = "dock", Rail = "rail" } default Sidebar
}
str_enum! {
    pub enum Side { Left = "left", Right = "right" } default Left
}
str_enum! {
    pub enum GridPanel { Title = "title", Right = "right" } default Title
}
str_enum! {
    /// Spotlight's buttons: Play, status, favourite and more, or only a wide
    /// Play and more.
    pub enum SpotActions { Full = "full", Play = "play" } default Full
}
str_enum! {
    pub enum WheelItems { Name = "name", Logo = "logo" } default Name
}
str_enum! {
    pub enum AutoConsole { Off = "off", Ask = "ask", On = "on" } default Off
}
str_enum! {
    pub enum OpenIn { Last = "last", Console = "console", Desktop = "desktop" } default Last
}
str_enum! {
    pub enum Idle { Off = "off", Attract = "attract", Dim = "dim" } default Off
}

/// One LB / RB tab: a layout showing a set of games.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TabSpec {
    pub name: String,
    pub layout: Layout,
    /// Which games: `all`, `installed`, `not-installed`, `favourites`, `pc`,
    /// `emulated`, `status:<id>`, `category:<name>`, `store:<label>`,
    /// `platform:<name>`. Unknown values show every game.
    pub source: String,
}

impl Default for TabSpec {
    fn default() -> Self {
        Self { name: String::new(), layout: Layout::Shelves, source: "all".into() }
    }
}

impl TabSpec {
    pub fn new(name: &str, layout: Layout, source: &str) -> Self {
        Self { name: name.into(), layout, source: source.into() }
    }
}

/// One shelf of the Shelves / Cinema / Spotlight / Xbox layouts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShelfSpec {
    /// `recent`, `favourites`, `installed`, `added`, `backlog-short`,
    /// `random`, `forgotten`, `statuses` (one shelf per status of the
    /// library), `status:<id>`, `store:<label>`,
    /// `platform:<name>`, `genre:<name>`, `category:<name>`.
    pub kind: String,
    pub on: bool,
    /// Shown instead of the kind's own name when not empty.
    pub title: String,
}

impl Default for ShelfSpec {
    fn default() -> Self {
        Self { kind: String::new(), on: true, title: String::new() }
    }
}

impl ShelfSpec {
    pub fn new(kind: &str, on: bool) -> Self {
        Self { kind: kind.into(), on, title: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NavOptions {
    pub start_on: StartOn,
    pub a_button: AButton,
    /// Moving past the last item goes back to the first.
    pub wrap: bool,
}

impl Default for NavOptions {
    fn default() -> Self {
        Self { start_on: StartOn::First, a_button: AButton::Details, wrap: false }
    }
}

/// The focused game's name and facts (Shelves' top, Spotlight, Cinema...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HeroOptions {
    pub title: HeroTitle,
    /// Store, installed, playtime and status.
    pub facts: bool,
    pub overview: bool,
    pub size: Size3,
    pub fact_style: FactStyle,
    pub align: HeroAlign,
    /// A panel of numbers at the right edge: time played, last played,
    /// size on disk, score, genre.
    pub stats: bool,
}

impl Default for HeroOptions {
    fn default() -> Self {
        Self {
            title: HeroTitle::Text,
            facts: true,
            overview: true,
            size: Size3::Medium,
            fact_style: FactStyle::Line,
            align: HeroAlign::Left,
            stats: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TileOptions {
    pub shape: TileShape,
    pub size: TileSize,
    pub titles: TileTitles,
    pub badges: Badges,
    pub corners: Corners,
    pub focus: FocusStyle,
    pub grey_uninstalled: bool,
    /// The store's name in a strip above the cover.
    pub header: bool,
}

impl Default for TileOptions {
    fn default() -> Self {
        Self {
            shape: TileShape::Portrait,
            size: TileSize::Medium,
            titles: TileTitles::Never,
            badges: Badges::Running,
            corners: Corners::Round,
            focus: FocusStyle::Ring,
            grey_uninstalled: false,
            header: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BackgroundOptions {
    pub source: BackgroundSource,
    /// Extra darkening, 0..=[`DIM_MAX`] percent.
    pub dim: u8,
    /// Blur of the art, 0..=[`BLUR_MAX`] px.
    pub blur: u8,
    pub motion: BackgroundMotion,
    /// [`BackgroundSource::Image`]: a file path or an http(s) URL.
    pub image: String,
}

impl Default for BackgroundOptions {
    fn default() -> Self {
        Self { source: BackgroundSource::Art, dim: 0, blur: 0, motion: BackgroundMotion::Still, image: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InterfaceOptions {
    /// Focus colour: `white`, `game` (from the focused cover), `ice`,
    /// `green`, `amber`, `pink`, `violet`, or `#rrggbb`.
    pub accent: String,
    pub font: ConsoleFont,
    /// Size of everything, percent ([`SCALE_RANGE`]): bigger reads better
    /// from a sofa.
    pub scale: u16,
    pub clock: Clock,
    pub date: bool,
    /// Text in the top-left corner; empty shows nothing.
    pub label: String,
    /// Controller, downloads and battery icons in the top bar.
    pub status_icons: bool,
    pub motion: Motion,
    pub tab_bar: TabBar,
    pub headings: Headings,
}

impl Default for InterfaceOptions {
    fn default() -> Self {
        Self {
            accent: "white".into(),
            font: ConsoleFont::Heronite,
            scale: 100,
            clock: Clock::H24,
            date: false,
            label: String::new(),
            status_icons: true,
            motion: Motion::Full,
            tab_bar: TabBar::Top,
            headings: Headings::Normal,
        }
    }
}

/// Colours a theme can replace (`#rrggbb`; empty keeps the built-in one). A
/// light `background` makes the whole console light: the built-in text and
/// surfaces turn dark and the art is washed out instead of darkened.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColourOptions {
    pub background: String,
    /// Menus, sheets, the hint bar.
    pub panel: String,
    pub text: String,
    pub muted: String,
    /// Text on the focus colour.
    pub focus_text: String,
    /// "On" switches.
    pub on: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ControlOptions {
    pub glyphs: Glyphs,
    pub hints: Hints,
    /// Confirm with the east button and go back with the south one.
    pub swap_ab: bool,
    pub sounds: bool,
}

impl Default for ControlOptions {
    fn default() -> Self {
        Self { glyphs: Glyphs::Xbox, hints: Hints::Always, swap_ab: false, sounds: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuItemSpec {
    /// `collection`, `downloads`, `wine-manager`, `stores`, `emulators`,
    /// `statistics`, `separator`, `add-game`, `customize`, `settings`,
    /// `desktop`, `power`.
    pub item: String,
    pub on: bool,
}

impl MenuItemSpec {
    pub fn new(item: &str) -> Self {
        Self { item: item.into(), on: true }
    }
}

impl Default for MenuItemSpec {
    fn default() -> Self {
        Self { item: String::new(), on: true }
    }
}

/// Menu items that cannot be hidden: the way back to the settings and to
/// desktop mode.
pub const MENU_LOCKED: [&str; 2] = ["settings", "desktop"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuOptions {
    pub style: MenuStyle,
    pub side: Side,
    pub items: Vec<MenuItemSpec>,
}

impl Default for MenuOptions {
    fn default() -> Self {
        Self { style: MenuStyle::Sidebar, side: Side::Left, items: presets::default_menu() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DashboardWidgets {
    pub week: bool,
    pub downloads: bool,
    pub random: bool,
    pub journal: bool,
    pub shortcuts: bool,
}

impl Default for DashboardWidgets {
    fn default() -> Self {
        Self { week: true, downloads: true, random: true, journal: true, shortcuts: true }
    }
}

/// Options of one layout each.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayoutOptions {
    pub grid_panel: GridPanel,
    /// Grid: LT / RT jump to the next / previous letter.
    pub jump_letters: bool,
    pub wheel_items: WheelItems,
    /// Spotlight: playtime, time to beat and journal cards.
    pub spotlight_cards: bool,
    pub spotlight_actions: SpotActions,
    pub carousel_reflect: bool,
    /// Xbox: the shelves under the row of tiles.
    pub xbox_groups: bool,
    pub dashboard: DashboardWidgets,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            grid_panel: GridPanel::Title,
            jump_letters: false,
            wheel_items: WheelItems::Name,
            spotlight_cards: true,
            spotlight_actions: SpotActions::Full,
            carousel_reflect: false,
            xbox_groups: true,
            dashboard: DashboardWidgets::default(),
        }
    }
}

/// A console mode theme. JSON keys are camelCase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConsoleTheme {
    /// [`FORMAT`] of the app that wrote it.
    pub format: u32,
    /// Stable id of a saved or built-in theme (`[a-z0-9-]`); empty for an
    /// unsaved one.
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    /// The theme's own version, free text ("1.2").
    pub version: String,
    pub tabs: Vec<TabSpec>,
    pub shelves: Vec<ShelfSpec>,
    /// Covers per shelf; 0 = all.
    pub shelf_limit: u32,
    /// The shelves without the focus step back.
    pub dim_others: bool,
    pub nav: NavOptions,
    pub hero: HeroOptions,
    pub tiles: TileOptions,
    pub background: BackgroundOptions,
    pub interface: InterfaceOptions,
    pub colors: ColourOptions,
    pub controls: ControlOptions,
    pub game_page: GamePage,
    pub menu: MenuOptions,
    pub layouts: LayoutOptions,
    /// Keys this app does not know, kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for ConsoleTheme {
    fn default() -> Self {
        presets::classic()
    }
}

/// `#rrggbb`, `rrggbb` or `#rgb` to bytes.
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    crate::appearance::parse_hex(s)
}

impl ColourOptions {
    /// The background is a light colour (relative luminance over 0.5): the
    /// console is drawn light.
    pub fn is_light(&self) -> bool {
        parse_hex(&self.background).is_some_and(|c| crate::appearance::luminance(c) > 0.5)
    }
}

/// Lower-case `[a-z0-9-]` from a name, for ids and file names.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in crate::fuzzy::fold(name).chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "theme".into()
    } else {
        out.chars().take(48).collect()
    }
}

impl ConsoleTheme {
    /// Clamps numbers and repairs lists so any theme renders: 1..=MAX_TABS
    /// tabs, no empty names, the locked menu items present.
    pub fn normalized(mut self) -> Self {
        self.format = FORMAT;
        // The id names the theme's file: nothing but `[a-z0-9-]`.
        if !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            self.id = slug(&self.id);
        }
        if self.tabs.is_empty() {
            self.tabs = presets::classic().tabs;
        }
        self.tabs.truncate(MAX_TABS);
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if t.name.trim().is_empty() {
                t.name = format!("{}", i + 1);
            }
            if t.source.trim().is_empty() {
                t.source = "all".into();
            }
        }
        let mut kinds = std::collections::HashSet::new();
        self.shelves.retain(|s| !s.kind.trim().is_empty() && kinds.insert(s.kind.clone()));
        self.background.dim = self.background.dim.min(DIM_MAX);
        self.background.blur = self.background.blur.min(BLUR_MAX);
        self.interface.scale = self.interface.scale.clamp(SCALE_RANGE.0, SCALE_RANGE.1);
        let accent = self.interface.accent.trim();
        if !ACCENT_NAMES.contains(&accent) && parse_hex(accent).is_none() {
            self.interface.accent = "white".into();
        }
        self.menu.items.retain(|m| !m.item.trim().is_empty());
        for locked in MENU_LOCKED {
            match self.menu.items.iter_mut().find(|m| m.item == locked) {
                Some(m) => m.on = true,
                None => self.menu.items.push(MenuItemSpec::new(locked)),
            }
        }
        self
    }

    /// The layouts its tabs use.
    pub fn uses(&self, layout: Layout) -> bool {
        self.tabs.iter().any(|t| t.layout == layout)
    }

    /// Reads a theme from JSON (an export, a saved theme): refused when its
    /// `format` is newer than this app's; a value of the wrong type reads as
    /// that field's default; normalized.
    pub fn from_json(text: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(text)?;
        if !value.is_object() {
            anyhow::bail!("not a console theme");
        }
        let format = value.get("format").and_then(Value::as_u64).unwrap_or(1);
        if format > u64::from(FORMAT) {
            anyhow::bail!("this theme needs a newer version of Heronite (theme format {format}, this app reads {FORMAT})");
        }
        lenient::<ConsoleTheme>(value).map(Self::normalized).ok_or_else(|| anyhow::anyhow!("not a console theme"))
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// `console.json`: the theme in use and this machine's console options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConsolePrefs {
    pub theme: ConsoleTheme,
    /// The preset or saved theme `theme` started from (shows "edited" when
    /// they differ).
    pub based_on: String,
    pub auto_console: AutoConsole,
    pub open_in: OpenIn,
    pub idle: Idle,
    /// The tab shown last (`nav.startOn: last`).
    pub last_tab: usize,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for ConsolePrefs {
    fn default() -> Self {
        Self {
            theme: presets::default_theme(),
            based_on: presets::DEFAULT.into(),
            auto_console: AutoConsole::Off,
            open_in: OpenIn::Last,
            idle: Idle::Off,
            last_tab: 0,
            extra: Map::new(),
        }
    }
}

impl ConsolePrefs {
    /// A missing or unreadable file gives the defaults (the Minimal theme).
    pub fn load(paths: &AppPaths) -> Self {
        let value = json_store::read_value(&paths.console_prefs()).unwrap_or(Value::Null);
        if !value.is_object() {
            return Self::default();
        }
        match lenient::<ConsolePrefs>(value) {
            Some(mut p) => {
                p.theme = p.theme.normalized();
                p
            }
            None => {
                log::warn!("console.json is not readable; using the defaults");
                Self::default()
            }
        }
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        json_store::write_indented(&paths.console_prefs(), self, b"  ")
    }
}

/// `T` from `value`, leaving out what does not read: a value of the wrong
/// type (`"shelfLimit": "x"`) gives that field's default instead of losing
/// the whole theme. Each key is tried in turn; an object that does not read
/// is gone through key by key.
fn lenient<T: DeserializeOwned>(value: Value) -> Option<T> {
    if let Ok(t) = serde_json::from_value(value.clone()) {
        return Some(t);
    }
    let Value::Object(map) = value else { return None };
    let mut root = Map::new();
    keep_readable::<T>(&mut root, &[], map);
    serde_json::from_value(Value::Object(root)).ok()
}

fn keep_readable<T: DeserializeOwned>(root: &mut Map<String, Value>, path: &[String], map: Map<String, Value>) {
    let reads = |root: &Map<String, Value>| serde_json::from_value::<T>(Value::Object(root.clone())).is_ok();
    for (key, value) in map {
        let mut at = path.to_vec();
        at.push(key);
        set_at(root, &at, value.clone());
        if reads(root) {
            continue;
        }
        if let Value::Object(inner) = value {
            set_at(root, &at, Value::Object(Map::new()));
            if reads(root) {
                keep_readable::<T>(root, &at, inner);
                continue;
            }
        }
        log::warn!("console theme: {} is not readable, using the default", at.join("."));
        remove_at(root, &at);
    }
}

fn set_at(root: &mut Map<String, Value>, path: &[String], value: Value) {
    let (last, parents) = path.split_last().expect("a path has a key");
    let mut map = root;
    for p in parents {
        let next = map.entry(p.clone()).or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new());
        }
        map = next.as_object_mut().expect("made an object above");
    }
    map.insert(last.clone(), value);
}

fn remove_at(root: &mut Map<String, Value>, path: &[String]) {
    let (last, parents) = path.split_last().expect("a path has a key");
    let mut map = root;
    for p in parents {
        let Some(next) = map.get_mut(p).and_then(Value::as_object_mut) else { return };
        map = next;
    }
    map.remove(last);
}

#[cfg(test)]
mod tests;
