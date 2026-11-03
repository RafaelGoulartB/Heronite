//! The Epic Games Store, natively: what legendary did for Heronite, in Rust.
//! Everything here is blocking (call it from worker threads) and knows no
//! GPUI.
//!
//! * [`manifest`], [`chunk`]: build manifests and their chunks; [`manifest::plan`]
//!   is the work of an install, update or repair.

mod bin_io;
pub mod chunk;
pub mod crypt;
pub mod manifest;

pub use manifest::Manifest;
