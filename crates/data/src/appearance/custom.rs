//! The user's own theme (`theme: "custom"`): six colours the interface is
//! derived from (`customPalette` in `appearance.json`), the ones saved under
//! a name (`customThemes`), and a file format to share one. Also the colour
//! arithmetic the editor and the theme share (contrast, HSL, readable shades).

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::json_store;
use crate::paths::AppPaths;

/// An sRGB colour.
pub type Rgb = (u8, u8, u8);

/// A colour of a [`Palette`] the user picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Behind everything: the Collection's page and the fade over its art.
    Page,
    /// Menus, dialogs, drawers, pages like Settings.
    Panel,
    /// What sits a step off a panel: popovers, the active tab, fields.
    Lift,
    Text,
    /// Captions, descriptions, labels.
    Muted,
    /// The selection ring, links, active buttons and sliders.
    Accent,
}

impl Role {
    pub const ALL: [Role; 6] = [Role::Page, Role::Panel, Role::Lift, Role::Text, Role::Muted, Role::Accent];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Page => "page",
            Role::Panel => "panel",
            Role::Lift => "lift",
            Role::Text => "text",
            Role::Muted => "muted",
            Role::Accent => "accent",
        }
    }
}

/// The colours of a custom theme; everything else (lines, hovers, buttons,
/// shadows, the fade over the art) is derived from them, on the light or the
/// dark side as `light` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub light: bool,
    pub page: Rgb,
    pub panel: Rgb,
    pub lift: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
}

impl Palette {
    /// The light theme's colours, where a new custom theme starts.
    pub const LIGHT: Palette = Palette {
        light: true,
        page: (223, 227, 232),
        panel: (232, 234, 238),
        lift: (241, 243, 246),
        text: (0x20, 0x26, 0x2f),
        muted: (0x47, 0x52, 0x62),
        accent: (0x19, 0x64, 0x79),
    };
    /// The dark theme's colours.
    pub const DARK: Palette = Palette {
        light: false,
        page: (7, 9, 12),
        panel: (22, 27, 36),
        lift: (44, 51, 64),
        text: (0xe8, 0xed, 0xf4),
        muted: (0x9a, 0xa6, 0xb6),
        accent: (0xa5, 0xdc, 0xeb),
    };

    pub fn get(&self, role: Role) -> Rgb {
        match role {
            Role::Page => self.page,
            Role::Panel => self.panel,
            Role::Lift => self.lift,
            Role::Text => self.text,
            Role::Muted => self.muted,
            Role::Accent => self.accent,
        }
    }

    pub fn set(&mut self, role: Role, c: Rgb) {
        match role {
            Role::Page => self.page = c,
            Role::Panel => self.panel = c,
            Role::Lift => self.lift = c,
            Role::Text => self.text = c,
            Role::Muted => self.muted = c,
            Role::Accent => self.accent = c,
        }
    }

    /// The colours text sits on: the panel, a card on it (5% of the text
    /// colour) and the page.
    pub fn backgrounds(&self) -> [Rgb; 3] {
        [self.panel, mix(self.panel, self.text, 0.05), self.page]
    }

    /// Lowest contrast of `role` against [`Self::backgrounds`] (`None` for
    /// the surfaces themselves).
    pub fn contrast_of(&self, role: Role) -> Option<f32> {
        matches!(role, Role::Text | Role::Muted | Role::Accent)
            .then(|| self.backgrounds().iter().map(|bg| contrast(self.get(role), *bg)).fold(f32::MAX, f32::min))
    }

    /// `role`'s colour moved until it reads on every background (4.5:1).
    pub fn readable(&self, role: Role) -> Rgb {
        readable_on(self.get(role), &self.backgrounds(), self.light)
    }

    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("base".into(), json!(if self.light { "light" } else { "dark" }));
        for r in Role::ALL {
            m.insert(r.as_str().into(), json!(hex(self.get(r))));
        }
        Value::Object(m)
    }

    /// Lenient: a missing or bad colour takes the base theme's.
    pub fn from_value(value: &Value) -> Self {
        let base = if value.get("base").and_then(Value::as_str) == Some("dark") { Palette::DARK } else { Palette::LIGHT };
        let mut p = base;
        for r in Role::ALL {
            if let Some(c) = value.get(r.as_str()).and_then(Value::as_str).and_then(parse_hex) {
                p.set(r, c);
            }
        }
        p
    }
}

impl Default for Palette {
    fn default() -> Self {
        Palette::LIGHT
    }
}

/// A custom theme the user saved.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomTheme {
    pub name: String,
    pub palette: Palette,
}

