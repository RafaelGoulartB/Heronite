//! The console theme in use (`heronite_data::console_theme`), read while
//! drawing: no I/O, no `cx`. The Collection installs it when console mode
//! starts and whenever it changes (Settings → Console mode, ☰ → Customize);
//! [`clear`] drops it when console mode ends. Until then everything reads the
//! built-in Classic theme, so the desktop and the in-game overlay never load a
//! file or allocate anything for it.
//!
//! The controller's options (how its buttons are drawn, which one confirms)
//! are not the theme's: they apply everywhere, the overlay too. They are
//! installed at start from the mirror in `ui-state.json` ([`set_controls`])
//! and follow every change; [`set_controller`] says which controller is in
//! use, for the automatic choices.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use gpui_kit::{rgb, Rgba};
use heronite_data::console_theme::{
    parse_hex, Clock, ConfirmButton, ConsoleFont, ConsoleTheme, ControllerPrefs, Corners, Glyphs, Motion, TileShape, TileSize,
};

/// The theme in use with its colours resolved (once, when it is installed).
#[derive(Clone)]
struct Installed {
    theme: Arc<ConsoleTheme>,
    palette: Palette,
}

impl Installed {
    fn new(theme: ConsoleTheme) -> Self {
        let palette = Palette::of(&theme);
        Self { theme: Arc::new(theme), palette }
    }
}

static THEME: RwLock<Option<Installed>> = RwLock::new(None);
static EPOCH: AtomicU64 = AtomicU64::new(0);
/// Colour of the focused game's cover, for the `game` focus colour.
static GAME_ACCENT: RwLock<Option<(u8, u8, u8)>> = RwLock::new(None);
/// The connected controller's family, for `Glyphs::Auto`.
static CONTROLLER: RwLock<Option<Glyphs>> = RwLock::new(None);
/// The controller's options (`ConsolePrefs::controller`).
static CONTROLS: RwLock<ControllerPrefs> = RwLock::new(ControllerPrefs { glyphs: Glyphs::Auto, confirm: ConfirmButton::Auto });

fn classic() -> &'static Installed {
    static C: OnceLock<Installed> = OnceLock::new();
    C.get_or_init(|| Installed::new(ConsoleTheme::default()))
}

/// The theme in use (Classic when none was set).
pub fn theme() -> Arc<ConsoleTheme> {
    THEME.read().unwrap_or_else(|e| e.into_inner()).as_ref().unwrap_or_else(|| classic()).theme.clone()
}

/// Installs `theme` (normalized), resolving its colours.
pub fn set(theme: ConsoleTheme) {
    *THEME.write().unwrap_or_else(|e| e.into_inner()) = Some(Installed::new(theme.normalized()));
    EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// Back to Classic (console mode ended): frees the theme.
pub fn clear() {
    *THEME.write().unwrap_or_else(|e| e.into_inner()) = None;
    *GAME_ACCENT.write().unwrap_or_else(|e| e.into_inner()) = None;
    EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// Changes every time the theme does (part of animation and cache ids).
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::Relaxed)
}

/// Sets the focused game's colour.
pub fn set_game_accent(c: Option<(u8, u8, u8)>) {
    *GAME_ACCENT.write().unwrap_or_else(|e| e.into_inner()) = c;
}

/// Whether the focus colour follows the focused game (worth sampling its cover).
pub fn wants_game_accent() -> bool {
    palette().game_focus
}

/// The family of a controller from its name, for `Glyphs::Auto`: Xbox
/// unless the name says otherwise (most others copy its layout).
pub fn glyphs_for_controller(name: &str) -> Glyphs {
    let n = name.to_ascii_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| n.contains(k));
    if has(&["xbox", "microsoft", "x-box"]) {
        Glyphs::Xbox
    } else if has(&["dualsense", "dualshock", "playstation", "ps3", "ps4", "ps5", "sony"]) || n.trim() == "wireless controller" {
        // "Wireless Controller" alone is a DualShock 4 over Bluetooth.
        Glyphs::PlayStation
    } else if has(&["nintendo", "pro controller", "joy-con", "joycon", "switch"]) {
        Glyphs::Nintendo
    } else if has(&["steam deck", "valve", "steam controller"]) {
        Glyphs::Deck
    } else {
        Glyphs::Xbox
    }
}

