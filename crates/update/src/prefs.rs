//! `update.json` in this app's config folder (`AppPaths::update_state`): the
//! update switches of Settings → About and what the last check found.

use std::path::Path;

use anyhow::Result;
use heronite_data::json_store;
use serde::{Deserialize, Serialize};

use crate::release::Release;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// Look for a new version at start and every few hours.
    pub auto_check: bool,
    /// Download and swap in a new version as soon as it is found (it runs
    /// from the next start).
    pub auto_install: bool,
    /// "Skip this version": not announced again (a newer one is).
    pub skipped_version: Option<String>,
    /// Unix seconds of the last successful check.
    pub last_check: Option<u64>,
    /// ETag of the last answer (conditional requests).
    pub etag: Option<String>,
    /// The latest release the last answer described.
    pub latest: Option<Release>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { auto_check: true, auto_install: false, skipped_version: None, last_check: None, etag: None, latest: None }
    }
}

impl Prefs {
    /// Missing or unreadable file: the defaults.
    pub fn load(path: &Path) -> Self {
        json_store::read(path).unwrap_or_else(|e| {
            log::warn!("{e:#}: using the default update settings");
            Self::default()
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        json_store::write(path, self)
    }
}

#[cfg(test)]
mod tests {
    use super::Prefs;

    #[test]
    fn defaults_and_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("update.json");
        assert_eq!(Prefs::load(&path), Prefs::default());
        assert!(Prefs::default().auto_check);
        let p = Prefs { auto_install: true, skipped_version: Some("0.2.0".into()), last_check: Some(5), ..Default::default() };
        p.save(&path).unwrap();
        assert_eq!(Prefs::load(&path), p);
        // Partial or broken files fall back field by field / entirely.
        std::fs::write(&path, r#"{"autoCheck": false}"#).unwrap();
        assert_eq!(Prefs::load(&path), Prefs { auto_check: false, ..Default::default() });
        std::fs::write(&path, "{").unwrap();
        assert_eq!(Prefs::load(&path), Prefs::default());
    }
}
