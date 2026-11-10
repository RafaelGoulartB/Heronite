//! What Settings → Metadata shows for each source: its state ([`SourceStatus`])
//! and a row ([`SourceRow`]) with its options and the fields it fills.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use heronite_data::models::MetadataSettings;
use heronite_data::paths::DataPaths;
use heronite_data::source_prefs::{is_field_on, is_on};
use parking_lot::Mutex;

use super::download::{DownloadState, SourceDownload};
use super::options::{option_values, OptionValue, SourceOption};
use super::{Needs, SourceDescriptor, SourceHealth, SourceKind, SourceRegistry};

/// A source's state, worded by the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStatus {
    /// Switched off by the user.
    Off,
    On,
    /// Its keys are set and were not refused by the last test.
    Connected,
    /// A key it needs is empty.
    NeedsKey,
    /// The last test with the current keys failed.
    TestFailed,
    NeedsSignIn,
    /// Its database is not downloaded yet.
    NeedsDownload,
    /// Does not work on this operating system.
    Unavailable,
    /// On, but asked only when the user searches by hand.
    ByHandOnly,
    /// An extension that could not be loaded (broken files, or made for a
    /// newer app): never asked; [`SourceRow::problem`] says why.
    LoadFailed,
    /// An extension that failed too many times in a row: not asked until
    /// switched on again or updated; [`SourceRow::problem`] has the last error.
    Failing,
}

/// `d`'s status. `last_test`: the outcome of its last test with the current
/// option values, `None` when not tested since they changed. A source that
/// needs a download needs it here: see [`status_with`].
pub fn status(d: &SourceDescriptor, on: bool, options: &[(SourceOption, OptionValue)], last_test: Option<bool>) -> SourceStatus {
    status_with(d, on, options, last_test, None, &SourceHealth::Ok)
}

