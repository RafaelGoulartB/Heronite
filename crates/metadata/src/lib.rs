//! Collection metadata and artwork backend: IGDB, Steam, Lutris, store and
//! Playnite providers, the per-field merge, details editing, custom art,
//! Steam details/hero caches, covers, web image search and HowLongToBeat.
//!
//! Port of `HeroicGamesLauncher/src/local-library/{metadata,art,heroes,covers,
//! steam-details,web-images}` and the HLTB part of
//! `src/backend/wiki_game_info`. Every write goes through
//! `heronite_data::json_store` (or an atomic file write for images and the
//! Steam details cache) into the same files Heronite uses, so both apps keep
//! working on one `~/.config/heroic`.
//!
//! Everything is blocking (`ureq`); call from worker threads.

pub mod art;
pub mod covers;
pub mod edit;
pub mod hltb;
pub mod http;
pub mod igdb;
pub mod images;
pub mod lutris;
pub mod merge;
pub mod providers;
pub mod service;
pub mod steam;
pub mod store;
pub mod types;
pub mod util;
pub mod web_images;

pub use http::{Endpoints, Http};
pub use providers::{PlayniteGame, PlayniteSource};
pub use service::MetadataService;
pub use types::*;
