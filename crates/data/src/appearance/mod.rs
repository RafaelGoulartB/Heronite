//! Appearance settings that belong to this app only (Heronite has no
//! equivalent), kept in `AppPaths::appearance()` next to `ui-state.json`:
//!
//! - the opacity of the Collection background art (`collectionOpacity`, %);
//! - the look of the interface surfaces (menus, dialogs, pages...): glass
//!   (`interfaceMode: "transparent"`, art blurred behind translucent panels)
//!   or solid, with the panel opacity, the blur and the solid tint;
//! - colour, reading, covers, background picture, motion and layout
//!   ([`choices`]), the user's saved looks ([`profiles`]) and own theme
//!   ([`custom`]).
//!
//! The on/off, blur, darkening... of the Collection background itself stay in
//! `local_library/settings.json` (`background`).
//! Loading is lenient: anything missing or invalid gives the default, numbers
//! are clamped.

mod choices;
mod custom;
mod profiles;

pub use choices::*;
pub use custom::*;
pub use profiles::*;

use anyhow::Result;
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

pub const COLLECTION_OPACITY_RANGE: (f32, f32) = (0., 100.);
pub const INTERFACE_OPACITY_RANGE: (f32, f32) = (50., 100.);
pub const INTERFACE_BLUR_RANGE: (f32, f32) = (0., 60.);
/// The blur is applied in steps (each value is a cached raster).
pub const INTERFACE_BLUR_STEP: f32 = 5.;
/// Window widths [`AutoView`] can switch at, logical px.
pub const AUTO_VIEW_WIDTH_RANGE: (f32, f32) = (800., 2560.);
pub const AUTO_VIEW_WIDTH_STEP: f32 = 20.;

/// The Collection's view picked by the window's width: Grid while the window
/// is narrower than `width`, Focus from `width` up. Off by default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoView {
    pub on: bool,
    /// Logical px, within [`AUTO_VIEW_WIDTH_RANGE`].
    pub width: f32,
}

impl AutoView {
    pub const DEFAULT: AutoView = AutoView { on: false, width: 1400. };

    /// `Some(true)` for Grid, `Some(false)` for Focus; `None` while off.
    pub fn grid_for(self, window_w: f32) -> Option<bool> {
        self.on.then_some(window_w < self.width)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceMode {
    /// Blurred game art visible through translucent panels.
    Transparent,
    /// Opaque panels in a solid colour, no art.
    Solid,
}

impl InterfaceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transparent => "transparent",
            Self::Solid => "solid",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "transparent" => Some(Self::Transparent),
            "solid" => Some(Self::Solid),
            _ => None,
        }
    }
}

/// Preset colours of the solid panels: four for the dark theme, four for
/// the light one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidTint {
    Slate,
    Graphite,
    Midnight,
    Plum,
    Paper,
    Sand,
    Frost,
    Lilac,
}

