//! The Epic Games Store, natively: what legendary did for Heronite, in Rust.
//! Everything here is blocking (call it from worker threads) and knows no
//! GPUI.
//!
//! It reads and writes legendary's own files in `legendaryConfig/legendary`
//! (`user.json`, `assets.json`, `metadata/<app>.json`, `installed.json`,
//! `manifests/*.manifest`, `overlay_install.json`) in legendary's shapes, so
//! the rest of the app and every install made with legendary keep working.
//!
//! * [`auth`]: the sign-in (`user.json`), renewing it, the codes a game
//!   signs in with.
//! * [`api`]: the launcher, catalog, library and download services.
//! * [`library`]: refreshing the account's games (`legendary list`).
//! * [`manifest`], [`chunk`]: build manifests and their chunks; [`manifest::plan`]
//!   is the work of an install, update or repair.
//! * [`info`]: the install information of `legendary info --json`.
//! * [`installed`]: `installed.json` and the stored manifests.
//! * [`launch`]: how a game starts (`legendary launch --json`).
//! * [`saves`]: cloud saves (`legendary sync-saves`).
//! * [`eos`]: the EOS overlay.
//! * [`legendary_config`]: what legendary's API publishes (launcher version,
//!   per-game program overrides).
//!
//! Downloading the files of a plan is the caller's: [`api`] fetches and
//! [`chunk`] decodes each chunk.

pub mod api;
pub mod auth;
mod bin_io;
pub mod chunk;
pub mod client;
pub mod crypt;
pub mod eos;
pub mod files;
pub mod http;
pub mod info;
pub mod installed;
pub mod launch;
pub mod legendary_config;
pub mod library;
pub mod manifest;
pub mod saves;
pub mod wine_reg;

pub use auth::{is_sign_in_refused, Session, SignInRefused};
pub use client::{Client, Endpoints};
pub use manifest::Manifest;
