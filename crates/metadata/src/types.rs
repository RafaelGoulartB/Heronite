//! Argument and result types: ports of the Collection types in
//! `src/common/types/local-library.ts` that the metadata/art backend uses.

pub use heronite_data::cover_family::CoverFamily;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata, LocalGameMeta};
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

/// The year of a release date (`2011-04-18`, `2011`); `None` when unknown.
pub fn release_year(date: Option<&str>) -> Option<i32> {
    let head: String = date?.chars().take(4).collect();
    head.trim().parse::<i32>().ok().filter(|y| *y != 0)
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
    /// Which game of the source this is (IGDB id, Steam app id, Lutris
    /// slug). `None`: the source has no ids (the store's own record).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_id: Option<String>,
    pub metadata: CollectionGameMetadata,
    /// The pictures of `metadata` the source flags as adult, by field id
    /// (`cover`, `hero`, `coverSquare`, `coverWide`). Unless
    /// `settings.metadata.allowAdultCovers` is on, the gather drops them
    /// ([`MetadataCandidate::drop_adult`]) so the next source fills them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adult_pictures: Vec<String>,
}

impl MetadataCandidate {
    /// The candidate of `source` for `metadata`: its title and release year
    /// are the match shown to the user.
    pub fn new(source: &str, metadata: CollectionGameMetadata, matched_by: Option<MatchedBy>) -> Self {
        let match_year = release_year(metadata.release_date.as_deref());
        Self {
            source: source.to_owned(),
            match_name: metadata.title.clone(),
            match_year,
            matched_by,
            match_id: None,
            metadata,
            adult_pictures: Vec::new(),
        }
    }

    /// Clears the pictures flagged adult.
    pub fn drop_adult(&mut self) {
        for field in std::mem::take(&mut self.adult_pictures) {
            if let Some(kind) = ArtKind::from_field(&field) {
                *kind.of_record_mut(&mut self.metadata) = None;
            }
        }
    }

    /// Says which game of the source this is.
    pub fn with_id(mut self, id: Option<String>) -> Self {
        self.match_id = id.filter(|i| !i.is_empty());
        self
    }

    /// The candidate as a choice the user can pick: its game, where it is
    /// known, its cover as the thumbnail.
    pub fn hit(&self) -> SourceHit {
        let md = &self.metadata;
        let platform = (!md.platforms.is_empty()).then(|| md.platforms.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
        SourceHit {
            source: self.source.clone(),
            id: self.match_id.clone(),
            title: self.match_name.clone(),
            year: self.match_year,
            platform,
            thumb_url: md.cover_url.clone().filter(|u| u.starts_with("http")),
            matched_by: self.matched_by,
            alt_title: None,
            thumb_adult: self.adult_pictures.iter().any(|f| f == "cover"),
        }
    }
}

/// A game a source has: a search hit, or what a source proposed for a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHit {
    pub source: String,
    /// The game's id in the source; `None` when it has none (it cannot be
    /// pinned).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    /// Platform names as the source writes them ("PlayStation, PC").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// A small cover (an `http` URL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    /// How the source found it for the game; `None` for a search hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<MatchedBy>,
    /// Its name in another script (the original Japanese of a visual
    /// novel), shown after the year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_title: Option<String>,
    /// The source flags the thumbnail as adult: it is left out unless
    /// adult covers are allowed ([`crate::sources::SourceRegistry::search`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub thumb_adult: bool,
}

impl SourceHit {
    /// Clears the thumbnail when it is flagged adult.
    pub fn drop_adult(&mut self) {
        if std::mem::take(&mut self.thumb_adult) {
            self.thumb_url = None;
        }
    }
}

/// Why a source was not asked about a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotAsked {
    /// Switched off in Settings → Metadata.
    Off,
    /// Not for this game (its audience, or no id to look it up with).
    NotForThisGame,
    /// It needs a key or an account the user has not set.
    NeedsKey,
    /// Its database is not downloaded ([`crate::sources::Needs::Download`]).
    NeedsDownload,
    /// Left out by the request (other fields or sources were chosen).
    NotChosen,
    /// Asked only as a fallback, and nothing called for it: the other
    /// sources found the game ([`crate::sources::Asked::AsFallback`]).
    Fallback,
    /// It could not be loaded, or failed too many times in a row
    /// ([`crate::sources::SourceHealth`]).
    Failing,
}