/// The controller in use (`None` when the last one goes away).
pub fn set_controller(name: Option<&str>) {
    *CONTROLLER.write().unwrap_or_else(|e| e.into_inner()) = name.map(glyphs_for_controller);
}

/// The overlay's list of controllers (evdev on Linux): the first one is the
/// one in use. XInput's (Windows) names no family and misses PlayStation and
/// Nintendo controllers, which console mode finds through SDL: not used.
/// An empty list keeps the last one.
pub fn controllers_found(list: &[heronite_overlay::input::Controller]) {
    if cfg!(windows) {
        return;
    }
    if let Some(first) = list.first() {
        set_controller(Some(&first.name));
    }
}

/// The family of the controller in use, when one is known.
pub fn controller_family() -> Option<Glyphs> {
    *CONTROLLER.read().unwrap_or_else(|e| e.into_inner())
}

/// Installs the controller's options; whether they changed.
pub fn set_controls(c: ControllerPrefs) -> bool {
    let mut cur = CONTROLS.write().unwrap_or_else(|e| e.into_inner());
    let changed = *cur != c;
    *cur = c;
    changed
}

/// The controller's options in use.
pub fn controls() -> ControllerPrefs {
    *CONTROLS.read().unwrap_or_else(|e| e.into_inner())
}

/// How to draw the buttons, `Auto` resolved (Xbox when no controller is known).
pub fn glyphs() -> Glyphs {
    resolve(controls().glyphs, controller_family())
}

/// [`glyphs`] for a drawn controller: the keyboard's becomes the controller's.
pub fn pad_glyphs() -> Glyphs {
    match glyphs() {
        Glyphs::Keyboard => controller_family().unwrap_or(Glyphs::Xbox),
        g => g,
    }
}

/// The east face button confirms and the south one goes back.
pub fn confirm_east() -> bool {
    confirms_east(controls().confirm, pad_glyphs())
}

fn resolve(glyphs: Glyphs, family: Option<Glyphs>) -> Glyphs {
    match glyphs {
        Glyphs::Auto => family.unwrap_or(Glyphs::Xbox),
        g => g,
    }
}

/// `Auto` confirms the Nintendo way (its A is east) with Nintendo buttons.
fn confirms_east(confirm: ConfirmButton, pad: Glyphs) -> bool {
    match confirm {
        ConfirmButton::East => true,
        ConfirmButton::South => false,
        ConfirmButton::Auto => pad == Glyphs::Nintendo,
    }
}

/// The named focus colours.
pub const ACCENTS: [(&str, u32); 7] = [
    ("white", 0xeef2f7),
    ("game", 0xeef2f7),
    ("ice", 0xa5dceb),
    ("green", 0x5ee3a1),
    ("amber", 0xf7c46c),
    ("pink", 0xf49ac1),
    ("violet", 0xb9a4fb),
];

/// The console's dark ink: the text of a light console theme, and what the
/// surfaces' faint fills, tints and inked logos use there.
pub const INK: u32 = 0x15191f;
/// [`INK`] as bytes (for the image pipeline).
pub const INK_RGB: [u8; 3] = [(INK >> 16) as u8, (INK >> 8) as u8, INK as u8];

/// What a light theme uses for the colours it leaves empty (a dark theme has
/// the built-in colours of `theme::color`). The focus is dark and its text
/// white for the `white` and `game` accents, which would not show on it.
struct LightDefaults {
    text: u32,
    muted: u32,
    panel: u32,
    focus: u32,
    focus_text: u32,
}

const LIGHT: LightDefaults = LightDefaults { text: INK, muted: 0x58606e, panel: 0xffffff, focus: 0x1f232a, focus_text: 0xffffff };

/// A theme's colours, resolved when it is installed so drawing only copies
/// them (`None`: the built-in colour of `theme::color` stays).
#[derive(Clone, Copy)]
pub struct Palette {
    /// The theme's background is a light colour: the console is drawn light
    /// (dark text, white panels, the art washed out).
    pub light: bool,
    background: Option<Rgba>,
    panel: Option<Rgba>,
    text: Option<Rgba>,
    muted: Option<Rgba>,
    on: Option<Rgba>,
    focus_text: Option<Rgba>,
    /// The focus colour, or what it is until a game colour exists.
    focus: Rgba,
    /// The focus colour follows the focused game.
    game_focus: bool,
    motion: Motion,
}

