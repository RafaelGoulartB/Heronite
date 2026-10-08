//! Wine/Proton manager and compatibility layer detection (port of Heroic's
//! `backend/wine/manager/**` and `backend/utils/compatibility_layers.ts`).
//!
//! STUB: the public items below are the contract other code builds against;
//! the implementation is in progress.

use heronite_data::HeroicPaths;
use serde::{Deserialize, Serialize};

/// Heroic's `WineInstallation` (`common/types.ts`), the value stored as
/// `wineVersion` in `config.json` / `GamesConfig/<app>.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WineInstallation {
    pub bin: String,
    pub name: String,
    /// `wine` | `proton` | `crossover` | `toolkit`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lib32: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wineserver: Option<String>,
}

/// Every Wine/Proton the game settings may pick (Heroic's `getWineList` on
/// Linux), in Heroic's order. Blocking (scans folders): call off the UI thread.
pub fn list_installations(_heroic: &HeroicPaths) -> Vec<WineInstallation> {
    Vec::new()
}
