//! Playnite import / merge / export, compatible with Heronite's data files.

pub mod litedb;
pub mod mapper;
pub mod reader;
pub mod remap;
pub mod store_ops;
pub mod types;
pub mod metadata_seed;
pub mod user_emulators;
pub mod game_config;
pub mod covers;
pub mod export;
pub mod import;

pub use covers::{CoverFetcher, Covers, NoCovers, SteamCdnCovers};
pub use export::{build_export, export_playnite_library};
pub use import::Playnite;
pub use reader::{load_playnite_library, PlayniteLibraryDump};
pub use remap::DriveRemap;
pub use types::*;

#[cfg(test)]
mod synthetic_tests;
