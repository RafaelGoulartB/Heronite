//! What went up last, kept on this PC (`cloud-sent.json` in the app state),
//! so Settings → Cloud tells whether everything is in the cloud, something
//! new is not yet, or nothing was ever sent, without asking the account.
//!
//! Each list is what an upload set out to send, recorded only once it went
//! through: an upload stopped half way (the app closed, the network fell)
//! leaves the old list, and what is missing shows as not sent.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use heronite_data::{json_store, AppPaths};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sent {
    /// Where these went (`SyncPrefs::destination`): another place has none.
    pub destination: String,
    /// Screenshots: path in the captures folder → bytes.
    pub captures: BTreeMap<String, u64>,
    /// Ludusavi's backups: path in its folder → bytes.
    pub saves: BTreeMap<String, u64>,
    /// Backups sent, by folder name (`heronite-<date>`).
    pub backups: BTreeSet<String>,
    /// The artwork sent with them: `art/…`, `metadata-art/…` → bytes.
    pub art: BTreeMap<String, u64>,
}

/// How much of a kind is in the cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing on this PC to send.
    Empty,
    /// Nothing ever went up.
    Nothing,
    /// Some of it is not up yet.
    Behind { files: u64, bytes: u64 },
    /// All of it is up.
    Done,
}

/// `now` (path → bytes) against what went up.
pub fn compare<'a>(now: impl IntoIterator<Item = (&'a str, u64)>, sent: &BTreeMap<String, u64>) -> Status {
    let (mut any, mut files, mut bytes) = (false, 0, 0);
    for (path, size) in now {
        any = true;
        if sent.get(path) != Some(&size) {
            files += 1;
            bytes += size;
        }
    }
    match (any, files) {
        (false, _) => Status::Empty,
        _ if sent.is_empty() => Status::Nothing,
        (_, 0) => Status::Done,
        _ => Status::Behind { files, bytes },
    }
}

/// What went up to `destination` (nothing, when the record is of another).
pub fn load(app: &AppPaths, destination: &str) -> Sent {
    let sent: Sent = json_store::read(&app.cloud_sent()).unwrap_or_default();
    if sent.destination == destination {
        sent
    } else {
        Sent { destination: destination.to_owned(), ..Default::default() }
    }
}

/// Changes the record of `destination` (a record of another place is
/// dropped first).
pub fn update(app: &AppPaths, destination: &str, f: impl FnOnce(&mut Sent)) -> Result<()> {
    if let Some(dir) = app.cloud_sent().parent() {
        std::fs::create_dir_all(dir)?;
    }
    json_store::update::<Sent, _>(&app.cloud_sent(), |s| {
        if s.destination != destination {
            *s = Sent { destination: destination.to_owned(), ..Default::default() };
        }
        f(s);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_yellow_red() {
        let sent: BTreeMap<String, u64> = [("a.webp".to_owned(), 10)].into();
        assert_eq!(compare([("a.webp", 10)], &sent), Status::Done);
        assert_eq!(compare([("a.webp", 10), ("b.webp", 5)], &sent), Status::Behind { files: 1, bytes: 5 });
        // Changed since it went up.
        assert_eq!(compare([("a.webp", 12)], &sent), Status::Behind { files: 1, bytes: 12 });
        assert_eq!(compare([("a.webp", 10)], &BTreeMap::new()), Status::Nothing);
        assert_eq!(compare([], &sent), Status::Empty);
    }

    #[test]
    fn what_went_to_another_place_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        update(&app, "a:Heronite", |s| {
            s.backups.insert("heronite-1".into());
        })
        .unwrap();
        assert!(load(&app, "a:Heronite").backups.contains("heronite-1"));
        // Signed in again, or another account: nothing there yet.
        assert!(load(&app, "b:Heronite").backups.is_empty());
        update(&app, "b:Heronite", |s| {
            s.captures.insert("x.webp".into(), 1);
        })
        .unwrap();
        assert!(load(&app, "a:Heronite").backups.is_empty(), "the old record is gone");
    }
}
