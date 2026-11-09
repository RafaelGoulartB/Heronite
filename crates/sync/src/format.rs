//! The file each PC publishes: `devices/<device id>/state.json` in the sync
//! folder. Only its PC writes it, and it always holds everything that PC has
//! to say (not a list of changes), so reading it again or missing a version
//! loses nothing.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use heronite_data::models::{CompletionStatus, LocalGameSession};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of the format. A file of a newer version is left alone (never
/// read, never merged) until this app is updated.
pub const FORMAT: u32 = 1;

/// Name of each PC's file in its folder.
pub const STATE_FILE: &str = "state.json";
/// The folder holding one folder per PC.
pub const DEVICES_DIR: &str = "devices";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFile {
    pub format: u32,
    pub device: DeviceInfo,
    /// RFC 3339, UTC.
    pub written_at: String,
    /// The PC's logical clock when it wrote the file (see [`Stamp`]).
    #[serde(default)]
    pub clock: u64,
    /// By Collection key (`<runner>_<appName>`).
    #[serde(default)]
    pub games: BTreeMap<String, GameState>,
    /// The completion statuses the user made (so a status id of another PC
    /// is known here).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<CompletionStatus>,
    /// Smart groups by id, each as last written wins: the stored group
    /// (`heronite_data::smart_groups`) without `pinned`, which each PC keeps
    /// for itself; `null` for one deleted, so it does not come back from a
    /// PC that still has it. Older versions skip the key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, Stamped>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    /// `linux` or `windows` (`std::env::consts::OS`).
    pub os: String,
    #[serde(default)]
    pub app_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    pub runner: String,
    pub app_name: String,
    #[serde(default)]
    pub title: String,
    /// Each field with when it last changed ([`Field`] keys).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Stamped>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playtime: Option<Playtime>,
    /// Only the sessions played on this PC: the others' come from their own
    /// files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<LocalGameSession>,
}

/// Prefix of a game's field that says whether it is in a custom category
/// (`category:<name>`: `true`, or `null` when not), each game and category
/// on its own so that two PCs adding a game to two categories keep both.
/// Older versions skip these keys.
pub const CATEGORY_PREFIX: &str = "category:";

/// The category a field key stands for.
pub fn category_of(key: &str) -> Option<&str> {
    key.strip_prefix(CATEGORY_PREFIX).filter(|n| !n.is_empty())
}

/// The fields that sync, each kept as last written wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// Completion status id (`completionStatusId`).
    Status,
    Favourite,
    Hidden,
    /// Stars, `1..=5`.
    Rating,
    Notes,
}

impl Field {
    pub const ALL: [Field; 5] = [Field::Status, Field::Favourite, Field::Hidden, Field::Rating, Field::Notes];

    pub fn key(self) -> &'static str {
        match self {
            Field::Status => "status",
            Field::Favourite => "favourite",
            Field::Hidden => "hidden",
            Field::Rating => "rating",
            Field::Notes => "notes",
        }
    }

    pub fn parse(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// When a value was written, for "last written wins".
///
/// Ordered by the clock first: a PC's clock goes past every clock it read
/// from the others, so a change made after reading another PC's always
/// wins, even when the computer's time is wrong (a dual boot whose Windows
/// keeps local time in the hardware clock). The time breaks ties between
/// changes neither PC had seen, the device id ties of the same millisecond.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub clock: u64,
    /// RFC 3339, UTC.
    pub at: String,
    pub device: String,
}

impl Ord for Stamp {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.clock, &self.at, &self.device).cmp(&(other.clock, &other.at, &other.device))
    }
}

impl PartialOrd for Stamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A field's value (`null`: none, off) and its stamp.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stamped {
    pub value: Value,
    #[serde(flatten)]
    pub stamp: Stamp,
}

/// The game's total play time as this PC has it.
///
/// Totals merge by the largest: the stores' own play time (Steam's counts
/// every PC) is in them too, so adding the PCs' totals would count it twice.
/// A total made smaller by hand carries `lowered`, and the newest of those
/// is taken as is.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Playtime {
    pub minutes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_played: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowered: Option<Stamp>,
}

impl DeviceFile {
    /// Reads a file of another PC. `None` for a file this version cannot
    /// read (broken, cut short while it was copied, or newer).
    pub fn parse(bytes: &[u8]) -> Option<DeviceFile> {
        let file: DeviceFile = serde_json::from_slice(bytes).ok()?;
        (file.format <= FORMAT && !file.device.id.is_empty()).then_some(file)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(clock: u64, at: &str, device: &str) -> Stamp {
        Stamp { clock, at: at.into(), device: device.into() }
    }

    #[test]
    fn the_clock_wins_over_the_time() {
        // Windows' clock is hours ahead, but Linux read it before changing.
        let windows = stamp(4, "2026-11-01T23:00:00.000Z", "w");
        let linux = stamp(5, "2026-11-01T20:00:00.000Z", "l");
        assert!(linux > windows);
        assert!(stamp(5, "2026-11-01T20:00:01.000Z", "l") > linux);
        assert!(stamp(5, "2026-11-01T20:00:00.000Z", "z") > linux);
    }

    #[test]
    fn files_of_a_newer_version_or_broken_are_not_read() {
        let mut file = DeviceFile { format: FORMAT, device: DeviceInfo { id: "a".into(), ..Default::default() }, ..Default::default() };
        assert!(DeviceFile::parse(&file.to_bytes()).is_some());
        file.format = FORMAT + 1;
        assert!(DeviceFile::parse(&file.to_bytes()).is_none());
        assert!(DeviceFile::parse(b"{\"format\":1,\"dev").is_none());
    }
}
