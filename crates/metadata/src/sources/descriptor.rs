//! What a source says about itself, as plain data: the settings, the bulk
//! update and the per-game dialog are drawn from it without running the
//! source, and an extension's manifest carries the same shape (camelCase
//! JSON; every key but `id`, `name` and `kind` may be left out).

use std::collections::BTreeMap;

use heronite_data::models::LocalGameMeta;
use serde::{Deserialize, Serialize};

use super::download::SourceDownload;
use super::options::SourceOption;
use crate::types::{MatchedBy, MetadataApplyArgs};

/// [`SourceDescriptor::provides`] ids beyond `heronite_data::settings::ALL_FIELDS`
/// (whose `cover`, `hero`, `coverSquare` and `coverWide` pictures sources
/// name too). The list is open: any string is a valid id.
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
/// JSON: `"all"`, `"hasSteamId"`, `"emulated"`, `"pc"`, `{"stores": ["gog"]}`,
/// `{"platforms": ["nintendo_super_nes"]}`.
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
    /// Emulated games of these platform ids ([`SourceGame::platform`],
    /// `crate::platforms`).
    Platforms(Vec<String>),
    /// Games that are not emulated.
    Pc,
}

impl AppliesTo {
    pub fn matches(&self, game: &SourceGame) -> bool {
        match self {
            Self::All => true,
            Self::HasSteamId => game.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()),
            Self::Stores(stores) => stores.iter().any(|s| *s == game.runner || game.store.as_deref() == Some(s.as_str())),
            Self::Emulated => game.emulated,
            Self::Platforms(platforms) => game.platform.as_deref().is_some_and(|p| platforms.iter().any(|x| x == p)),
            Self::Pc => !game.emulated,
        }
    }

    /// Whether answering needs the overlay entry (`local_library/library.json`)
    /// beyond the Steam id.
    pub(crate) fn needs_overlay(&self) -> bool {
        matches!(self, Self::Stores(_) | Self::Emulated | Self::Platforms(_) | Self::Pc)
    }
}

/// When a source is asked about a game of its audience.
///
/// JSON: `"always"` (the default) or `"asFallback"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Asked {
    #[default]
    Always,
    /// Only when the game is already identified with it (a pinned or
    /// stored match), when the game is of its kind ([`GameKind`]: the
    /// record, the Steam tags or another source say so), when no other
    /// online details source found the game for sure, or when the user
    /// names it (a search of it, a pin, a job of only such sources).
    AsFallback,
}

/// The kind of game a source is about (visual novels), and how a game
/// says it is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameKind {
    /// English `tr!` key naming the games ("Visual novels"); Settings shows
    /// it as the games the source is for.
    pub name: String,
    /// A genre, theme or Steam tag with one of these names (compared
    /// normalized) makes a game of this kind.
    #[serde(default)]
    pub signals: Vec<String>,
}

impl GameKind {
    /// Whether one of `names` is a signal.
    pub fn said_by<'a>(&self, names: impl IntoIterator<Item = &'a String>) -> bool {
        let signals: Vec<String> = self.signals.iter().map(|s| crate::util::normalize_title(s)).collect();
        names.into_iter().any(|n| signals.contains(&crate::util::normalize_title(n)))
    }
}

/// More games a source is for while one of its switch options is on (the
/// LaunchBox Games Database's "Also for PC games").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlsoFor {
    /// The key of a [`Switch`](super::OptionKind::Switch) option.
    pub option: String,
    pub applies_to: AppliesTo,
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
    /// Field ids it can fill: `ALL_FIELDS` entries (`cover` is the tall
    /// cover; `coverSquare` and `coverWide` the other families) and
    /// [`extra_fields`].
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
    /// Can be searched by name, and asked for one of its games by id: the
    /// user can pick (pin) which of its games a game is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub searchable: bool,
    /// Operating systems it works on (`std::env::consts::OS`: `windows`,
    /// `linux`, `macos`); empty = all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only_on: Vec<String>,
    /// What the user can set for it, in the order shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SourceOption>,
    #[serde(default, skip_serializing_if = "is_always")]
    pub asked: Asked,
    /// The kind of game it is about, when it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_kind: Option<GameKind>,
    /// English `tr!` key: a line on what the source is, shown in its open
    /// row in Settings. An extension's is its own text, shown as is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// The database it downloads before it can be asked
    /// ([`Needs::Download`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<SourceDownload>,
    /// More games it is for while an option says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub also_for: Option<AlsoFor>,
    /// The games for which, when it finds one surely, it goes ahead of the
    /// sources that found that game only by its title, in every field
    /// whose order the user did not set (`crate::sources::order`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leads_for: Option<AppliesTo>,
    /// Set for a source installed as an extension; `None` for a built-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionInfo>,
}

