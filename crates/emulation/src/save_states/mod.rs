//! Save states per emulator engine (port of `emulation/save-states.ts`,
//! `save-state-engines.ts`): where the files live, how a ROM maps to the file
//! names, thumbnails as raw image bytes, and the one-shot launch arguments
//! that boot into a chosen state.
//!
//! Wiring into launching (the launch crate builds plans, this crate only
//! produces data): take the [`LaunchStateArgs`] returned by
//! [`PendingLaunchStates::take`] and call [`LaunchStateArgs::apply`] on the
//! emulator argv (before the Flatpak `run <id>` wrapper), skipping Wine
//! emulators, exactly where `tryLaunchEmulatedGame` does.

mod engine;
mod layouts;
mod listing;
mod pending;
mod thumbnails;

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

pub use engine::{engine_for, engine_id_for, Engine, EngineContext};
pub use listing::{list_emulator_save_states, remember_save_state_key};
pub use pending::{set_pending_launch_state, LaunchStateArgs, PendingLaunchStates, PENDING_TTL};
pub use thumbnails::{thumbnail_for, Thumbnail};

pub const MAX_STATES: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveStateKind {
    Slot,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedState {
    pub key: String,
    pub kind: SaveStateKind,
    /// Slot number as the emulator's own menu shows it.
    pub slot: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorSaveState {
    pub path: String,
    pub kind: SaveStateKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// ISO 8601 UTC, like `Date#toISOString`.
    pub modified_at: String,
    #[serde(skip)]
    pub thumbnail: Option<Arc<Thumbnail>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorSaveStateList {
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emulator_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rom_path: Option<String>,
    pub states: Vec<EmulatorSaveState>,
}

/// Non-empty values without repeats, in first-seen order.
fn unique(values: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values.into_iter().flatten() {
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