impl SolidTint {
    pub const ALL: [SolidTint; 8] =
        [Self::Slate, Self::Graphite, Self::Midnight, Self::Plum, Self::Paper, Self::Sand, Self::Frost, Self::Lilac];
    pub const DARK: [SolidTint; 4] = [Self::Slate, Self::Graphite, Self::Midnight, Self::Plum];
    pub const LIGHT: [SolidTint; 4] = [Self::Paper, Self::Sand, Self::Frost, Self::Lilac];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slate => "slate",
            Self::Graphite => "graphite",
            Self::Midnight => "midnight",
            Self::Plum => "plum",
            Self::Paper => "paper",
            Self::Sand => "sand",
            Self::Frost => "frost",
            Self::Lilac => "lilac",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Slate => tr!("Slate"),
            Self::Graphite => tr!("Graphite"),
            Self::Midnight => tr!("Midnight"),
            Self::Plum => tr!("Plum"),
            Self::Paper => tr!("Paper"),
            Self::Sand => tr!("Sand"),
            Self::Frost => tr!("Frost"),
            Self::Lilac => tr!("Lilac"),
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
    /// One of the light theme's tints.
    pub fn is_light(self) -> bool {
        Self::LIGHT.contains(&self)
    }
    /// Panel colour (Slate is the dark glass tint, `rgb(22,27,36)`; Paper
    /// the light one, `rgb(232,234,238)`). The light tints are soft greys,
    /// not white, so a big window of them does not glare.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Slate => (22, 27, 36),
            Self::Graphite => (26, 26, 28),
            Self::Midnight => (16, 22, 42),
            Self::Plum => (34, 24, 40),
            Self::Paper => (232, 234, 238),
            Self::Sand => (234, 228, 218),
            Self::Frost => (224, 231, 240),
            Self::Lilac => (231, 226, 236),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    /// How visible the Collection background art is, 0-100 (%).
    pub collection_opacity: f32,
    pub interface_mode: InterfaceMode,
    /// Opacity of the glass panels, 50-100 (%); 100 is fully opaque.
    pub interface_opacity: f32,
    /// Blur of the art behind the glass panels, px.
    pub interface_blur: f32,
    /// Light or dark panels and text.
    pub theme: Theme,
    /// The solid panels' colour on the dark theme (one of [`SolidTint::DARK`]).
    pub solid_tint: SolidTint,
    /// The solid panels' colour on the light theme (one of [`SolidTint::LIGHT`]).
    pub solid_tint_light: SolidTint,
    /// A colour picked by hand for the dark theme's solid panels, over
    /// `solid_tint` (always a dark colour, see [`own_panel`]).
    pub solid_own: Option<Rgb>,
    /// The same for the light theme's.
    pub solid_own_light: Option<Rgb>,
    /// The colours of [`Theme::Custom`] (kept while another theme is in use).
    pub custom: Palette,
    pub accent: Accent,
    /// An accent picked by hand, over `accent` (each theme shows it in a
    /// shade that reads on its panels).
    pub accent_own: Option<Rgb>,
    /// Stronger secondary text and borders.
    pub high_contrast: bool,
    pub font: FontChoice,
    pub density: Density,
    pub cover_corners: Corners,
    pub cover_label: CoverLabel,
    /// Favourite and "update available" badges on the covers.
    pub cover_badges: bool,
    pub cover_hover: CoverHover,
    pub background: BackgroundSource,
    /// With [`BackgroundSource::Screenshot`]: a different screenshot every
    /// [`SLIDESHOW_SECS`].
    pub background_slideshow: bool,
    pub motion: Motion,
    pub start_view: StartView,
    pub auto_view: AutoView,
    pub sections: Sections,
}

/// Seconds between two screenshots of the background slideshow.
pub const SLIDESHOW_SECS: u64 = 20;

impl Appearance {
    /// Today's look (also what a missing file gives).
    pub const DEFAULT: Appearance = Appearance {
        collection_opacity: 100.,
        interface_mode: InterfaceMode::Transparent,
        interface_opacity: 90.,
        interface_blur: 30.,
        theme: Theme::DEFAULT,
        solid_tint: SolidTint::Slate,
        solid_tint_light: SolidTint::Paper,
        solid_own: None,
        solid_own_light: None,
        custom: Palette::LIGHT,
        accent: Accent::DEFAULT,
        accent_own: None,
        high_contrast: false,
        font: FontChoice::DEFAULT,
        density: Density::DEFAULT,
        cover_corners: Corners::DEFAULT,
        cover_label: CoverLabel::DEFAULT,
        cover_badges: false,
        cover_hover: CoverHover::DEFAULT,
        background: BackgroundSource::DEFAULT,
        background_slideshow: false,
        motion: Motion::DEFAULT,
        start_view: StartView::DEFAULT,
        auto_view: AutoView::DEFAULT,
        sections: Sections::DEFAULT,
    };
}

