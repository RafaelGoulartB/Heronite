//! The console theme in use (`heronite_data::console_theme`), read while
//! drawing: no I/O, no `cx`. The Collection installs it when console mode
//! starts and whenever it changes (Settings → Console mode, ☰ → Customize);
//! [`clear`] drops it when console mode ends. Until then everything reads the
//! built-in Classic theme, so the desktop and the in-game overlay never load a
//! file or allocate anything for it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use gpui_kit::{rgb, Rgba};
use heronite_data::console_theme::{parse_hex, Clock, ConsoleFont, ConsoleTheme, Corners, Glyphs, Motion, TileShape, TileSize, MAX_TABS};

static THEME: RwLock<Option<Arc<ConsoleTheme>>> = RwLock::new(None);
static EPOCH: AtomicU64 = AtomicU64::new(0);
/// Colour of the focused game's cover, for the `game` focus colour.
static GAME_ACCENT: RwLock<Option<(u8, u8, u8)>> = RwLock::new(None);
/// The connected controller's family, for `Glyphs::Auto`.
static CONTROLLER: RwLock<Option<Glyphs>> = RwLock::new(None);

fn classic() -> &'static Arc<ConsoleTheme> {
    static C: OnceLock<Arc<ConsoleTheme>> = OnceLock::new();
    C.get_or_init(|| Arc::new(ConsoleTheme::default()))
}

/// The theme in use (Classic when none was set).
pub fn theme() -> Arc<ConsoleTheme> {
    THEME.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| classic().clone())
}

/// Installs `theme` (normalized).
pub fn set(theme: ConsoleTheme) {
    *THEME.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(theme.normalized()));
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

/// Sets the focused game's colour; true when it changed.
pub fn set_game_accent(c: Option<(u8, u8, u8)>) -> bool {
    let mut cur = GAME_ACCENT.write().unwrap_or_else(|e| e.into_inner());
    let changed = *cur != c;
    *cur = c;
    changed
}

/// Whether the focus colour follows the focused game (worth sampling its cover).
pub fn wants_game_accent() -> bool {
    theme().interface.accent == "game"
}

/// The family of a controller from its name, for `Glyphs::Auto`.
pub fn glyphs_for_controller(name: &str) -> Glyphs {
    let n = name.to_ascii_lowercase();
    if n.contains("xbox") || n.contains("microsoft") {
        Glyphs::Xbox
    } else if ["dualsense", "dualshock", "playstation", "ps3", "ps4", "ps5", "sony", "wireless controller"].iter().any(|k| n.contains(k)) {
        Glyphs::PlayStation
    } else if ["nintendo", "pro controller", "joy-con", "switch"].iter().any(|k| n.contains(k)) {
        Glyphs::Nintendo
    } else if n.contains("steam deck") || n.contains("valve") {
        Glyphs::Deck
    } else {
        Glyphs::Xbox
    }
}

/// The connected controller's name (`None` when it disconnects).
pub fn set_controller(name: Option<&str>) {
    *CONTROLLER.write().unwrap_or_else(|e| e.into_inner()) = name.map(glyphs_for_controller);
}

/// How to draw the buttons, `Auto` resolved.
pub fn glyphs() -> Glyphs {
    match theme().controls.glyphs {
        Glyphs::Auto => CONTROLLER.read().unwrap_or_else(|e| e.into_inner()).unwrap_or(Glyphs::Xbox),
        g => g,
    }
}

fn hex(s: &str) -> Option<Rgba> {
    parse_hex(s).map(|(r, g, b)| rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)))
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

/// The focus colour (the light pill, the ring around covers).
pub fn focus() -> Rgba {
    let t = theme();
    let a = t.interface.accent.as_str();
    if a == "game" {
        if let Some((r, g, b)) = *GAME_ACCENT.read().unwrap_or_else(|e| e.into_inner()) {
            // Light enough for the dark focus text.
            let lift = |c: u8| (f32::from(c) * 0.45 + 255. * 0.55) as u32;
            return rgb(lift(r) << 16 | lift(g) << 8 | lift(b));
        }
    }
    if let Some(c) = hex(a) {
        return c;
    }
    rgb(ACCENTS.iter().find(|(n, _)| *n == a).map_or(0xeef2f7, |(_, c)| *c))
}

/// Text on the focus colour.
pub fn focus_text() -> Option<Rgba> {
    hex(&theme().colors.focus_text)
}
pub fn background() -> Option<Rgba> {
    hex(&theme().colors.background)
}
pub fn panel() -> Option<Rgba> {
    hex(&theme().colors.panel)
}
pub fn text() -> Option<Rgba> {
    hex(&theme().colors.text)
}
pub fn muted() -> Option<Rgba> {
    hex(&theme().colors.muted)
}
pub fn on() -> Option<Rgba> {
    hex(&theme().colors.on)
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
    theme().interface.motion
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

/// Tab count bounds, for the editors.
pub const TABS_MAX: usize = MAX_TABS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_families() {
        assert_eq!(glyphs_for_controller("Sony Interactive Entertainment DualSense Wireless Controller"), Glyphs::PlayStation);
        assert_eq!(glyphs_for_controller("Nintendo Switch Pro Controller"), Glyphs::Nintendo);
        assert_eq!(glyphs_for_controller("Steam Deck"), Glyphs::Deck);
        assert_eq!(glyphs_for_controller("Xbox Wireless Controller"), Glyphs::Xbox);
    }

    #[test]
    fn tile_sizes_keep_their_shape() {
        let (w, h) = tile_size(TileShape::Landscape, TileSize::Medium, 1.5);
        assert!((w / h - 16. / 9.).abs() < 0.01);
        assert_eq!(tile_size(TileShape::Square, TileSize::Small, 1.5), (120., 120.));
    }
}
