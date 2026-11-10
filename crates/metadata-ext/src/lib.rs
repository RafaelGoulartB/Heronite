//! Metadata sources the community writes: an extension is a folder (shared
//! as a zip of it) with a manifest, `source.toml`, and a script,
//! `source.js`, run in a sandbox. The manifest carries everything a screen
//! shows ([`manifest::Manifest::descriptor`]), so no script runs to draw a
//! row; the script finds a game and reads its record ([`source::ScriptSource`]).
//!
//! - [`manifest`]: parsing and checking `source.toml`.
//! - [`package`]: reading a folder or a zip safely.
//! - [`store`]: what is installed under the data folder.
//! - [`engine`]: the sandbox (QuickJS) and the `ctx` API scripts get.
//! - [`record`]: what a script returns, checked and clamped like the
//!   built-in sources' answers.
//! - [`ExtensionsExt`]: the entry points the app calls on the
//!   `MetadataService` (load, inspect, install, remove, list).
//!
//! `docs/EXTENSIONS.md` is the authors' reference.

pub mod engine;
mod error;
pub mod limits;
pub mod manifest;
pub mod net;
pub mod package;
pub mod record;
pub mod service;
pub mod source;
pub mod store;

pub use error::ExtensionError;
pub use limits::Limits;
pub use service::{ExtensionState, ExtensionsExt, InstallAction, InstallOutcome, InstalledExtension, PackageInfo};
