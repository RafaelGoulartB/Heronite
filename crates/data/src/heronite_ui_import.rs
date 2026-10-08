//! One-time import of Heronite's Collection UI state, which lives in
//! Chromium's localStorage (LevelDB under `<heroic>/Local Storage/leveldb`).
//!
//! Keys look like `_file://\0\x01collection_install_filter`; values start
//! with an encoding byte (`0x01` Latin-1, `0x00` UTF-16LE). The DB is copied
//! to a temp dir first because a running Heronite holds its lock.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::ui_state::UiState;
use crate::HeroicPaths;

const KEYS: [&str; 6] = [
    "collection_install_filter",
    "collection_facet_filter",
    "collection_sort",
    "collection_sidebar_hidden",
    "collection_collapsed_groups",
    "collection_focused_game",
];

fn decode_value(raw: &[u8]) -> Option<String> {
    let (tag, rest) = raw.split_first()?;
    match tag {
        1 => Some(rest.iter().map(|&b| b as char).collect()),
        0 => {
            let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16(&units).ok()
        }
        _ => None,
    }
}

fn key_name(raw: &[u8]) -> Option<&str> {
    // `_<origin>\0\x01<key>`
    let pos = raw.windows(2).position(|w| w == [0, 1])?;
    std::str::from_utf8(&raw[pos + 2..]).ok()
}

pub fn read_local_storage(leveldb_dir: &Path) -> Result<HashMap<String, String>> {
    let tmp = tempfile::tempdir()?;
    for entry in std::fs::read_dir(leveldb_dir).with_context(|| format!("reading {}", leveldb_dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "LOCK" {
            continue;
        }
        std::fs::copy(entry.path(), tmp.path().join(&name))?;
    }
    let mut opts = rusty_leveldb::Options::default();
    opts.create_if_missing = false;
    let mut db = rusty_leveldb::DB::open(tmp.path(), opts).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut out = HashMap::new();
    let mut it = db.new_iter().map_err(|e| anyhow::anyhow!("{e}"))?;
    use rusty_leveldb::LdbIterator;
    while let Some((k, v)) = it.next() {
        if let Some(name) = key_name(&k) {
            if KEYS.contains(&name) {
                if let Some(value) = decode_value(&v) {
                    out.insert(name.to_string(), value);
                }
            }
        }
    }
    Ok(out)
}

/// Builds a [`UiState`] from Heronite's localStorage, or `None` when there is
/// nothing to import.
pub fn import(paths: &HeroicPaths) -> Option<UiState> {
    let dir = paths.root.join("Local Storage").join("leveldb");
    let map = match read_local_storage(&dir) {
        Ok(m) if !m.is_empty() => m,
        Ok(_) => return None,
        Err(err) => {
            log::warn!("importing Heronite UI state: {err:#}");
            return None;
        }
    };
    let parse = |k: &str| map.get(k).and_then(|s| serde_json::from_str::<Value>(s).ok());
    let doc = json!({
        "installFilter": map.get("collection_install_filter"),
        "sort": map.get("collection_sort"),
        "facetFilters": parse("collection_facet_filter"),
        "sidebarHidden": map.get("collection_sidebar_hidden").map(|v| v == "true"),
        "collapsedGroups": parse("collection_collapsed_groups"),
        "focusedGame": map.get("collection_focused_game"),
    });
    Some(UiState::from_value(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_values_and_keys() {
        assert_eq!(decode_value(b"\x01installed").as_deref(), Some("installed"));
        assert_eq!(decode_value(b"\x00h\x00i\x00").as_deref(), Some("hi"));
        assert_eq!(key_name(b"_file://\x00\x01collection_sort"), Some("collection_sort"));
    }

    #[test]
    fn imports_real_heronite_state_when_available() {
        let Some(home) = dirs::config_dir() else { return };
        let paths = HeroicPaths::at(home.join("heroic"));
        if let Some(state) = import(&paths) {
            eprintln!("{:?}", state.to_value());
        }
    }
}