impl Default for Appearance {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A hand-picked panel colour stored at `key`, kept only on its own side.
fn own_colour(value: &Value, key: &str, light: bool) -> Option<Rgb> {
    let c = parse_hex(value.get(key)?.as_str()?)?;
    (is_light_colour(c) == light).then_some(c)
}

fn clamp((lo, hi): (f32, f32), v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        lo
    }
}

impl Appearance {
    /// Clamps every number to its range (the blur to its step).
    pub fn normalized(mut self) -> Self {
        self.collection_opacity = clamp(COLLECTION_OPACITY_RANGE, self.collection_opacity).round();
        self.interface_opacity = clamp(INTERFACE_OPACITY_RANGE, self.interface_opacity).round();
        self.interface_blur = clamp(INTERFACE_BLUR_RANGE, (self.interface_blur / INTERFACE_BLUR_STEP).round() * INTERFACE_BLUR_STEP);
        self.auto_view.width = clamp(AUTO_VIEW_WIDTH_RANGE, (self.auto_view.width / AUTO_VIEW_WIDTH_STEP).round() * AUTO_VIEW_WIDTH_STEP);
        self
    }

    /// `self` with `other`'s [`AutoView`], [`Theme`] and custom colours. They
    /// follow the window and the desktop, not a look: applying or matching a
    /// profile neither changes nor breaks them.
    pub fn with_window_choices_of(self, other: &Appearance) -> Self {
        Self { auto_view: other.auto_view, theme: other.theme, custom: other.custom, ..self }
    }

    pub fn from_value(value: &Value) -> Self {
        let d = Self::default();
        let num = |k: &str, default: f32| value.get(k).and_then(Value::as_f64).map(|v| v as f32).unwrap_or(default);
        let flag = |k: &str, default: bool| value.get(k).and_then(Value::as_bool).unwrap_or(default);
        let name = |k: &str| value.get(k).and_then(Value::as_str);
        let names = |k: &str| -> Vec<&str> {
            value.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
        };
        Self {
            collection_opacity: num("collectionOpacity", d.collection_opacity),
            interface_mode: name("interfaceMode").and_then(InterfaceMode::parse).unwrap_or(d.interface_mode),
            interface_opacity: num("interfaceOpacity", d.interface_opacity),
            interface_blur: num("interfaceBlur", d.interface_blur),
            theme: name("theme").and_then(Theme::parse).unwrap_or_default(),
            solid_tint: name("interfaceSolidTint").and_then(SolidTint::parse).filter(|t| !t.is_light()).unwrap_or(d.solid_tint),
            solid_tint_light: name("interfaceSolidTintLight")
                .and_then(SolidTint::parse)
                .filter(|t| t.is_light())
                .unwrap_or(d.solid_tint_light),
            solid_own: own_colour(value, "interfaceSolidColour", false),
            solid_own_light: own_colour(value, "interfaceSolidColourLight", true),
            custom: value.get("customPalette").map(Palette::from_value).unwrap_or(d.custom),
            accent: name("accent").and_then(Accent::parse).unwrap_or_default(),
            accent_own: name("accentColour").and_then(parse_hex),
            high_contrast: flag("highContrast", d.high_contrast),
            font: name("font").and_then(FontChoice::parse).unwrap_or_default(),
            density: name("density").and_then(Density::parse).unwrap_or_default(),
            cover_corners: name("coverCorners").and_then(Corners::parse).unwrap_or_default(),
            cover_label: name("coverLabel").and_then(CoverLabel::parse).unwrap_or_default(),
            cover_badges: flag("coverBadges", d.cover_badges),
            cover_hover: name("coverHover").and_then(CoverHover::parse).unwrap_or_default(),
            background: name("background").and_then(BackgroundSource::parse).unwrap_or_default(),
            background_slideshow: flag("backgroundSlideshow", d.background_slideshow),
            motion: name("motion").and_then(Motion::parse).unwrap_or_default(),
            start_view: name("startView").and_then(StartView::parse).unwrap_or_default(),
            auto_view: AutoView { on: flag("autoView", d.auto_view.on), width: num("autoViewWidth", d.auto_view.width) },
            sections: Sections::from_names(names("sections"), names("hiddenSections")),
        }
        .normalized()
    }

