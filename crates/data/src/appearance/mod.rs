//! Appearance settings that belong to this app only (Heronite has no
//! equivalent), kept in `AppPaths::appearance()` next to `ui-state.json`:
//!
//! - the opacity of the Collection background art (`collectionOpacity`, %);
//! - the look of the interface surfaces (menus, dialogs, pages...): glass
//!   (`interfaceMode: "transparent"`, art blurred behind translucent panels)
//!   or solid, with the panel opacity, the blur and the solid tint;
//! - colour, reading, covers, background picture, motion and layout
//!   ([`choices`]), and the user's saved looks ([`profiles`]).
//!
//! The on/off, blur, darkening... of the Collection background itself stay in
//! `local_library/settings.json` (`background`).
//! Loading is lenient: anything missing or invalid gives the default, numbers
//! are clamped.

mod choices;
mod profiles;

pub use choices::*;
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

/// Preset colours of the solid panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidTint {
    Slate,
    Graphite,
    Midnight,
    Plum,
}

impl SolidTint {
    pub const ALL: [SolidTint; 4] = [Self::Slate, Self::Graphite, Self::Midnight, Self::Plum];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slate => "slate",
            Self::Graphite => "graphite",
            Self::Midnight => "midnight",
            Self::Plum => "plum",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Slate => tr!("Slate"),
            Self::Graphite => tr!("Graphite"),
            Self::Midnight => tr!("Midnight"),
            Self::Plum => tr!("Plum"),
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
    /// Panel colour (Slate is the glass tint, `rgb(22,27,36)`).
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Slate => (22, 27, 36),
            Self::Graphite => (26, 26, 28),
            Self::Midnight => (16, 22, 42),
            Self::Plum => (34, 24, 40),
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
    pub solid_tint: SolidTint,
    pub accent: Accent,
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
        solid_tint: SolidTint::Slate,
        accent: Accent::DEFAULT,
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
        sections: Sections::DEFAULT,
    };
}

impl Default for Appearance {
    fn default() -> Self {
        Self::DEFAULT
    }
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
        self
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
            solid_tint: name("interfaceSolidTint").and_then(SolidTint::parse).unwrap_or(d.solid_tint),
            accent: name("accent").and_then(Accent::parse).unwrap_or_default(),
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
            "interfaceSolidTint": self.solid_tint.as_str(),
            "accent": self.accent.as_str(),
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
            solid_tint: SolidTint::Midnight,
            accent: Accent::Game,
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
            sections: Sections::DEFAULT.moved(Section::Related, -4).toggled(Section::Timeline),
        };
        a.save(&p).unwrap();
        assert_eq!(Appearance::load(&p), a);
        let on_disk = json_store::read_value(&p.appearance()).unwrap();
        assert_eq!(on_disk["future"], json!([1]));
        assert_eq!(on_disk["interfaceMode"], "solid");
        assert_eq!(on_disk["interfaceSolidTint"], "midnight");
        assert_eq!(on_disk["collectionOpacity"], 60.0);
        assert_eq!(on_disk["coverLabel"], "lastPlayed");
        assert_eq!(on_disk["sections"][0], "related");
        assert_eq!(on_disk["hiddenSections"], json!(["timeline"]));
    }

    #[test]
    fn old_files_keep_todays_look() {
        let v = json!({"collectionOpacity": 80, "interfaceMode": "solid"});
        let a = Appearance::from_value(&v);
        assert_eq!(a.accent, Accent::Ice);
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
    }
}
