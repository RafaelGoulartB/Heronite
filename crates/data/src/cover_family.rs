//! Cover shapes ("families"). A game can hold one cover per family plus its
//! background:
//!
//! - [`CoverFamily::Tall`]: the 2:3, 3:4, 11:15 and 5:7 aspects share one
//!   picture, trimmed to the aspect. It is `coverUrl` of a record and of
//!   `art.json`, as it always was.
//! - [`CoverFamily::Square`] (1:1): `coverSquareUrl`.
//! - [`CoverFamily::Wide`] (16:9, Steam's 460:215 header): `coverWideUrl`.
//!
//! Each view asks for the family it draws (`crate::art::family_cover`); a
//! game without a picture of that family shows its tall cover cropped (for
//! Wide, its background first, as the Banner aspect always did).
//!
//! Which families an installation keeps ([`CoverFamilies`]): those in use
//! (the desktop's Cover aspect, the console theme's tiles; the tall one is
//! always in use: the Focus panel, list rows and the drawer draw it) and
//! those the user asked to keep (`settings.metadata.keepCoverFamilies`).
//! Fetches fill only the kept families.

use serde::{Deserialize, Serialize};

use crate::console_theme::{ConsolePrefs, ConsoleTheme, Layout, TileShape};
use crate::models::MetadataSettings;
use crate::paths::{AppPaths, DataPaths};
use crate::settings::CoverAspect;

/// A cover shape. JSON: `"tall"`, `"square"`, `"wide"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverFamily {
    #[default]
    Tall,
    Square,
    Wide,
}

impl CoverFamily {
    pub const ALL: [CoverFamily; 3] = [Self::Tall, Self::Square, Self::Wide];
    /// The families beside the tall cover: the ones that can be switched off.
    pub const EXTRA: [CoverFamily; 2] = [Self::Square, Self::Wide];

    pub fn id(self) -> &'static str {
        match self {
            Self::Tall => "tall",
            Self::Square => "square",
            Self::Wide => "wide",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    /// Its field id in the merge, `fieldSources`, `fieldPriority`, locks and
    /// `fieldsOff`. The tall one keeps `cover`, so saved priorities and
    /// switches keep their meaning.
    pub fn field(self) -> &'static str {
        match self {
            Self::Tall => "cover",
            Self::Square => "coverSquare",
            Self::Wide => "coverWide",
        }
    }

    pub fn from_field(field: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.field() == field)
    }

    /// The key of its address in a `metadata.json` record and in `art.json`.
    pub fn json_key(self) -> &'static str {
        match self {
            Self::Tall => "coverUrl",
            Self::Square => "coverSquareUrl",
            Self::Wide => "coverWideUrl",
        }
    }

    /// Width / height of the family's pictures (Tall: Steam's 2:3; Wide: 16:9).
    pub fn aspect(self) -> f32 {
        match self {
            Self::Tall => 2. / 3.,
            Self::Square => 1.,
            Self::Wide => 16. / 9.,
        }
    }
}

impl CoverAspect {
    /// The family whose picture this aspect shows.
    pub fn family(self) -> CoverFamily {
        match self {
            Self::Steam | Self::Igdb | Self::Gog | Self::Dvd => CoverFamily::Tall,
            Self::Square => CoverFamily::Square,
            Self::Banner => CoverFamily::Wide,
        }
    }
}

impl TileShape {
    /// The family a console tile of this shape draws.
    pub fn family(self) -> CoverFamily {
        match self {
            Self::Portrait => CoverFamily::Tall,
            Self::Square => CoverFamily::Square,
            Self::Landscape => CoverFamily::Wide,
        }
    }
}

/// The families a console theme draws: its tiles' shape, plus the tiles a
/// layout draws in shapes of its own (Xbox: a wide tile and square ones).
pub fn console_families(theme: &ConsoleTheme) -> Vec<CoverFamily> {
    let mut out = vec![theme.tiles.shape.family()];
    if theme.uses(Layout::Xbox) {
        out.extend([CoverFamily::Wide, CoverFamily::Square, CoverFamily::Tall]);
    }
    out.sort();
    out.dedup();
    out
}

/// What uses a family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FamilyUser {
    /// Settings → Appearance → Cover aspect: the covers of the desktop Grid
    /// and Focus.
    DesktopCovers,
    /// The rest of the desktop (the Focus panel's cover, list rows, the
    /// drawer, the pills): always tall.
    DesktopDetails,
    /// Console mode, with the theme of this name (a built-in theme's name is
    /// an English key the interface translates).
    Console { theme: String },
}

/// One family: who uses it and whether the user keeps it anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyState {
    pub family: CoverFamily,
    /// Empty: no view draws it.
    pub used_by: Vec<FamilyUser>,
    /// Listed in `keepCoverFamilies`.
    pub kept_by_choice: bool,
}

