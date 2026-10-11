//! Reads of the Epic library file (`GameInfo` entries and the counts).

use std::collections::HashMap;
use std::sync::Arc;

use heronite_data::json_store;
use heronite_data::DataPaths;
use serde_json::Value;

use super::{GameFlags, Stats};

/// The library file's `GameInfo` of one game.
pub fn library_entry(data_paths: &DataPaths, app_name: &str) -> Option<Value> {
    let doc = json_store::read_value(&data_paths.legendary_library()).ok()?;
    doc.get("library")?
        .as_array()?
        .iter()
        .find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name))
        .cloned()
}

/// Owned (without DLC) and installed counts from the library file.
pub(super) fn read_stats(data_paths: &DataPaths) -> Stats {
    let doc = json_store::read_value(&data_paths.legendary_library()).unwrap_or(Value::Null);
    let list = doc.get("library").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut stats = Stats::default();
    let mut flags = HashMap::new();
    for g in &list {
        if let Some(app) = g.get("app_name").and_then(Value::as_str) {
            let has_dlcs = g.get("dlcList").and_then(Value::as_array).is_some_and(|l| !l.is_empty());
            let third_party = g.get("thirdPartyManagedApp").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
            flags.insert(app.to_string(), GameFlags { has_dlcs, third_party });
        }
        let is_dlc = g.pointer("/install/is_dlc").and_then(Value::as_bool).unwrap_or(false);
        if is_dlc {
            continue;
        }
        stats.owned += 1;
        if g.get("is_installed").and_then(Value::as_bool).unwrap_or(false) {
            stats.installed += 1;
        }
    }
    stats.flags = Arc::new(flags);
    stats
}
