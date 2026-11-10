//! Argument and result types: ports of the Collection types in
//! `src/common/types/local-library.ts` that the metadata/art backend uses.

use heronite_data::models::{CollectionGameMetadata, LocalGameMeta};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};

pub use heronite_data::settings::SOURCE_IDS;

/// What a network provider got: tells "nothing there" from "could not ask".
#[derive(Debug, Clone)]
pub enum Lookup<T> {
    Found(T),
    /// The service answered and has nothing for this game.
    NotFound,
    /// No answer at all (offline, DNS, timeout).
    Unreachable,
    /// Not asked (no credentials, no id to look up).
    Skipped,
}

impl<T> Lookup<T> {
    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Found(v) => Some(v),
            _ => None,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Lookup<U> {
        match self {
            Self::Found(v) => Lookup::Found(f(v)),
            Self::NotFound => Lookup::NotFound,
            Self::Unreachable => Lookup::Unreachable,
            Self::Skipped => Lookup::Skipped,
        }
    }

    /// A provider failure: transport errors are `Unreachable`, the rest
    /// (bad status, bad JSON) count as an answer without data.
    pub fn from_error(err: &anyhow::Error) -> Self {
        if crate::http::is_unreachable(err) {
            Self::Unreachable
        } else {
            Self::NotFound
        }
    }
}

/// How a source found the game. The order is from surest to least sure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchedBy {
    /// The game's own record (its entry in the store library).
    Game,
    /// An id the game carries for this source: an IGDB id picked or matched
    /// before, a Lutris slug, a Playnite id.
    SourceId,
    SteamId,
    RomHash,
    /// A title search whose best hit has the same title (normalized).
    ExactTitle,
    /// A title search whose best hit only looks like the title.
    FuzzyTitle,
}

impl MatchedBy {
    /// Whether the match can be saved without asking: everything but a
    /// fuzzy title.
    pub fn is_sure(self) -> bool {
        self != Self::FuzzyTitle
    }

    /// [`ExactTitle`](Self::ExactTitle) when `found` is `title` once
    /// normalized, else [`FuzzyTitle`](Self::FuzzyTitle).
    pub fn by_title(title: &str, found: Option<&str>) -> Self {
        if found.is_some_and(|f| crate::util::score_title(title, f) == 100) {
            Self::ExactTitle
        } else {
            Self::FuzzyTitle
        }
    }
}

/// `CollectionMetadataCandidate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataCandidate {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_year: Option<i32>,
    /// `None` (absent in older data): not said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<MatchedBy>,
    pub metadata: CollectionGameMetadata,
}

impl MetadataCandidate {
    /// The candidate of `source` for `metadata`: its title and release year
    /// are the match shown to the user.
    pub fn new(source: &str, metadata: CollectionGameMetadata, matched_by: Option<MatchedBy>) -> Self {
        let match_year = metadata.release_date.as_deref().and_then(|d| {
            let head: String = d.chars().take(4).collect();
            head.trim().parse::<i32>().ok().filter(|y| *y != 0)
        });
        Self { source: source.to_owned(), match_name: metadata.title.clone(), match_year, matched_by, metadata }
    }
}

/// `CollectionMetadataPreview`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPreview {
    pub app_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<CollectionGameMetadata>,
    pub proposed: CollectionGameMetadata,
    pub candidates: Vec<MetadataCandidate>,
    pub igdb_configured: bool,
    /// Every online provider that was asked was unreachable: the candidates
    /// hold local data only and must not be saved as a fetch.
    #[serde(default)]
    pub offline: bool,
}

/// `CollectionMetadataApplyArgs` (plus the optional Steam `language` that
/// the TS `GameRef` carries; without it Steam text is English).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataApplyArgs {
    pub app_name: String,
    pub runner: String,
    pub title: String,
    #[serde(default)]
    pub steam_app_id: Option<String>,
    /// A source id or `"auto"`.
    #[serde(default)]
    pub source: Option<String>,
    /// field -> source id | `"keep"`.
    #[serde(default)]
    pub field_picks: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub overwrite_existing: Option<bool>,
    #[serde(default)]
    pub preserve_manual: Option<bool>,
    #[serde(default)]
    pub download_images: Option<bool>,
    #[serde(default)]
    pub igdb_id: Option<i64>,
    #[serde(default)]
    pub lutris_slug: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