impl FamilyState {
    pub fn in_use(&self) -> bool {
        !self.used_by.is_empty()
    }

    /// Fetches fill it: in use, or kept by choice.
    pub fn kept(&self) -> bool {
        self.in_use() || self.kept_by_choice
    }
}

/// Which families an installation keeps and why (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverFamilies {
    /// One per family, in [`CoverFamily::ALL`] order.
    pub states: Vec<FamilyState>,
}

impl CoverFamilies {
    /// From the desktop's aspect, the families the user keeps and the console
    /// theme in use (`None`: no console mode).
    pub fn compute(aspect: CoverAspect, keep: &[CoverFamily], console: Option<&ConsoleTheme>) -> Self {
        let console_uses = console.map(console_families).unwrap_or_default();
        let states = CoverFamily::ALL
            .into_iter()
            .map(|family| {
                let mut used_by = Vec::new();
                if aspect.family() == family {
                    used_by.push(FamilyUser::DesktopCovers);
                }
                if family == CoverFamily::Tall {
                    used_by.push(FamilyUser::DesktopDetails);
                }
                if let Some(theme) = console.filter(|_| console_uses.contains(&family)) {
                    used_by.push(FamilyUser::Console { theme: theme.name.clone() });
                }
                FamilyState { family, used_by, kept_by_choice: family != CoverFamily::Tall && keep.contains(&family) }
            })
            .collect();
        Self { states }
    }

    /// From the settings and the console theme of the data folder `paths`
    /// (its app state: `AppPaths::for_data`). Unreadable files give defaults.
    pub fn load(paths: &DataPaths) -> Self {
        let settings = crate::settings::load(paths).unwrap_or_default();
        let console = ConsolePrefs::load(&AppPaths::for_data(paths));
        Self::compute(settings.cover_aspect, &kept_by_choice(&settings.metadata), Some(&console.theme))
    }

    pub fn state(&self, family: CoverFamily) -> &FamilyState {
        self.states.iter().find(|s| s.family == family).expect("one state per family")
    }

    pub fn is_kept(&self, family: CoverFamily) -> bool {
        self.state(family).kept()
    }

    pub fn is_in_use(&self, family: CoverFamily) -> bool {
        self.state(family).in_use()
    }

    /// The families fetches fill, in [`CoverFamily::ALL`] order (always
    /// with the tall one).
    pub fn kept(&self) -> Vec<CoverFamily> {
        self.states.iter().filter(|s| s.kept()).map(|s| s.family).collect()
    }

    /// The families some view draws.
    pub fn in_use(&self) -> Vec<CoverFamily> {
        self.states.iter().filter(|s| s.in_use()).map(|s| s.family).collect()
    }

    /// The fields of the kept families beside the tall cover (`coverSquare`,
    /// `coverWide`): what a fetch adds to `METADATA_FIELDS`.
    pub fn kept_extra_fields(&self) -> Vec<&'static str> {
        self.kept().into_iter().filter(|f| *f != CoverFamily::Tall).map(CoverFamily::field).collect()
    }
}