fn hex(s: &str) -> Option<Rgba> {
    parse_hex(s).map(|(r, g, b)| rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)))
}

impl Palette {
    fn of(t: &ConsoleTheme) -> Self {
        let c = &t.colors;
        let light = c.is_light();
        let a = t.interface.accent.as_str();
        let white_focus = light && matches!(a, "white" | "game");
        let or_light = |own: &str, default: u32| hex(own).or_else(|| light.then(|| rgb(default)));
        let focus = hex(a).unwrap_or_else(|| {
            rgb(if white_focus { LIGHT.focus } else { ACCENTS.iter().find(|(n, _)| *n == a).map_or(0xeef2f7, |(_, c)| *c) })
        });
        Self {
            light,
            background: hex(&c.background),
            panel: or_light(&c.panel, LIGHT.panel),
            text: or_light(&c.text, LIGHT.text),
            muted: or_light(&c.muted, LIGHT.muted),
            on: hex(&c.on),
            focus_text: hex(&c.focus_text).or_else(|| white_focus.then(|| rgb(LIGHT.focus_text))),
            focus,
            game_focus: a == "game",
            motion: t.interface.motion,
        }
    }
}

/// The resolved colours of the theme in use.
pub fn palette() -> Palette {
    THEME.read().unwrap_or_else(|e| e.into_inner()).as_ref().unwrap_or_else(|| classic()).palette
}

/// The theme's background is a light colour: the console is drawn light
/// (dark text, white panels, the art washed out).
pub fn light() -> bool {
    palette().light
}

/// The focus colour (the light pill, the ring around covers).
pub fn focus() -> Rgba {
    let p = palette();
    if p.game_focus {
        if let Some((r, g, b)) = *GAME_ACCENT.read().unwrap_or_else(|e| e.into_inner()) {
            // Light enough for the dark focus text (dark enough for white
            // text on a light theme).
            let (k, to) = if p.light { (0.55, 0.) } else { (0.45, 255.) };
            let lift = |c: u8| (f32::from(c) * k + to * (1. - k)) as u32;
            return rgb(lift(r) << 16 | lift(g) << 8 | lift(b));
        }
    }
    p.focus
}

/// Text on the focus colour (white on a light theme's dark focus).
pub fn focus_text() -> Option<Rgba> {
    palette().focus_text
}
pub fn background() -> Option<Rgba> {
    palette().background
}
pub fn panel() -> Option<Rgba> {
    palette().panel
}
pub fn text() -> Option<Rgba> {
    palette().text
}
pub fn muted() -> Option<Rgba> {
    palette().muted
}
pub fn on() -> Option<Rgba> {
    palette().on
}

/// Heading and body font families.
pub fn fonts() -> (&'static str, &'static str) {
    match theme().interface.font {
        ConsoleFont::Heronite => ("Rubik", "Cabin"),
        ConsoleFont::Cabin => ("Cabin", "Cabin"),
        ConsoleFont::Rubik => ("Rubik", "Rubik"),
        ConsoleFont::System => (".SystemUIFont", ".SystemUIFont"),
        ConsoleFont::Mono => (crate::theme::FONT_MONO, crate::theme::FONT_MONO),
    }
}

/// Interface scale multiplier (1.0 = 100%).
pub fn scale() -> f32 {
    f32::from(theme().interface.scale) / 100.
}

/// Corner radius of covers, design px.
pub fn tile_radius() -> f32 {
    match theme().tiles.corners {
        Corners::Square => 3.,
        Corners::Soft => 7.,
        Corners::Round => 10.,
    }
}

/// Size of a cover tile, design px: (width, height) for `shape`.
pub fn tile_size(shape: TileShape, size: TileSize, cover_aspect: f32) -> (f32, f32) {
    let w = match size {
        TileSize::Small => 120.,
        TileSize::Medium => 150.,
        TileSize::Large => 180.,
        TileSize::Huge => 215.,
    };
    match shape {
        TileShape::Portrait => (w, w * cover_aspect),
        TileShape::Square => (w, w),
        TileShape::Landscape => (w * 1.75, w * 1.75 * 9. / 16.),
    }
}