/// What one source answered about a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum SourceAnswer {
    #[serde(rename_all = "camelCase")]
    Found {
        hit: SourceHit,
        /// The user chose this game of the source; every lookup uses it.
        pinned: bool,
        /// The proposal takes its values ([`crate::confidence`] may leave
        /// out a guess that is another game).
        used: bool,
    },
    NotFound,
    Unreachable,
    NotAsked {
        why: NotAsked,
    },
}

/// A details source and its answer, for the per-game dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceState {
    pub source: String,
    pub answer: SourceAnswer,
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
    /// Every details source that is offered (and any other that answered),
    /// in registry order, with its answer.
    #[serde(default)]
    pub sources: Vec<SourceState>,
    /// Fields the record keeps as they are ([`crate::record::locked_fields`]).
    #[serde(default)]
    pub locks: Vec<String>,
    /// Set when an automatic update would not save these candidates but
    /// send the game to review; `proposed` then holds no fetched value
    /// unless a source is pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<crate::review::ReviewReason>,
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
    /// Only these fields (`METADATA_FIELDS` ids) are merged; the others stay
    /// exactly as they are, their recorded source too. `None`: all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_fields: Option<Vec<String>>,
    /// Only these sources are asked (sources that are off never are).
    /// `None`: every source that is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_sources: Option<Vec<String>>,
    /// source id -> the game of that source the user chose (looked up by
    /// that id, saved as pinned), or `None` to go back to finding it
    /// automatically. Sources left out keep what the record has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pins: Option<IndexMap<String, Option<String>>>,
    /// field -> locked after saving. A field named here takes the merge's
    /// value (the user's pick) before the lock is set; a locked field not
    /// named keeps its value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locks: Option<IndexMap<String, bool>>,
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

/// `CollectionMetadataBulkArgs`, plus what to update. The constructors in
/// [`crate::job`] are the presets.
///
/// Without `mode` a job does what the older dialog asked:
/// `overwrite_existing` not `true` skips the games fetched without error and
/// fills only empty fields of the others; `true` fetches every game again,
/// keeping the fields edited by hand unless `preserve_manual` is `false`.
/// With `mode`, those two are ignored and no game is skipped for being fetched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkArgs {
    pub games: Vec<BulkItem>,
    #[serde(default)]
    pub overwrite_existing: Option<bool>,
    #[serde(default)]
    pub preserve_manual: Option<bool>,
    /// `None`: `settings.metadata.downloadImages`.
    #[serde(default)]
    pub download_images: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<crate::job::UpdateMode>,
    /// Ids of [`crate::job::bulk_fields`]; the others stay exactly as they
    /// are. `None`: every `METADATA_FIELDS` entry and no extras.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    /// The sources to ask, among those that are on (one = "only this
    /// source"). `None`: every source that is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    #[serde(default)]
    pub kind: crate::job::JobKind,
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

/// `CollectionMetadataBulkProgress` of the running or last job, plus the
/// jobs waiting and every game's result.
///
/// The counters keep the older meaning: `updated` = saved without error
/// (changed or not), `skipped` = not fetched (already fetched, nothing
/// missing, no source for it), `failed` = not found, offline or failed;
/// `processed` is their sum and leaves out the games a Stop left. `errors`
/// holds the first 8 "title: reason" lines. [`BulkProgress::tally`] counts
/// the results the newer way.
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
    /// The job this is about (1, 2, ... per session; 0 = none yet).
    #[serde(default)]
    pub job: u64,
    #[serde(default)]
    pub kind: crate::job::JobKind,
    #[serde(default)]
    pub mode: Option<crate::job::UpdateMode>,
    /// When the job started and finished (Unix ms; 0 = not yet).
    #[serde(default)]
    pub started_at_ms: i64,
    #[serde(default)]
    pub finished_at_ms: i64,
    /// One per game done so far, in the job's order; a stopped job ends
    /// with the games it left ([`crate::job::SkipReason::Stopped`]). Never
    /// capped: a result is a few hundred bytes.
    #[serde(skip)]
    pub results: Vec<std::sync::Arc<crate::job::GameResult>>,
    /// The jobs waiting, first to run first.
    #[serde(default)]
    pub queued: Vec<crate::job::QueuedJob>,
    /// The last finished or stopped job can be undone
    /// (`MetadataService::undo_bulk_metadata`).
    #[serde(default)]
    pub can_undo: bool,
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