/// [`status`] knowing where its download stands (`None`: not downloaded)
/// and whether it can be asked ([`SourceHealth`]).
pub fn status_with(
    d: &SourceDescriptor,
    on: bool,
    options: &[(SourceOption, OptionValue)],
    last_test: Option<bool>,
    download: Option<&DownloadState>,
    health: &SourceHealth,
) -> SourceStatus {
    if !d.available_here() {
        return SourceStatus::Unavailable;
    }
    if matches!(health, SourceHealth::LoadFailed(_)) {
        return SourceStatus::LoadFailed;
    }
    if !on {
        return SourceStatus::Off;
    }
    if matches!(health, SourceHealth::Failing(_)) {
        return SourceStatus::Failing;
    }
    // No built-in source signs in yet: nothing can say it is done.
    match d.needs {
        Needs::Account => return SourceStatus::NeedsSignIn,
        Needs::Download if !download.is_some_and(|s| s.ready) => return SourceStatus::NeedsDownload,
        Needs::Download | Needs::ApiKey | Needs::Nothing => {}
    }
    if options.iter().any(|(o, v)| o.missing(v)) {
        return SourceStatus::NeedsKey;
    }
    if last_test == Some(false) {
        return SourceStatus::TestFailed;
    }
    if d.needs == Needs::ApiKey {
        return SourceStatus::Connected;
    }
    if d.manual_only {
        SourceStatus::ByHandOnly
    } else {
        SourceStatus::On
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionRow {
    pub option: SourceOption,
    /// The real value, a secret's too: show it masked when
    /// `option.kind` is [`OptionKind::Secret`](super::OptionKind::Secret).
    pub value: OptionValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRow {
    /// A `METADATA_FIELDS` entry or an [`extra_fields`](super::extra_fields) id.
    pub field: String,
    pub on: bool,
    /// Whether switching it off does something: only the merge honours it,
    /// so only the fields of details sources and extensions can be switched.
    pub switchable: bool,
}

/// The database a source downloads, as its row shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRow {
    pub download: SourceDownload,
    pub state: DownloadState,
}

/// One source as Settings → Metadata shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceRow {
    pub descriptor: SourceDescriptor,
    pub enabled: bool,
    pub status: SourceStatus,
    pub options: Vec<OptionRow>,
    /// The fields it fills, in the descriptor's order.
    pub fields: Vec<FieldRow>,
    /// Whether it has a "Test" ([`SourceRegistry::test`]).
    pub can_test: bool,
    /// Whether dragging it changes anything: only the order of the details
    /// sources and of the extensions feeds the merge.
    pub reorderable: bool,
    /// Its database, when it downloads one (whether the source is on or not).
    pub download: Option<DownloadRow>,
    /// Why it cannot be asked ([`SourceStatus::LoadFailed`],
    /// [`SourceStatus::Failing`]): technical English text for the details.
    pub problem: Option<String>,
}

/// The outcome of each source's last test, remembered with the option
/// values it ran with, so changing a key forgets it. In memory only.
#[derive(Default)]
pub(crate) struct TestResults(Mutex<HashMap<String, (u64, bool)>>);

impl TestResults {
    fn fingerprint(values: &[(SourceOption, OptionValue)]) -> u64 {
        let mut h = DefaultHasher::new();
        values.iter().for_each(|(o, v)| (&o.key, v).hash(&mut h));
        h.finish()
    }

    pub(crate) fn record(&self, id: &str, values: &[(SourceOption, OptionValue)], ok: bool) {
        self.0.lock().insert(id.to_owned(), (Self::fingerprint(values), ok));
    }

    pub(crate) fn get(&self, id: &str, values: &[(SourceOption, OptionValue)]) -> Option<bool> {
        let (at, ok) = *self.0.lock().get(id)?;
        (at == Self::fingerprint(values)).then_some(ok)
    }
}

impl SourceRegistry {
    /// One row per visible (not hidden) source, in the user's order; the
    /// interface groups them by `descriptor.kind`. Reads the global settings
    /// for SteamGridDB's key: build them on a change, not per frame.
    pub(crate) fn rows(&self, paths: &DataPaths, m: &MetadataSettings, tests: &TestResults) -> Vec<SourceRow> {
        self.order(m)
            .iter()
            .filter_map(|id| self.get(id))
            .filter(|d| !d.hidden)
            .map(|d| {
                let values = option_values(paths, m, d);
                let enabled = is_on(m, &d.id);
                let merged = d.kind == SourceKind::Details || (d.extension.is_some() && self.has_lookup(&d.id));
                let switchable = merged;
                let health = self.health(&d.id);
                let download =
                    d.download.clone().and_then(|download| Some(DownloadRow { download, state: self.download_state(paths, &d.id)? }));
                SourceRow {
                    status: status_with(d, enabled, &values, tests.get(&d.id, &values), download.as_ref().map(|r| &r.state), &health),
                    download,
                    enabled,
                    options: values.into_iter().map(|(option, value)| OptionRow { option, value }).collect(),
                    fields: d.provides.iter().map(|f| FieldRow { field: f.clone(), on: is_field_on(m, &d.id, f), switchable }).collect(),
                    can_test: self.has_test(&d.id),
                    reorderable: merged,
                    problem: match health {
                        SourceHealth::Ok => None,
                        SourceHealth::LoadFailed(why) | SourceHealth::Failing(why) => Some(why),
                    },
                    descriptor: d.clone(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use heronite_data::settings::normalize_metadata;
    use heronite_data::source_prefs::{set_field_on, set_on};

    use super::*;
    use crate::sources::TEST_FALLBACK;

    fn setup() -> (tempfile::TempDir, DataPaths, SourceRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths, SourceRegistry::for_tests())
    }

    fn row<'a>(rows: &'a [SourceRow], id: &str) -> &'a SourceRow {
        rows.iter().find(|r| r.descriptor.id == id).unwrap()
    }

    #[test]
    fn igdb_needs_both_keys_then_is_connected_until_a_test_fails() {
        let (_dir, paths, reg) = setup();
        let tests = TestResults::default();
        let mut m = normalize_metadata(None);
        let igdb = |m: &MetadataSettings| row(&reg.rows(&paths, m, &tests), "igdb").clone();
        assert_eq!(igdb(&m).status, SourceStatus::NeedsKey);
        m.igdb_client_id = "id".into();
        assert_eq!(igdb(&m).status, SourceStatus::NeedsKey, "the secret is missing");
        m.igdb_client_secret = "secret".into();
        let r = igdb(&m);
        assert_eq!(r.status, SourceStatus::Connected);
        assert!(r.enabled && r.can_test && r.reorderable);
        let values: Vec<_> = r.options.iter().map(|o| (o.option.key.as_str(), o.value.as_text())).collect();
        assert_eq!(values, [("clientId", "id"), ("clientSecret", "secret")]);

        let opts = option_values(&paths, &m, reg.get("igdb").unwrap());
        tests.record("igdb", &opts, false);
        assert_eq!(igdb(&m).status, SourceStatus::TestFailed);
        m.igdb_client_secret = "other".into();
        assert_eq!(igdb(&m).status, SourceStatus::Connected, "new keys: not tested yet");
        tests.record("igdb", &option_values(&paths, &m, reg.get("igdb").unwrap()), true);
        assert_eq!(igdb(&m).status, SourceStatus::Connected);

        set_on(&mut m, "igdb", false);
        let r = igdb(&m);
        assert_eq!((r.enabled, r.status), (false, SourceStatus::Off));
    }

    #[test]
    fn rows_cover_the_visible_sources_in_order() {
        let (_dir, paths, _) = setup();
        let reg = SourceRegistry::for_tests_with_fallback();
        let mut m = normalize_metadata(None);
        set_field_on(&mut m, "steam", "cover", false);
        let rows = reg.rows(&paths, &m, &TestResults::default());
        let ids: Vec<&str> = rows.iter().map(|r| r.descriptor.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "igdb",
                "steam",
                "store",
                "lutris",
                "launchbox",
                "steamgriddb",
                "libretro",
                "webimages",
                "howlongtobeat",
                "steamtags",
                "protondb",
                TEST_FALLBACK
            ]
        );
        let status = |id| row(&rows, id).status;
        assert_eq!(status("steam"), SourceStatus::On);
        assert_eq!(status(TEST_FALLBACK), SourceStatus::On);
        let fallback = row(&rows, TEST_FALLBACK);
        assert!(fallback.reorderable && !fallback.can_test && fallback.fields.iter().all(|f| f.switchable && f.on));
        let options: Vec<(&str, &OptionValue)> = fallback.options.iter().map(|o| (o.option.key.as_str(), &o.value)).collect();
        assert_eq!(options, [("titleOrder", &OptionValue::Text("latin".into())), ("screenshotBackground", &OptionValue::Switch(true))]);
        assert_eq!(status("steamgriddb"), SourceStatus::NeedsKey);
        assert_eq!(status("webimages"), SourceStatus::ByHandOnly);
        let libretro = row(&rows, "libretro");
        assert_eq!(libretro.status, SourceStatus::On, "no key, no download, no option");
        assert!(libretro.options.is_empty() && libretro.download.is_none() && !libretro.can_test && !libretro.reorderable);
        assert_eq!(libretro.fields.iter().map(|f| f.field.as_str()).collect::<Vec<_>>(), ["cover", "coverSquare", "coverWide"]);
        assert_eq!(status("protondb"), SourceStatus::On);
        let steam = row(&rows, "steam");
        assert!(steam.fields.iter().all(|f| f.switchable && f.on == (f.field != "cover")));
        let tags = row(&rows, "steamtags");
        assert!(!tags.reorderable && !tags.can_test && tags.fields.iter().all(|f| !f.switchable));
        assert_eq!(tags.options[0].value, OptionValue::Text("10".into()));
        assert!(!row(&rows, "lutris").can_test);
    }

    #[test]
    fn status_of_other_needs_and_systems() {
        let mut d: SourceDescriptor = serde_json::from_str(r#"{"id":"x","name":"X","kind":"details","needs":"account"}"#).unwrap();
        assert_eq!(status(&d, true, &[], None), SourceStatus::NeedsSignIn);
        assert_eq!(status(&d, false, &[], None), SourceStatus::Off);
        d.needs = Needs::Download;
        assert_eq!(status(&d, true, &[], None), SourceStatus::NeedsDownload);
        let mut state = DownloadState::default();
        assert_eq!(status_with(&d, true, &[], None, Some(&state), &SourceHealth::Ok), SourceStatus::NeedsDownload);
        state.ready = true;
        assert_eq!(status_with(&d, true, &[], None, Some(&state), &SourceHealth::Ok), SourceStatus::On);
        assert_eq!(
            status_with(&d, false, &[], None, Some(&state), &SourceHealth::Ok),
            SourceStatus::Off,
            "off keeps the database, and says off"
        );
        d.needs = Needs::Nothing;
        assert_eq!(status(&d, true, &[], Some(false)), SourceStatus::TestFailed);
        d.only_on = vec!["no-such-os".into()];
        assert_eq!(status(&d, true, &[], None), SourceStatus::Unavailable);
        d.only_on = vec![std::env::consts::OS.into()];
        assert_eq!(status(&d, true, &[], None), SourceStatus::On);
        let reg = SourceRegistry::for_tests();
        let playnite = reg.get("playnite").unwrap();
        assert_eq!(status(playnite, true, &[], None) == SourceStatus::Unavailable, !cfg!(windows));
    }
}