/// The families listed in `keepCoverFamilies` (unknown ids left out).
pub fn kept_by_choice(m: &MetadataSettings) -> Vec<CoverFamily> {
    m.keep_cover_families.iter().filter_map(|id| CoverFamily::parse(id)).filter(|f| *f != CoverFamily::Tall).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console_theme::presets;

    #[test]
    fn every_aspect_and_tile_shape_has_a_family() {
        use CoverFamily::*;
        let aspects = [
            (CoverAspect::Steam, Tall),
            (CoverAspect::Igdb, Tall),
            (CoverAspect::Gog, Tall),
            (CoverAspect::Dvd, Tall),
            (CoverAspect::Square, Square),
            (CoverAspect::Banner, Wide),
        ];
        for (aspect, family) in aspects {
            assert_eq!(aspect.family(), family, "{aspect:?}");
        }
        let shapes = [(TileShape::Portrait, Tall), (TileShape::Square, Square), (TileShape::Landscape, Wide)];
        for (shape, family) in shapes {
            assert_eq!(shape.family(), family, "{shape:?}");
        }
        assert_eq!(TileShape::ALL.len(), shapes.len());
    }

    #[test]
    fn ids_fields_and_keys_round_trip() {
        for f in CoverFamily::ALL {
            assert_eq!(CoverFamily::parse(f.id()), Some(f));
            assert_eq!(CoverFamily::from_field(f.field()), Some(f));
            assert_eq!(serde_json::to_value(f).unwrap(), f.id());
        }
        assert_eq!(CoverFamily::Tall.field(), "cover");
        assert_eq!(CoverFamily::Tall.json_key(), "coverUrl");
        assert_eq!(CoverFamily::parse("banner"), None);
    }

    #[test]
    fn kept_is_what_is_in_use_plus_what_the_user_keeps() {
        use CoverFamily::*;
        let minimal = presets::default_theme();
        let f = CoverFamilies::compute(CoverAspect::Steam, &[], Some(&minimal));
        assert_eq!(f.kept(), [Tall], "the defaults keep only the tall cover");
        assert_eq!(f.state(Tall).used_by[0], FamilyUser::DesktopCovers);
        assert!(f.state(Tall).used_by.contains(&FamilyUser::DesktopDetails));

        let f = CoverFamilies::compute(CoverAspect::Square, &[Wide], Some(&minimal));
        assert_eq!(f.in_use(), [Tall, Square]);
        assert_eq!(f.kept(), [Tall, Square, Wide]);
        assert!(f.state(Wide).kept_by_choice && !f.is_in_use(Wide));
        assert_eq!(f.kept_extra_fields(), ["coverSquare", "coverWide"]);
        assert_eq!(f.state(Square).used_by, [FamilyUser::DesktopCovers]);

        // A square console theme, and the Xbox layout's own tiles.
        let pulse = presets::by_id("pulse").unwrap();
        let f = CoverFamilies::compute(CoverAspect::Steam, &[], Some(&pulse));
        assert_eq!(f.in_use(), [Tall, Square]);
        assert_eq!(f.state(Square).used_by, [FamilyUser::Console { theme: pulse.name.clone() }]);
        let living = presets::by_id("living-room").unwrap();
        assert_eq!(console_families(&living), [Tall, Square, Wide]);
        let f = CoverFamilies::compute(CoverAspect::Banner, &[Tall], None);
        assert_eq!(f.in_use(), [Tall, Wide]);
        assert!(!f.state(Tall).kept_by_choice, "the tall cover is never a choice");
    }

    #[test]
    fn records_and_art_json_keep_their_meaning() {
        use crate::models::{ArtFile, CollectionGameMetadata};
        use serde_json::json;
        // A record and an art.json entry written before the families: read
        // and written back unchanged, the tall cover where it was.
        let old = json!({"appName": "a", "runner": "sideload", "developers": [], "publishers": [], "genres": [], "themes": [],
            "gameModes": [], "platforms": [], "features": [], "websites": [], "coverUrl": "https://t", "heroUrl": "https://h",
            "fieldSources": {"cover": "igdb"}, "futureKey": 1});
        let md: CollectionGameMetadata = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(
            (md.cover_url.as_deref(), md.cover_square_url.as_deref(), md.cover_wide_url.as_deref()),
            (Some("https://t"), None, None)
        );
        assert_eq!(serde_json::to_value(&md).unwrap(), old);
        let art = json!({"games": {"sideload:a": {"coverUrl": "localart://art/a/cover-1.png", "x": 2}}, "y": 3});
        let file: ArtFile = serde_json::from_value(art.clone()).unwrap();
        assert!(file.games["sideload:a"].cover_square_url.is_none());
        assert_eq!(serde_json::to_value(&file).unwrap(), art);

        // The new keys are plain additive strings: an older build keeps them
        // in its `extra` map and writes them back.
        let mut md = md;
        md.cover_square_url = Some("https://sq".into());
        md.cover_wide_url = Some("localart://metadata-art/sideload_a/cover-wide.jpg".into());
        md.field_sources.insert(CoverFamily::Square.field().into(), "steamgriddb".into());
        let v = serde_json::to_value(&md).unwrap();
        assert_eq!(v["coverSquareUrl"], "https://sq");
        assert_eq!(v["coverUrl"], "https://t");
        assert_eq!(v["fieldSources"], json!({"cover": "igdb", "coverSquare": "steamgriddb"}));
        let back: CollectionGameMetadata = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), v);
        let older: crate::models::Extra = serde_json::from_value(v).unwrap();
        assert!(older.contains_key("coverSquareUrl") && older.contains_key("coverWideUrl"));
    }

    #[test]
    fn load_reads_the_settings_and_the_console_theme_of_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        assert_eq!(CoverFamilies::load(&paths).kept(), [CoverFamily::Tall]);
        crate::settings::update_metadata(&paths, |m| {
            m.cover_aspect = "banner".into();
            m.keep_cover_families = vec!["square".into(), "future".into()];
        })
        .unwrap();
        let f = CoverFamilies::load(&paths);
        assert_eq!(f.kept(), CoverFamily::ALL);
        assert_eq!(f.in_use(), [CoverFamily::Tall, CoverFamily::Wide]);
    }
}
