//! What this PC remembers between syncs (`local_library/sync-ledger.json`):
//! each field's value as last published or taken in, with its stamp, the
//! sessions taken in from other PCs, and the play time last published.
//!
//! The data files keep Heroic's formats, which have no time of change per
//! field: a field whose value differs from the one here was changed on this
//! PC since the last sync, and gets a new stamp then.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use heronite_data::{json_store, DataPaths};
use serde::{Deserialize, Serialize};

use crate::format::{Stamp, Stamped};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    /// The logical clock: past every clock read from the other PCs.
    #[serde(default)]
    pub clock: u64,
    /// By Collection key.
    #[serde(default)]
    pub games: BTreeMap<String, LedgerGame>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerGame {
    #[serde(default)]
    pub runner: String,
    #[serde(default)]
    pub app_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Stamped>,
    /// `startedAt` of the sessions taken in from other PCs: never published
    /// as this PC's, and never taken in again once deleted here.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub imported: BTreeSet<String>,
    /// The total published by the last sync, to tell one made smaller by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_minutes: Option<u64>,
    /// The newest total made smaller by hand that this PC knows of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowered: Option<Stamp>,
}

impl LedgerGame {
    /// Whether there is anything to remember.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.imported.is_empty() && self.published_minutes.unwrap_or(0) == 0 && self.lowered.is_none()
    }
}

pub fn load(paths: &DataPaths) -> Result<Ledger> {
    json_store::read(&paths.sync_ledger())
}

pub fn save(paths: &DataPaths, ledger: &Ledger) -> Result<()> {
    json_store::write(&paths.sync_ledger(), ledger)
}