/// `#rrggbb`.
pub fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// `#rrggbb`, `rrggbb` or `#rgb`.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    let digits: Vec<u8> = s.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    match digits.as_slice() {
        [r, g, b] => Some((r * 17, g * 17, b * 17)),
        [r1, r2, g1, g2, b1, b2] => Some((r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2)),
        _ => None,
    }
}

/// `a` moved `t` (0..=1) of the way to `b`.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0., 1.)).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// WCAG relative luminance.
pub fn luminance((r, g, b): Rgb) -> f32 {
    let lin = |c: u8| {
        let c = c as f32 / 255.;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG contrast ratio, 1..=21.
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Hue 0..360, saturation and lightness 0..=1.
pub fn to_hsl((r, g, b): Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255., g as f32 / 255., b as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.;
    if max - min < 1e-6 {
        return (0., 0., l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2. - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.)
    } else if max == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    (h * 60., s, l)
}

pub fn from_hsl(h: f32, s: f32, l: f32) -> Rgb {
    let (s, l) = (s.clamp(0., 1.), l.clamp(0., 1.));
    let c = (1. - (2. * l - 1.).abs()) * s;
    let hp = h.rem_euclid(360.) / 60.;
    let x = c * (1. - (hp % 2. - 1.).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = l - c / 2.;
    let to8 = |v: f32| ((v + m) * 255.).round().clamp(0., 255.) as u8;
    (to8(r), to8(g), to8(b))
}

/// Hue 0..360, saturation and value 0..=1 (what a colour picker's square
/// and hue strip show).
pub fn to_hsv((r, g, b): Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255., g as f32 / 255., b as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d < 1e-6 {
        0.
    } else if max == r {
        60. * ((g - b) / d).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / d + 2.)
    } else {
        60. * ((r - g) / d + 4.)
    };
    let s = if max <= 0. { 0. } else { d / max };
    (h, s, max)
}

pub fn from_hsv(h: f32, s: f32, v: f32) -> Rgb {
    let (s, v) = (s.clamp(0., 1.), v.clamp(0., 1.));
    let c = v * s;
    let hp = h.rem_euclid(360.) / 60.;
    let x = c * (1. - (hp % 2. - 1.).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = v - c;
    let to8 = |x: f32| ((x + m) * 255.).round().clamp(0., 255.) as u8;
    (to8(r), to8(g), to8(b))
}

/// Light enough for dark text: black reads better on it than white.
pub fn is_light_colour(c: Rgb) -> bool {
    contrast(c, (0, 0, 0)) > contrast(c, (255, 255, 255))
}

/// A panel colour picked by hand: whether it belongs to the light theme, and
/// the colour moved until that theme's secondary text reads on it.
pub fn own_panel(c: Rgb) -> (bool, Rgb) {
    let light = is_light_colour(c);
    let muted = if light { Palette::LIGHT.muted } else { Palette::DARK.muted };
    (light, readable_on(c, &[muted], !light))
}

/// `c` with its hue kept, darkened (on a light theme) or lightened (on a
/// dark one) until it reads on every one of `bgs` (4.6:1).
pub fn readable_on(c: Rgb, bgs: &[Rgb], light: bool) -> Rgb {
    let ok = |c: Rgb| bgs.iter().all(|bg| contrast(c, *bg) >= 4.6);
    let (h, s, mut l) = to_hsl(c);
    let mut out = c;
    while !ok(out) && (if light { l > 0. } else { l < 1. }) {
        l = if light { (l - 0.01).max(0.) } else { (l + 0.01).min(1.) };
        out = from_hsl(h, s, l);
    }
    out
}

/// The saved custom themes, in the order they were saved; entries without a
/// name are skipped.
pub fn load_custom_themes(paths: &AppPaths) -> Vec<CustomTheme> {
    let Ok(doc) = json_store::read_value(&paths.appearance()) else { return Vec::new() };
    doc.get("customThemes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
                    Some(CustomTheme { name: name.to_owned(), palette: Palette::from_value(v) })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Replaces the saved custom themes; the rest of the file is kept.
pub fn save_custom_themes(paths: &AppPaths, themes: &[CustomTheme]) -> Result<()> {
    let list: Vec<Value> = themes
        .iter()
        .map(|t| {
            let mut v = t.palette.to_value();
            v["name"] = json!(t.name);
            v
        })
        .collect();
    json_store::update_value(&paths.appearance(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        doc["customThemes"] = Value::Array(list);
        Ok(())
    })
}

/// Marker key of an exported theme (its value is the format version).
const EXPORT_KEY: &str = "heroniteTheme";

/// Writes `palette` to `path` as a file another install can import.
pub fn export_theme(path: &Path, name: &str, palette: &Palette) -> Result<()> {
    let mut v = palette.to_value();
    v[EXPORT_KEY] = json!(1);
    v["name"] = json!(name);
    json_store::write(path, &v)
}

/// Reads a file written by [`export_theme`]: the palette and its name (the
/// file name when it has none).
pub fn import_theme(path: &Path) -> Result<(String, Palette)> {
    let v = json_store::read_value(path).with_context(|| format!("reading {}", path.display()))?;
    if v.get(EXPORT_KEY).is_none() {
        bail!("{} is not an exported theme", path.display());
    }
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default();
    Ok((name, Palette::from_value(&v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> AppPaths {
        AppPaths { config: dir.join("cfg"), cache: dir.join("cache") }
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(parse_hex("#1a2b3c"), Some((0x1a, 0x2b, 0x3c)));
        assert_eq!(parse_hex(" 1A2B3C "), Some((0x1a, 0x2b, 0x3c)));
        assert_eq!(parse_hex("#abc"), Some((0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#gg0000"), None);
        assert_eq!(hex((1, 2, 255)), "#0102ff");
    }

    #[test]
    fn hsl_round_trip() {
        for c in [(0, 0, 0), (255, 255, 255), (232, 234, 238), (0x19, 0x64, 0x79), (200, 30, 90)] {
            let (h, s, l) = to_hsl(c);
            let back = from_hsl(h, s, l);
            let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
            assert!(d(back.0, c.0) <= 1 && d(back.1, c.1) <= 1 && d(back.2, c.2) <= 1, "{c:?} -> {back:?}");
        }
    }

    #[test]
    fn hsv_round_trip() {
        for c in [(0, 0, 0), (255, 255, 255), (232, 234, 238), (0x19, 0x64, 0x79), (200, 30, 90), (255, 0, 0)] {
            let (h, s, v) = to_hsv(c);
            let back = from_hsv(h, s, v);
            let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
            assert!(d(back.0, c.0) <= 1 && d(back.1, c.1) <= 1 && d(back.2, c.2) <= 1, "{c:?} -> {back:?}");
        }
        assert_eq!(from_hsv(120., 1., 1.), (0, 255, 0));
        assert!(is_light_colour((232, 234, 238)) && !is_light_colour((22, 27, 36)));
    }

    #[test]
    fn palette_round_trip_and_lenient() {
        let mut p = Palette::DARK;
        p.set(Role::Accent, (255, 0, 128));
        assert_eq!(Palette::from_value(&p.to_value()), p);
        let p = Palette::from_value(&json!({"base": "dark", "text": "nope", "panel": "#101010"}));
        assert_eq!((p.light, p.text, p.panel), (false, Palette::DARK.text, (16, 16, 16)));
        assert_eq!(Palette::from_value(&json!(null)), Palette::LIGHT);
    }

    #[test]
    fn built_in_palettes_read_well() {
        for p in [Palette::LIGHT, Palette::DARK] {
            for r in [Role::Text, Role::Muted, Role::Accent] {
                assert!(p.contrast_of(r).unwrap() >= 4.5, "{r:?} {:?}", p.contrast_of(r));
            }
        }
        assert_eq!(Palette::LIGHT.contrast_of(Role::Panel), None);
    }

    #[test]
    fn readable_fixes_a_faint_colour_on_either_side() {
        let mut p = Palette::LIGHT;
        p.text = (180, 185, 190);
        let fixed = p.readable(Role::Text);
        assert!(p.backgrounds().iter().all(|bg| contrast(fixed, *bg) >= 4.5));
        let mut p = Palette::DARK;
        p.accent = (40, 60, 120);
        let fixed = p.readable(Role::Accent);
        assert!(p.backgrounds().iter().all(|bg| contrast(fixed, *bg) >= 4.5));
        assert!(luminance(fixed) > luminance(p.accent));
    }

    #[test]
    fn saved_themes_live_next_to_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        assert!(load_custom_themes(&p).is_empty());
        let saved = vec![CustomTheme { name: "Dusk".into(), palette: Palette::DARK }];
        save_custom_themes(&p, &saved).unwrap();
        assert_eq!(load_custom_themes(&p), saved);
        let file = dir.path().join("Dusk.json");
        export_theme(&file, "", &Palette::DARK).unwrap();
        assert_eq!(import_theme(&file).unwrap(), ("Dusk".to_string(), Palette::DARK));
        std::fs::write(&file, "{\"base\":\"dark\"}").unwrap();
        assert!(import_theme(&file).is_err());
    }
}