/// The theme's motion setting (the app-wide one and `HERONITE_NO_ANIMATIONS` win).
pub fn motion() -> Motion {
    palette().motion
}

/// "21:04", "9:04 PM", or nothing; with the date before it when asked.
pub fn clock_text(now: chrono::DateTime<chrono::Local>) -> Option<String> {
    let t = theme();
    let time = match t.interface.clock {
        Clock::Off => return None,
        Clock::H24 => now.format("%H:%M").to_string(),
        Clock::H12 => now.format("%-I:%M %p").to_string(),
    };
    Some(if t.interface.date { format!("{}  {time}", now.format("%d/%m")) } else { time })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_families() {
        assert_eq!(glyphs_for_controller("Sony Interactive Entertainment DualSense Wireless Controller"), Glyphs::PlayStation);
        assert_eq!(glyphs_for_controller("Nintendo Switch Pro Controller"), Glyphs::Nintendo);
        assert_eq!(glyphs_for_controller("Steam Deck"), Glyphs::Deck);
        assert_eq!(glyphs_for_controller("Xbox Wireless Controller"), Glyphs::Xbox);
        assert_eq!(glyphs_for_controller("Wireless Controller"), Glyphs::PlayStation, "a DualShock 4 over Bluetooth");
        assert_eq!(glyphs_for_controller("Sony Computer Entertainment Wireless Controller"), Glyphs::PlayStation);
        assert_eq!(glyphs_for_controller("GameSir G7 SE"), Glyphs::Xbox);
        assert_eq!(glyphs_for_controller("8BitDo Ultimate Wireless Controller"), Glyphs::Xbox);
        assert_eq!(glyphs_for_controller("Microsoft X-Box 360 pad"), Glyphs::Xbox);
        assert_eq!(glyphs_for_controller("Nintendo Co., Ltd. Pro Controller"), Glyphs::Nintendo);
    }

    #[test]
    fn auto_follows_the_controller_and_confirms_with_nintendo_s_a() {
        use heronite_data::console_theme::ConfirmButton;
        assert_eq!(resolve(Glyphs::Auto, None), Glyphs::Xbox);
        assert_eq!(resolve(Glyphs::Auto, Some(Glyphs::PlayStation)), Glyphs::PlayStation);
        assert_eq!(resolve(Glyphs::Nintendo, Some(Glyphs::PlayStation)), Glyphs::Nintendo, "chosen wins");
        assert!(confirms_east(ConfirmButton::Auto, Glyphs::Nintendo));
        assert!(!confirms_east(ConfirmButton::Auto, Glyphs::PlayStation));
        assert!(confirms_east(ConfirmButton::East, Glyphs::Xbox));
        assert!(!confirms_east(ConfirmButton::South, Glyphs::Nintendo));
    }

    #[test]
    fn light_themes_fill_in_the_colours_they_leave_empty() {
        let mut t = ConsoleTheme::default();
        t.colors.background = "#f2f4f7".into();
        let p = Palette::of(&t);
        assert!(p.light);
        assert_eq!(p.text, Some(rgb(INK)));
        assert_eq!(p.panel, Some(rgb(0xffffff)));
        assert_eq!(p.focus, rgb(LIGHT.focus));
        assert_eq!(p.focus_text, Some(rgb(LIGHT.focus_text)));
        // A named accent keeps its own colour and its text.
        t.interface.accent = "green".into();
        let p = Palette::of(&t);
        assert_eq!((p.focus, p.focus_text), (rgb(0x5ee3a1), None));
        // A dark theme leaves the built-in colours alone.
        let p = Palette::of(&ConsoleTheme::default());
        assert!(!p.light && p.text.is_none() && p.panel.is_none() && p.focus_text.is_none());
    }

    #[test]
    fn tile_sizes_keep_their_shape() {
        let (w, h) = tile_size(TileShape::Landscape, TileSize::Medium, 1.5);
        assert!((w / h - 16. / 9.).abs() < 0.01);
        assert_eq!(tile_size(TileShape::Square, TileSize::Small, 1.5), (120., 120.));
    }
}