    pub fn to_value(&self) -> Value {
        json!({
            "collectionOpacity": self.collection_opacity,
            "interfaceMode": self.interface_mode.as_str(),
            "interfaceOpacity": self.interface_opacity,
            "interfaceBlur": self.interface_blur,
            "theme": self.theme.as_str(),
            "interfaceSolidTint": self.solid_tint.as_str(),
            "interfaceSolidTintLight": self.solid_tint_light.as_str(),
            "interfaceSolidColour": self.solid_own.map(hex),
            "interfaceSolidColourLight": self.solid_own_light.map(hex),
            "customPalette": self.custom.to_value(),
            "accent": self.accent.as_str(),
            "accentColour": self.accent_own.map(hex),
            "highContrast": self.high_contrast,
            "font": self.font.as_str(),
            "density": self.density.as_str(),
            "coverCorners": self.cover_corners.as_str(),
            "coverLabel": self.cover_label.as_str(),
            "coverBadges": self.cover_badges,
            "coverHover": self.cover_hover.as_str(),
            "background": self.background.as_str(),
            "backgroundSlideshow": self.background_slideshow,
            "motion": self.motion.as_str(),
            "startView": self.start_view.as_str(),
            "autoView": self.auto_view.on,
            "autoViewWidth": self.auto_view.width,
            "sections": self.sections.order().iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "hiddenSections": self.sections.hidden_names(),
        })
    }

    /// A missing or unreadable file gives the defaults.
    pub fn load(paths: &AppPaths) -> Self {
        match json_store::read_value(&paths.appearance()) {
            Ok(value) if value.is_object() => Self::from_value(&value),
            _ => Self::default(),
        }
    }

