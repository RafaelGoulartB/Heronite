//! Mods of the Collection's games (`docs/MODS.md`): each mod in a folder of
//! its own, the load order in `mods.json`. On Linux `fuse-overlayfs` layers
//! the enabled mods over the game's folder while it runs, so the game's own
//! files are never written; on Windows they are linked or copied into it
//! with a journal and taken out when it ends.
//!
//! * [`store`] reads and writes `mods.json`; [`games`] knows how popular
//!   modded games lay out their mods (Bethesda's `Data`, BepInEx, …).
//! * [`install`] turns an archive or a folder into a mod ([`layout`] finds
//!   where its files go; [`fomod`] runs FOMOD installers).
//! * [`conflicts`] says which mod wins each file; [`plugins`] writes
//!   Bethesda's `Plugins.txt`; [`support`] says what to expect of a game's mods.
//! * [`deploy`] mounts and unmounts the layers on Linux ([`tool`] finds or
//!   downloads `fuse-overlayfs`), [`journal_deploy`] puts the files in place
//!   on Windows, [`deployment`] answers the app for both; [`apply`] is what a
//!   launch calls.
//! * [`nexus`] and [`thunderstore`] talk to the sites, [`download`] fetches
//!   files, [`links`] matches Collection games to the sites' games.
//!
//! Every function here blocks; the app calls them from worker threads.

pub mod apply;
mod bethesda_ini;
pub mod conflicts;
#[cfg(not(windows))]
pub mod deploy;
pub mod deployment;
pub mod download;
mod file_swap;
pub mod fomod;
mod fsutil;
pub mod games;
pub mod http;
pub mod install;
pub mod journal_deploy;
pub mod layout;
pub mod links;
pub mod loaders;
pub mod model;
pub mod nexus;
pub mod overwrite;
pub mod paths;
pub mod plugins;
pub mod store;
pub mod support;
pub mod thunderstore;
pub mod tool;

pub use model::*;
pub use paths::{GameDirs, ModsRoot};
