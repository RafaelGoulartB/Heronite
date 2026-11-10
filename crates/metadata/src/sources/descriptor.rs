//! What a source says about itself, as plain data: the settings, the bulk
//! update and the per-game dialog are drawn from it without running the
//! source, and an extension's manifest carries the same shape (camelCase
//! JSON; every key but `id`, `name` and `kind` may be left out).

use heronite_data::models::LocalGameMeta;
use serde::{Deserialize, Serialize};

use super::options::SourceOption;
use crate::types::{MatchedBy, MetadataApplyArgs};

/// [`SourceDescriptor::provides`] ids beyond `heronite_data::settings::METADATA_FIELDS`
/// (whose `cover` and `hero` pictures sources name too). The list is open:
/// any string is a valid id.
pub mod extra_fields {
    /// HowLongToBeat's main / extras / completionist times.
    pub const TIME_TO_BEAT: &str = "timeToBeat";
    pub const STEAM_TAGS: &str = "steamTags";
    /// ProtonDB's tier.
    pub const LINUX_RATING: &str = "linuxRating";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// Fills the record's fields (description, developers, ...) through the merge.
    Details,
    /// Covers and backgrounds.
    Pictures,
    /// Facts kept beside the record (time to beat, tags, Linux rating).
    Extras,
}

/// What a source needs before it can be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Needs {
    #[default]
    Nothing,
    /// A key or client id/secret the user types in.
    ApiKey,
    /// Signing in to an account.
    Account,
    /// A database downloaded once before the first lookup.
    Download,
}

/// The games a source is for. A source is not asked about other games.
///
/// JSON: `"all"`, `"hasSteamId"`, `"emulated"`, `{"stores": ["gog"]}`,
/// `{"platforms": ["snes"]}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppliesTo {
    #[default]
    All,
    /// Games with a Steam app id (their own or the overlay's).
    HasSteamId,
    /// Games whose runner (`legendary`, `gog`, `nile`, `sideload`) or overlay
    /// store (`steam`, `epic`, `xbox`, ...) is in the list.
    Stores(Vec<String>),
    /// Games launched through an emulator.
    Emulated,
    /// Emulated games of these platform ids (the overlay's `platformId`).
    Platforms(Vec<String>),
}

impl AppliesTo {
    pub fn matches(&self, game: &SourceGame) -> bool {
        match self {
            Self::All => true,
            Self::HasSteamId => game.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()),
            Self::Stores(stores) => stores.iter().any(|s| *s == game.runner || game.store.as_deref() == Some(s.as_str())),
            Self::Emulated => game.emulated,
            Self::Platforms(platforms) => game.platform.as_deref().is_some_and(|p| platforms.iter().any(|x| x == p)),
        }
    }

    /// Whether answering needs the overlay entry (`local_library/library.json`)
    /// beyond the Steam id.
    pub(crate) fn needs_overlay(&self) -> bool {
        matches!(self, Self::Stores(_) | Self::Emulated | Self::Platforms(_))
    }
}

/// A metadata source as data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDescriptor {
    /// Stable id, the one saved in `fieldSources` and `fieldPriority`.
    pub id: String,
    /// Shown as is when it is a proper noun ("IGDB"); otherwise an English
    /// `tr!` key the interface translates ("Store").
    pub name: String,
    pub kind: SourceKind,
    /// Field ids it can fill: `METADATA_FIELDS` entries and [`extra_fields`].
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub applies_to: AppliesTo,
    /// The ways it can find a game, best first. Informational.
    #[serde(default)]
    pub finds_by: Vec<MatchedBy>,
    #[serde(default)]
    pub needs: Needs,
    /// Asks a service on the internet. When every such source that was asked
    /// is unreachable, a fetch counts as offline and nothing is saved.
    #[serde(default)]
    pub network: bool,
    /// Kept in saved priorities and records, but not offered as a choice.
    #[serde(default)]
    pub hidden: bool,
    /// Asked only when the user searches by hand (the art picker), never by
    /// a fill or an update.
    #[serde(default)]
    pub manual_only: bool,
    /// Operating systems it works on (`std::env::consts::OS`: `windows`,
    /// `linux`, `macos`); empty = all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only_on: Vec<String>,
    /// What the user can set for it, in the order shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SourceOption>,
}

impl SourceDescriptor {
    pub fn provides(&self, field: &str) -> bool {
        self.provides.iter().any(|f| f == field)
    }

    /// Whether it works on this operating system.
    pub fn available_here(&self) -> bool {
        self.only_on.is_empty() || self.only_on.iter().any(|os| os == std::env::consts::OS)
    }
}

/// A game as sources see it: the apply arguments plus what the overlay
/// knows about it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceGame {
    pub app_name: String,
    pub runner: String,
    pub title: String,
    /// The argument's, else the overlay's. `None` = not on Steam as far as we know.
    pub steam_app_id: Option<String>,
    /// IGDB id picked by the user or matched before.
    pub igdb_id: Option<i64>,
    pub lutris_slug: Option<String>,
    /// Heroic language code (`pt-BR`) for sources with localized text.
    pub language: Option<String>,
    /// The overlay's `source` (`steam`, `epic`, `emulator`, ...).
    pub store: Option<String>,
    pub emulated: bool,
    /// The overlay's `platformId` of an emulated game.
    pub platform: Option<String>,
}

impl SourceGame {
    /// `overlay` is the game's entry in `local_library/library.json`, when read.
    pub fn new(args: &MetadataApplyArgs, overlay: Option<&LocalGameMeta>) -> Self {
        let non_empty = |s: Option<&String>| s.filter(|s| !s.is_empty()).cloned();
        Self {
            app_name: args.app_name.clone(),
            runner: args.runner.clone(),
            title: args.title.clone(),
            steam_app_id: non_empty(args.steam_app_id.as_ref()).or_else(|| non_empty(overlay.and_then(|m| m.steam_app_id.as_ref()))),
            igdb_id: args.igdb_id,
            lutris_slug: args.lutris_slug.clone(),
            language: args.language.clone(),
            store: overlay.map(|m| m.source.clone()).filter(|s| !s.is_empty()),
            emulated: overlay.is_some_and(|m| m.launch_kind == "emulator"),
            platform: non_empty(overlay.and_then(|m| m.platform_id.as_ref())),
        }
    }
}
