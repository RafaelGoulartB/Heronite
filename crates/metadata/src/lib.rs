//! Collection metadata and artwork backend: the registry of sources (IGDB,
//! Steam, Lutris, the LaunchBox Games Database, store and Playnite
//! details, plus pictures (SteamGridDB, libretro-thumbnails) and extras; `platforms` and `rom_names` for emulated games), when
//! a match is sure enough to save (`confidence`) and the review queue of the
//! others (`review`), the per-field merge, details editing, custom art,
//! Steam details/hero caches, covers, web image search and HowLongToBeat.
//!
//! Port of Heronite's `src/local-library/{metadata,art,heroes,covers,
//! steam-details,web-images}`
//! (<https://github.com/RafaelGoulartB/HeroicGamesLauncher>) and the HLTB part of
//! `src/backend/wiki_game_info`. Every write goes through
//! `heronite_data::json_store` (or an atomic file write for images and the
//! Steam details cache) into the data folder, in the formats Heronite used.
//!
//! Everything is blocking (`ureq`); call from worker threads.

pub mod art;
pub mod confidence;
pub mod covers;
pub mod edit;
pub mod hltb;
pub mod http;
pub mod igdb;
pub mod images;
pub mod job;
pub mod launchbox;
pub mod libretro;
pub mod lutris;
pub mod merge;
pub mod picture_size;
pub mod platforms;
pub mod protondb;
pub mod providers;
pub mod record;
pub mod remove;
pub mod review;
pub mod rom_names;
pub mod service;
pub mod sources;
pub mod steam;
pub mod steam_tags;
pub mod store;
pub mod types;
pub mod util;
pub mod web_images;

pub use http::{Endpoints, Http};
pub use job::*;
pub use providers::{PlayniteGame, PlayniteSource};
pub use record::SourceMatch;
pub use review::{MatchChoice, MatchPick, ReviewEntry, ReviewItem, ReviewReason};
pub use service::MetadataService;
pub use sources::{MetadataSource, SourceContext, SourceDescriptor, SourceGame, SourceKind, SourceRegistry};
pub use types::*;
