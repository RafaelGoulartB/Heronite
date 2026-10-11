//! Typed views of the JSON files in the data folder.
//!
//! Conventions (keep them when adding fields):
//! - `#[serde(rename_all = "camelCase")]`, matching the TypeScript types in
//!   Heronite's `src/common/types/local-library.ts`
//!   (<https://github.com/RafaelGoulartB/HeroicGamesLauncher>).
//! - Optional fields are `Option<_>` with `skip_serializing_if`, so a field
//!   absent on disk stays absent after a write.
//! - Every struct ends with `#[serde(flatten)] extra`, so fields this app
//!   does not model survive a read-modify-write.
//! - Numbers use [`JsNum`] so integers are written without `.0`.

mod overlay;
mod settings;
mod stores;

pub use overlay::*;
pub use settings::*;
pub use stores::*;

use serde_json::{Map, Value};

pub type Extra = Map<String, Value>;