/// `CollectionArtKind`: a picture of a game. `Cover` is the tall cover;
/// the other cover families ([`CoverFamily`]) have their own kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtKind {
    Cover,
    Hero,
    #[serde(rename = "coverSquare")]
    CoverSquare,
    #[serde(rename = "coverWide")]
    CoverWide,
}

impl ArtKind {
    /// Every kind, the tall cover and the background first.
    pub const ALL: [ArtKind; 4] = [Self::Cover, Self::Hero, Self::CoverSquare, Self::CoverWide];

    /// Its field id (`cover`, `hero`, `coverSquare`, `coverWide`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hero => "hero",
            cover => cover.family().expect("a cover").field(),
        }
    }

    /// The kind of a field id (`cover`, `hero`, `coverSquare`, `coverWide`).
    pub fn from_field(field: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == field)
    }

    /// The cover of `family`.
    pub fn cover(family: CoverFamily) -> Self {
        match family {
            CoverFamily::Tall => Self::Cover,
            CoverFamily::Square => Self::CoverSquare,
            CoverFamily::Wide => Self::CoverWide,
        }
    }

    /// The family of a cover; `None` for the background.
    pub fn family(self) -> Option<CoverFamily> {
        match self {
            Self::Cover => Some(CoverFamily::Tall),
            Self::CoverSquare => Some(CoverFamily::Square),
            Self::CoverWide => Some(CoverFamily::Wide),
            Self::Hero => None,
        }
    }

    /// The start of its file names in `metadata-art/<game>/` and
    /// `art/<game>/` (`cover.jpg`, `cover-square.jpg`, `hero-<ms>.png`).
    pub fn file_stem(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Hero => "hero",
            Self::CoverSquare => "cover-square",
            Self::CoverWide => "cover-wide",
        }
    }

    /// The address of this picture in a record.
    pub fn of_record(self, md: &CollectionGameMetadata) -> Option<&str> {
        match self {
            Self::Cover => md.cover_url.as_deref(),
            Self::Hero => md.hero_url.as_deref(),
            Self::CoverSquare => md.cover_square_url.as_deref(),
            Self::CoverWide => md.cover_wide_url.as_deref(),
        }
    }

    pub fn of_record_mut(self, md: &mut CollectionGameMetadata) -> &mut Option<String> {
        match self {
            Self::Cover => &mut md.cover_url,
            Self::Hero => &mut md.hero_url,
            Self::CoverSquare => &mut md.cover_square_url,
            Self::CoverWide => &mut md.cover_wide_url,
        }
    }

    /// The address of this picture in an `art.json` entry.
    pub fn of_art(self, art: &CollectionGameArt) -> Option<&str> {
        match self {
            Self::Cover => art.cover_url.as_deref(),
            Self::Hero => art.hero_url.as_deref(),
            Self::CoverSquare => art.cover_square_url.as_deref(),
            Self::CoverWide => art.cover_wide_url.as_deref(),
        }
    }

    pub fn of_art_mut(self, art: &mut CollectionGameArt) -> &mut Option<String> {
        match self {
            Self::Cover => &mut art.cover_url,
            Self::Hero => &mut art.hero_url,
            Self::CoverSquare => &mut art.cover_square_url,
            Self::CoverWide => &mut art.cover_wide_url,
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

/// A picture a source has by region (LaunchBox's box art): the region as
/// the source names it (`North America`; `""` for none) and the picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionImage {
    pub region: String,
    pub image: WebImage,
}

/// The art picker's search results, by where they come from. `launchbox`:
/// only the Covers dialog asks for them (they are of one game).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtImages {
    pub launchbox: Vec<RegionImage>,
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
    fn art_kinds_name_their_family_field_and_files() {
        for family in CoverFamily::ALL {
            let kind = ArtKind::cover(family);
            assert_eq!((kind.family(), kind.as_str()), (Some(family), family.field()));
        }
        assert_eq!(ArtKind::Hero.family(), None);
        let stems: Vec<&str> = ArtKind::ALL.iter().map(|k| k.file_stem()).collect();
        assert_eq!(stems, ["cover", "hero", "cover-square", "cover-wide"]);
        assert_eq!(serde_json::to_value(ArtKind::Cover).unwrap(), "cover");
        assert_eq!(serde_json::to_value(ArtKind::CoverWide).unwrap(), "coverWide");
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
