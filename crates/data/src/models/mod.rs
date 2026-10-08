//! Typed views of the JSON files shared with Heronite.
//!
//! Conventions (keep them when adding fields):
//! - `#[serde(rename_all = "camelCase")]`, matching the TypeScript types in
//!   `HeroicGamesLauncher/src/common/types/local-library.ts`.
//! - Optional fields are `Option<_>` with `skip_serializing_if`, so a field
//!   absent on disk stays absent after a write.
//! - Every struct ends with `#[serde(flatten)] extra`, so fields this app
//!   does not model survive a read-modify-write.
//! - Numbers use [`JsNum`] so integers are written without `.0`.

mod heroic;
mod overlay;
mod settings;

pub use heroic::*;
pub use overlay::*;
pub use settings::*;

use serde_json::{Map, Value};

pub type Extra = Map<String, Value>;