/// `CollectionMetadataSearchHit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSearchHit {
    pub source: String,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
}

/// `CollectionMetadataBulkItem`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkItem {
    pub app_name: String,
    pub runner: String,
    pub title: String,
    #[serde(default)]
    pub steam_app_id: Option<String>,
}

/// `CollectionMetadataBulkArgs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkArgs {
    pub games: Vec<BulkItem>,
    #[serde(default)]
    pub overwrite_existing: Option<bool>,
    #[serde(default)]
    pub preserve_manual: Option<bool>,
    #[serde(default)]
    pub download_images: Option<bool>,
}

/// `CollectionMetadataBulkState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BulkState {
    #[default]
    Idle,
    Running,
    Cancelling,
    Done,
}

/// `CollectionMetadataBulkProgress`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkProgress {
    pub state: BulkState,
    pub total: usize,
    pub processed: usize,
    pub updated: usize,
    pub skipped: usize,
    pub failed: usize,
    pub percent: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_title: Option<String>,
    pub errors: Vec<String>,
}

/// `CollectionGameDetailsPatch`. `critic_score`: `None` leaves it, `Some(None)`
/// clears it (JSON `null`), `Some(Some(n))` sets it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailsPatch {
    pub app_name: String,
    pub runner: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub developers: Option<Vec<String>>,
    #[serde(default)]
    pub publishers: Option<Vec<String>>,
    #[serde(default)]
    pub genres: Option<Vec<String>>,
    #[serde(default)]
    pub themes: Option<Vec<String>>,
    #[serde(default)]
    pub game_modes: Option<Vec<String>>,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    #[serde(default)]
    pub series: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub critic_score: Option<Option<f64>>,
    #[serde(default)]
    pub websites: Option<Vec<String>>,
}

fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// `CollectionGameDetailsResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailsResult {
    pub metadata: CollectionGameMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<LocalGameMeta>,
}

/// `CollectionArtKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtKind {
    Cover,
    Hero,
}

impl ArtKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Hero => "hero",
        }
    }
}

/// `CollectionWebImage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebImage {
    pub image_url: String,
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
}

/// The art picker's search results, by where they come from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtImages {
    pub steamgrid: Vec<WebImage>,
    pub web: Vec<WebImage>,
}

/// `SteamHeroCacheResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamHeroCacheResult {
    pub url: String,
    pub path: String,
    pub from_cache: bool,
}

/// `IgdbCredentialTest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgdbCredentialTest {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matched_by_title_tells_exact_from_fuzzy() {
        assert_eq!(MatchedBy::by_title("Portal 2", Some("portal 2")), MatchedBy::ExactTitle);
        assert_eq!(MatchedBy::by_title("Portal 2", Some("Portal 2: Complete")), MatchedBy::FuzzyTitle);
        assert_eq!(MatchedBy::by_title("Portal 2", None), MatchedBy::FuzzyTitle);
        assert!(MatchedBy::ExactTitle.is_sure() && MatchedBy::Game.is_sure() && !MatchedBy::FuzzyTitle.is_sure());
    }

    #[test]
    fn candidate_match_method_is_additive() {
        let old = r#"{"source":"igdb","matchName":"X","metadata":{"appName":"a","runner":"sideload"}}"#;
        let c: MetadataCandidate = serde_json::from_str(old).unwrap();
        assert_eq!(c.matched_by, None);
        assert!(serde_json::to_value(&c).unwrap().get("matchedBy").is_none());

        let mut md = c.metadata.clone();
        md.title = Some("X".into());
        md.release_date = Some("2011-04-18".into());
        let c = MetadataCandidate::new("steam", md, Some(MatchedBy::SteamId));
        assert_eq!((c.match_name.as_deref(), c.match_year), (Some("X"), Some(2011)));
        assert_eq!(serde_json::to_value(&c).unwrap()["matchedBy"], "steamId");
    }
}
