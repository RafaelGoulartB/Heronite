//! `manifest.json` of a backup: what it holds and where it came from, so a
//! restore can check every file and point paths at the folders of this
//! machine.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::catalog::{Part, Side};

pub const MANIFEST_FILE: &str = "manifest.json";
/// Version of the layout below; a newer one is refused by restore.
pub const FORMAT: u32 = 2;

/// Why a backup was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    Scheduled,
    Manual,
    /// Made by a restore of what was there before it.
    BeforeRestore,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub side: Side,
    /// Relative to the side's folder, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Of the original file, milliseconds since the epoch.
    pub mtime_ms: i64,
    /// Hex SHA-256 of the content.
    pub sha256: String,
    pub part: Part,
}

/// A few numbers to tell backups apart in the list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub games: usize,
    pub sessions: usize,
    pub journal: usize,
    pub files: usize,
    pub bytes: u64,
    /// Bytes this backup copied; the rest are links to the previous one.
    pub new_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    pub app_version: String,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub reason: Reason,
    pub host: String,
    pub home: String,
    pub data_root: String,
    pub config_root: String,
    pub parts: Vec<Part>,
    pub summary: Summary,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn read(snapshot: &Path) -> Result<Manifest> {
        let path = snapshot.join(MANIFEST_FILE);
        let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write(&self, snapshot: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        fs::write(snapshot.join(MANIFEST_FILE), text)?;
        Ok(())
    }
}

pub(crate) fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_default()
}

/// Counts of the Collection files in `data` (a data folder or a backup's
/// `data/`); missing or unreadable files count 0.
pub(crate) fn summarize(data: &Path) -> Summary {
    let count = |rel: &str, key: &str| -> usize {
        let Ok(text) = fs::read_to_string(data.join(rel)) else { return 0 };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return 0 };
        match &v[key] {
            serde_json::Value::Object(m) => m.len(),
            serde_json::Value::Array(a) => a.len(),
            _ => 0,
        }
    };
    // The journal is `{ entries: { <game>: [notes] } }`: count the notes.
    let journal = fs::read_to_string(data.join("local_library/journal.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["entries"].as_object().map(|m| m.values().filter_map(|l| l.as_array()).map(Vec::len).sum()))
        .unwrap_or(0);
    Summary {
        games: count("local_library/library.json", "games"),
        sessions: count("local_library/sessions.json", "sessions"),
        journal,
        ..Default::default()
    }
}