    /// Writes the settings atomically; keys this struct does not model are kept.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let Value::Object(known) = self.normalized().to_value() else { unreachable!("to_value builds an object") };
        json_store::update_value(&paths.appearance(), |doc| {
            if !doc.is_object() {
                *doc = Value::Object(Default::default());
            }
            let obj = doc.as_object_mut().expect("object");
            for (k, v) in known {
                obj.insert(k, v);
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("cfg"), cache: dir.join("cache") }
    }

    #[test]
    fn defaults_keep_todays_look() {
        let d = Appearance::default();
        assert_eq!(d.collection_opacity, 100.);
        assert_eq!(d.interface_mode, InterfaceMode::Transparent);
        assert_eq!(d.interface_opacity, 90.);
        assert_eq!(d.interface_blur, 30.);
        assert_eq!(d.solid_tint, SolidTint::Slate);
        assert_eq!(d.normalized(), d);
    }

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        assert_eq!(Appearance::load(&p), Appearance::default());
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::write(p.appearance(), "{not json").unwrap();
        assert_eq!(Appearance::load(&p), Appearance::default());
        std::fs::write(p.appearance(), r#"{"interfaceMode":"glow","interfaceOpacity":"x","interfaceSolidTint":3}"#).unwrap();
        assert_eq!(Appearance::load(&p), Appearance::default());
    }

    /// Dark until the user picks another theme, whatever the desktop uses.
    #[test]
    fn the_default_theme_is_dark() {
        assert_eq!(Appearance::default().theme, Theme::Dark);
        assert_eq!(Appearance::from_value(&json!({})).theme, Theme::Dark);
        assert_eq!(Appearance::from_value(&json!({"theme": "sepia"})).theme, Theme::Dark);
    }

    #[test]
    fn values_are_clamped() {
        let v = json!({"collectionOpacity": 250, "interfaceOpacity": 10, "interfaceBlur": 33, "interfaceMode": "solid"});
        let a = Appearance::from_value(&v);
        assert_eq!(a.collection_opacity, 100.);
        assert_eq!(a.interface_opacity, 50.);
        assert_eq!(a.interface_blur, 35.);
        assert_eq!(a.interface_mode, InterfaceMode::Solid);
        let a = Appearance::from_value(&json!({"collectionOpacity": -5, "interfaceOpacity": 400, "interfaceBlur": 999}));
        assert_eq!((a.collection_opacity, a.interface_opacity, a.interface_blur), (0., 100., 60.));
    }

    #[test]
    fn save_and_load_round_trip_keeping_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::write(p.appearance(), "{\n\t\"future\": [1]\n}").unwrap();
        let a = Appearance {
            collection_opacity: 60.,
            interface_mode: InterfaceMode::Solid,
            interface_opacity: 70.,
            interface_blur: 15.,
            theme: Theme::System,
            solid_tint: SolidTint::Midnight,
            solid_tint_light: SolidTint::Frost,
            solid_own: Some((40, 10, 20)),
            solid_own_light: None,
            custom: Palette { accent: (1, 2, 3), ..Palette::DARK },
            accent: Accent::Game,
            accent_own: Some((200, 90, 30)),
            high_contrast: true,
            font: FontChoice::System,
            density: Density::Compact,
            cover_corners: Corners::Round,
            cover_label: CoverLabel::LastPlayed,
            cover_badges: true,
            cover_hover: CoverHover::Glow,
            background: BackgroundSource::Screenshot,
            background_slideshow: true,
            motion: Motion::Reduced,
            start_view: StartView::Grid,
            auto_view: AutoView { on: true, width: 1600. },
            sections: Sections::DEFAULT.moved(Section::Related, -4).toggled(Section::Timeline),
        };
        a.save(&p).unwrap();
        assert_eq!(Appearance::load(&p), a);
        let on_disk = json_store::read_value(&p.appearance()).unwrap();
        assert_eq!(on_disk["future"], json!([1]));
        assert_eq!(on_disk["interfaceMode"], "solid");
        assert_eq!(on_disk["interfaceSolidTint"], "midnight");
        assert_eq!(on_disk["interfaceSolidTintLight"], "frost");
        assert_eq!(on_disk["theme"], "system");
        assert_eq!(on_disk["customPalette"]["accent"], "#010203");
        assert_eq!(on_disk["interfaceSolidColour"], "#280a14");
        assert_eq!(on_disk["interfaceSolidColourLight"], Value::Null);
        assert_eq!(on_disk["accentColour"], "#c85a1e");
        assert_eq!(on_disk["collectionOpacity"], 60.0);
        assert_eq!(on_disk["coverLabel"], "lastPlayed");
        assert_eq!(on_disk["sections"][0], "related");
        assert_eq!(on_disk["hiddenSections"], json!(["timeline"]));
        assert_eq!(on_disk["autoView"], true);
        assert_eq!(on_disk["autoViewWidth"], 1600.0);
    }

    #[test]
    fn auto_view_is_off_until_turned_on() {
        let a = Appearance::from_value(&json!({"startView": "grid"}));
        assert_eq!(a.auto_view, AutoView::DEFAULT);
        assert_eq!(a.auto_view.grid_for(900.), None);
        let a = Appearance::from_value(&json!({"autoView": true, "autoViewWidth": 1290}));
        assert_eq!(a.auto_view.width, 1300.);
        assert_eq!(a.auto_view.grid_for(1299.), Some(true));
        assert_eq!(a.auto_view.grid_for(1300.), Some(false));
        let a = Appearance::from_value(&json!({"autoView": true, "autoViewWidth": 99999}));
        assert_eq!(a.auto_view.width, AUTO_VIEW_WIDTH_RANGE.1);
    }

