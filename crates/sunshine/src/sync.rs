//! Brings Sunshine's application list in line with the entries Heronite
//! wants there. Only applications with Heronite's marker are changed or
//! removed; the user's own are never touched.
//!
//! Sunshine addresses applications by their place in the list and sorts the
//! list by name after every save, so the list is read again before each
//! change that needs an index. Deletions go last and from the highest index
//! down: a deletion keeps the order of the rest.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::api::{Client, Error};
use crate::entry::{marker, Entry};

/// What a sync changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

impl Report {
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.removed > 0
    }
}

/// The index of the first application marked `id`.
fn find(apps: &[Value], id: &str) -> Option<usize> {
    apps.iter().position(|a| marker(a) == Some(id))
}

/// Makes Heronite's applications exactly `wanted` (an id listed twice
/// counts once). Stops at the first error; what was done until then stays.
pub fn sync(client: &Client, wanted: &[Entry]) -> Result<Report, Error> {
    let mut seen = HashSet::new();
    let wanted: Vec<&Entry> = wanted.iter().filter(|e| seen.insert(e.id.as_str())).collect();
    let mut report = Report::default();
    let mut apps = client.apps()?;
    let mut stale = false;

    let existing: HashMap<&str, usize> = {
        let mut m = HashMap::new();
        for (i, a) in apps.iter().enumerate() {
            if let Some(id) = marker(a) {
                m.entry(id).or_insert(i);
            }
        }
        m
    };
    let (present, missing): (Vec<&Entry>, Vec<&Entry>) = wanted.iter().partition(|e| existing.contains_key(e.id.as_str()));
    let to_update: Vec<&Entry> = present.into_iter().filter(|e| !e.matches(&apps[existing[e.id.as_str()]])).collect();

    for e in to_update {
        if stale {
            apps = client.apps()?;
        }
        let Some(i) = find(&apps, &e.id) else { continue };
        let mut app = apps[i].as_object().cloned().unwrap_or_default();
        e.apply_to(&mut app);
        client.save(&Value::Object(app), Some(i))?;
        stale = true;
        report.updated += 1;
    }
    for e in missing {
        client.save(&e.to_app(), None)?;
        stale = true;
        report.added += 1;
    }

    if stale {
        apps = client.apps()?;
    }
    let keep: HashSet<&str> = wanted.iter().map(|e| e.id.as_str()).collect();
    let mut kept = HashSet::new();
    let mut doomed: Vec<usize> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let id = marker(a)?;
            // A second copy of one of ours goes too.
            (!keep.contains(id) || !kept.insert(id)).then_some(i)
        })
        .collect();
    doomed.sort_unstable_by(|a, b| b.cmp(a));
    for i in doomed {
        client.delete(i)?;
        report.removed += 1;
    }
    Ok(report)
}

/// Removes every application of Heronite's (the integration was turned off).
pub fn remove_all(client: &Client) -> Result<usize, Error> {
    sync(client, &[]).map(|r| r.removed)
}
