//! The Epic Games Store, natively: what legendary did for Heronite, in Rust.
//! Everything here is blocking (call it from worker threads) and knows no
//! GPUI.
//!
//! It reads and writes legendary's own files in `legendaryConfig/legendary`
//! in legendary's shapes, so the rest of the app and every install made
//! with legendary keep working.
//!
//! * [`auth`]: the sign-in (`user.json`), renewing it, the codes a game
//!   signs in with.
//! * [`api`]: the launcher, catalog, library and download services.
//! * [`manifest`], [`chunk`]: build manifests and their chunks; [`manifest::plan`]
//!   is the work of an install, update or repair.
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
pub mod files;
pub mod http;
pub mod legendary_config;
pub mod manifest;

pub use auth::{is_sign_in_refused, Session, SignInRefused};
pub use client::{Client, Endpoints};
pub use manifest::Manifest;