    #[test]
    fn old_files_keep_todays_look() {
        let v = json!({"collectionOpacity": 80, "interfaceMode": "solid"});
        let a = Appearance::from_value(&v);
        assert_eq!(a.accent, Accent::Ice);
        assert_eq!(a.theme, Theme::Dark);
        assert_eq!(a.solid_tint_light, SolidTint::Paper);
        assert_eq!(a.cover_corners.radius(), 6.);
        assert_eq!(a.sections, Sections::DEFAULT);
        assert_eq!(a.motion, Motion::Full);
    }

    #[test]
    fn tint_names_round_trip() {
        for t in SolidTint::ALL {
            assert_eq!(SolidTint::parse(t.as_str()), Some(t));
        }
        assert_eq!(SolidTint::Slate.rgb(), (22, 27, 36));
        assert!(SolidTint::DARK.iter().all(|t| !t.is_light()));
        assert!(SolidTint::LIGHT.iter().all(|t| t.is_light()));
    }

    #[test]
    fn each_theme_keeps_its_own_tints() {
        let a = Appearance::from_value(&json!({"interfaceSolidTint": "sand", "interfaceSolidTintLight": "plum"}));
        assert_eq!((a.solid_tint, a.solid_tint_light), (SolidTint::Slate, SolidTint::Paper));
        let a = Appearance::from_value(&json!({"interfaceSolidTint": "plum", "interfaceSolidTintLight": "sand"}));
        assert_eq!((a.solid_tint, a.solid_tint_light), (SolidTint::Plum, SolidTint::Sand));
    }

    #[test]
    fn hand_picked_panels_stay_on_their_side() {
        let a = Appearance::from_value(&json!({"interfaceSolidColour": "#f0f0f0", "interfaceSolidColourLight": "#101010"}));
        assert_eq!((a.solid_own, a.solid_own_light), (None, None));
        let a = Appearance::from_value(&json!({"interfaceSolidColour": "#101820", "interfaceSolidColourLight": "#eeeeee"}));
        assert_eq!((a.solid_own, a.solid_own_light), (Some((16, 24, 32)), Some((238, 238, 238))));
        // A mid grey is moved until the secondary text of its side reads on it.
        let (light, c) = own_panel((128, 128, 128));
        let muted = if light { Palette::LIGHT.muted } else { Palette::DARK.muted };
        assert!(contrast(c, muted) >= 4.5);
    }

    #[test]
    fn profiles_keep_the_theme_and_auto_view() {
        let mine = Appearance {
            theme: Theme::Custom,
            custom: Palette::DARK,
            auto_view: AutoView { on: true, width: 1200. },
            ..Appearance::DEFAULT
        };
        let look = Appearance { accent: Accent::Rose, ..Appearance::DEFAULT }.with_window_choices_of(&mine);
        assert_eq!((look.theme, look.custom, look.auto_view, look.accent), (Theme::Custom, Palette::DARK, mine.auto_view, Accent::Rose));
    }

    /// WCAG relative luminance of an sRGB colour.
    fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
        let lin = |c: u8| {
            let c = c as f64 / 255.;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Also on a card (the panel under 5% of the light theme's ink, `rgb(20,26,36)`).
    #[test]
    fn light_accents_are_readable_on_every_light_tint() {
        let card = |(r, g, b): (u8, u8, u8)| {
            let mix = |c: u8, ink: u8| (c as f64 * 0.95 + ink as f64 * 0.05).round() as u8;
            (mix(r, 20), mix(g, 26), mix(b, 36))
        };
        for a in Accent::ALL.iter().filter_map(|a| a.rgb_light()) {
            for t in SolidTint::LIGHT {
                for bg in [t.rgb(), card(t.rgb())] {
                    assert!(contrast(a, bg) >= 4.5, "{a:?} on {t:?}: {:.2}", contrast(a, bg));
                }
            }
        }
    }
}