/// What an extension's manifest says about the extension itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionInfo {
    /// Semantic version (`1.2.0`).
    pub version: String,
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// It is about adult games: every picture it gives counts as adult
    /// (dropped unless the user allows adult covers).
    #[serde(default)]
    pub adult: bool,
}

fn is_always(a: &Asked) -> bool {
    *a == Asked::Always
}

impl SourceDescriptor {
    pub fn provides(&self, field: &str) -> bool {
        self.provides.iter().any(|f| f == field)
    }

    /// Whether it is asked only as a fallback ([`Asked::AsFallback`]).
    pub fn is_fallback(&self) -> bool {
        self.asked == Asked::AsFallback
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
    /// The app's platform id of an emulated game (`nintendo_super_nes`):
    /// the overlay's `platformId`, else what the ROM, the emulator, the
    /// folders or the record say ([`crate::platforms::resolve`]). Each
    /// database's names for it: [`crate::platforms::by_id`].
    pub platform: Option<String>,
    /// The file an emulated game's dump is named by
    /// (`Super Metroid (Japan, USA) (En,Ja).sfc`), without its folder
    /// ([`crate::platforms::rom_file`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rom_file: Option<String>,
    /// Hashes of the ROM the app already keeps, by kind (`crc32`, `md5`,
    /// `sha1`, lower-case hex). A lookup never hashes a file; the app keeps
    /// none yet, so this is empty until it does.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rom_hashes: BTreeMap<String, String>,
    /// The release year the record has, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    /// The name (never the path) the game's files go by: its ROM's
    /// ([`rom_file`](Self::rom_file)) for an emulated game, else its
    /// install folder's, when the overlay has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// The fields asked of this source (`None`: every field it gives). Set
    /// for pictures sources, which fetch one picture per field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

impl SourceGame {
    /// `overlay` is the game's entry in `local_library/library.json`, when
    /// read; `record_platforms` the platforms its metadata names (the last
    /// thing an emulated game's platform is told by).
    pub fn new(args: &MetadataApplyArgs, overlay: Option<&LocalGameMeta>, record_platforms: &[String]) -> Self {
        let non_empty = |s: Option<&String>| s.filter(|s| !s.is_empty()).cloned();
        let emulated = overlay.is_some_and(|m| m.launch_kind == "emulator" || m.source == "emulator");
        let emulation = overlay.filter(|_| emulated);
        let rom_file = emulation.and_then(crate::platforms::rom_file);
        Self {
            app_name: args.app_name.clone(),
            runner: args.runner.clone(),
            title: args.title.clone(),
            steam_app_id: non_empty(args.steam_app_id.as_ref()).or_else(|| non_empty(overlay.and_then(|m| m.steam_app_id.as_ref()))),
            igdb_id: args.igdb_id,
            lutris_slug: args.lutris_slug.clone(),
            language: args.language.clone(),
            store: overlay.map(|m| m.source.clone()).filter(|s| !s.is_empty()),
            emulated,
            platform: emulation
                .and_then(|m| crate::platforms::resolve(m, record_platforms))
                .map(|p| p.id.to_owned())
                .or_else(|| non_empty(emulation.and_then(|m| m.platform_id.as_ref())).filter(|p| p != "other")),
            year: None,
            file_name: rom_file.clone().or_else(|| overlay.and_then(folder_name)),
            rom_file,
            rom_hashes: BTreeMap::new(),
            fields: None,
        }
    }
}

/// The last part of the game's install folder.
fn folder_name(m: &LocalGameMeta) -> Option<String> {
    let path = m.windows_install_directory.as_deref()?;
    let name = path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}
