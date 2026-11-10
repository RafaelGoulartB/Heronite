//! What the user set per metadata source, as stored in `settings.metadata`.
//! The sources themselves (names, options, the fields they fill) are
//! `heronite_metadata::sources`; this module only reads and writes the
//! settings, so the app and both crates agree on what "on" means.
//!
//! JSON (every key optional; a file without them behaves as before):
//!
//! ```json
//! "metadata": {
//!   "sources": {
//!     "steamgriddb": { "enabled": false },
//!     "steam":       { "fieldsOff": ["description", "cover"] },
//!     "exampledb":   { "enabled": true, "options": { "language": "ja" } }
//!   },
//!   "sourceOrder": ["store", "igdb", "steam", "lutris", "playnite"],
//!   "fieldPriority": { "cover": ["steam", "igdb"] }
//! }
//! ```
//!
//! - `sources.<id>.enabled`: absent = on. Steam tags and HowLongToBeat keep
//!   their switches in `steamTags` and `howLongToBeat` (older builds and
//!   other PCs read those), never here.
//! - `sources.<id>.options`: the source's own options. IGDB's client id and
//!   secret stay in `igdbClientId` / `igdbClientSecret`, the Steam tags count
//!   in `steamTagLimit` and the SteamGridDB key in the global settings
//!   (`steamGridDbApiKey`); `heronite_metadata::sources` routes them.
//! - `sources.<id>.fieldsOff`: fields the merge never takes from this source.
//! - `sourceOrder`: written only while it differs from the registry's order.
//! - `fieldPriority` keeps its meaning: a list that differs from
//!   [`default_field_priority`] is the user's order for that field.
//!
//! Ids this build does not know are kept everywhere.

use crate::models::{MetadataSettings, SourcePrefs};
use crate::settings::{default_field_priority, ADDED_SOURCES, DEFAULT_LISTED_EXTENSIONS};

/// Ids of the built-in sources whose state is read outside the metadata
/// service (legacy switches, fetch paths of their own, the app).
pub mod ids {
    pub const IGDB: &str = "igdb";
    pub const STEAMGRIDDB: &str = "steamgriddb";
    pub const WEB_IMAGES: &str = "webimages";
    pub const HOW_LONG_TO_BEAT: &str = "howlongtobeat";
    pub const STEAM_TAGS: &str = "steamtags";
    pub const PROTONDB: &str = "protondb";
}

/// Whether source `id` may be asked.
pub fn is_on(m: &MetadataSettings, id: &str) -> bool {
    match id {
        ids::STEAM_TAGS => m.steam_tags,
        ids::HOW_LONG_TO_BEAT => m.how_long_to_beat,
        _ => m.sources.get(id).and_then(|p| p.enabled) != Some(false),
    }
}

pub fn set_on(m: &mut MetadataSettings, id: &str, on: bool) {
    match id {
        ids::STEAM_TAGS => m.steam_tags = on,
        ids::HOW_LONG_TO_BEAT => m.how_long_to_beat = on,
        // On is the default: nothing to store.
        _ => edit(m, id, |p| p.enabled = (!on).then_some(false)),
    }
}

/// Whether the merge may take `field` from source `id`.
pub fn is_field_on(m: &MetadataSettings, id: &str, field: &str) -> bool {
    !m.sources.get(id).is_some_and(|p| p.fields_off.iter().any(|f| f == field))
}

pub fn set_field_on(m: &mut MetadataSettings, id: &str, field: &str, on: bool) {
    edit(m, id, |p| {
        p.fields_off.retain(|f| f != field);
        if !on {
            p.fields_off.push(field.to_owned());
        }
    });
}

/// Option `key` of source `id` as stored in `sources.<id>.options` (not the
/// options kept in their own keys, see the module docs).
pub fn option(m: &MetadataSettings, id: &str, key: &str) -> Option<serde_json::Value> {
    m.sources.get(id)?.options.get(key).cloned()
}

/// `None` removes the option.
pub fn set_option(m: &mut MetadataSettings, id: &str, key: &str, value: Option<serde_json::Value>) {
    edit(m, id, |p| match value {
        Some(v) => {
            p.options.insert(key.to_owned(), v);
        }
        None => {
            p.options.remove(key);
        }
    });
}

/// Whether the user set an order for `field`: its saved `fieldPriority`
/// differs from the default one, and from the default as it was before the
/// [`ADDED_SOURCES`] (what every settings file saved until then), once a
/// [`DEFAULT_LISTED_EXTENSIONS`] id is left out where those defaults had it.
pub fn is_field_customised(m: &MetadataSettings, field: &str) -> bool {
    m.field_priority.get(field).is_some_and(|saved| {
        let list = without_default_listed(saved);
        let default = default_field_priority(field);
        // The default as each earlier build saved it: without the sources
        // added after it.
        let saved_before = (0..ADDED_SOURCES.len()).map(|built| {
            let later = &ADDED_SOURCES[built..];
            default.iter().filter(|id| !later.contains(&id.as_str())).cloned().collect::<Vec<String>>()
        });
        list != default && saved_before.into_iter().all(|before| list != before)
    })
}

/// `list` without the [`DEFAULT_LISTED_EXTENSIONS`] ids followed only by
/// [`ADDED_SOURCES`]: the place the defaults that named them had them.
fn without_default_listed(list: &[String]) -> Vec<String> {
    let at_default_place = |i: usize| {
        DEFAULT_LISTED_EXTENSIONS.contains(&list[i].as_str()) && list[i + 1..].iter().all(|id| ADDED_SOURCES.contains(&id.as_str()))
    };
    (0..list.len()).filter(|i| !at_default_place(*i)).map(|i| list[i].clone()).collect()
}

/// Back to the default order for `field`, stored as the default list so
/// every build reads the same.
pub fn reset_field(m: &mut MetadataSettings, field: &str) {
    m.field_priority.insert(field.to_owned(), default_field_priority(field));
}

/// Applies `f` to the entry of `id` and drops the entry when it is left empty.
fn edit(m: &mut MetadataSettings, id: &str, f: impl FnOnce(&mut SourcePrefs)) {
    let p = m.sources.entry(id.to_owned()).or_default();
    f(p);
    if p.is_empty() {
        m.sources.shift_remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_store;
    use crate::models::CollectionSettingsFile;
    use crate::paths::DataPaths;
    use crate::settings::{self, normalize_metadata, METADATA_FIELDS};
    use serde_json::{json, Value};

    fn read(json: &str) -> MetadataSettings {
        let file: CollectionSettingsFile = serde_json::from_str(json).unwrap();
        normalize_metadata(file.metadata.as_ref())
    }

    fn write(m: &MetadataSettings) -> Value {
        serde_json::to_value(m).unwrap()
    }

    #[test]
    fn an_older_file_reads_as_before_and_writes_no_new_keys() {
        let m = read(r#"{"metadata":{"igdbClientId":"id","steamTags":false,"fieldPriority":{"cover":["steam","igdb"]}}}"#);
        assert!(m.sources.is_empty() && m.source_order.is_empty());
        assert!(!is_on(&m, ids::STEAM_TAGS) && is_on(&m, ids::HOW_LONG_TO_BEAT) && is_on(&m, "igdb"));
        assert!(is_field_customised(&m, "cover") && !is_field_customised(&m, "hero"));
        let out = write(&m);
        assert!(out.get("sources").is_none() && out.get("sourceOrder").is_none());
        for field in METADATA_FIELDS {
            assert!(is_field_on(&m, "igdb", field));
        }
        // A list saved before LaunchBox was a source is still the default.
        let m =
            read(r#"{"metadata":{"fieldPriority":{"cover":["igdb","steam","lutris","playnite","store"],"hero":["exampledb","igdb"]}}}"#);
        assert!(!is_field_customised(&m, "cover") && is_field_customised(&m, "hero"));
        // Nor is a list saved when LaunchBox was the last one added, before libretro-thumbnails.
        let m = read(
            r#"{"metadata":{"fieldPriority":{"cover":["igdb","steam","lutris","playnite","store","launchbox"],
                "coverSquare":["steamgriddb","launchbox"],"coverWide":["steam","store","steamgriddb"]}}}"#,
        );
        assert!(["cover", "coverSquare", "coverWide"].iter().all(|f| !is_field_customised(&m, f)));
        assert_eq!(m.field_priority["coverWide"], ["steam", "store", "steamgriddb"], "the saved list is kept as it is");
        let m = read(r#"{"metadata":{"fieldPriority":{"coverWide":["steamgriddb","steam","store"]}}}"#);
        assert!(is_field_customised(&m, "coverWide"));
        // The defaults of the builds where VNDB was built in, with it last of
        // the sources of then: still the default.
        let m = read(
            r#"{"metadata":{"fieldPriority":{"cover":["igdb","steam","lutris","playnite","store","vndb","launchbox","libretro"],
                "hero":["igdb","steam","lutris","playnite","vndb","launchbox"],"themes":["igdb","playnite","vndb"],
                "description":["igdb","steam","store","lutris","playnite","vndb"]}}}"#,
        );
        assert!(["cover", "hero", "themes", "description"].iter().all(|f| !is_field_customised(&m, f)));
        assert_eq!(m.field_priority["themes"], ["igdb", "playnite", "vndb"], "the saved list is kept as it is");
        // Placed elsewhere by the user, it is the user's list.
        let m = read(
            r#"{"metadata":{"fieldPriority":{"themes":["vndb","igdb","playnite"],"hero":["igdb","steam","lutris","vndb","playnite","launchbox"]}}}"#,
        );
        assert!(is_field_customised(&m, "themes") && is_field_customised(&m, "hero"));
        // No metadata at all: defaults, nothing customised.
        let m = read("{}");
        assert!(METADATA_FIELDS.iter().all(|f| !is_field_customised(&m, f)));
    }

    #[test]
    fn unknown_ids_and_keys_survive_a_round_trip() {
        let m = read(
            r#"{"metadata":{"fieldPriority":{"description":["exampledb","igdb"," "]},"sourceOrder":["exampledb","igdb","exampledb",""],
                "sources":{"exampledb":{"enabled":false,"options":{"lang":"ja"},"fieldsOff":["cover","cover"],"later":1},"empty":{}},
                "laterKey":{"a":1}}}"#,
        );
        assert_eq!(m.field_priority["description"], ["exampledb", "igdb"]);
        assert_eq!(m.source_order, ["exampledb", "igdb"]);
        assert!(!is_on(&m, "exampledb") && !is_field_on(&m, "exampledb", "cover"));
        assert_eq!(option(&m, "exampledb", "lang"), Some(json!("ja")));
        assert!(!m.sources.contains_key("empty"));
        let out = write(&m);
        assert_eq!(out["sources"]["exampledb"], json!({"enabled": false, "options": {"lang": "ja"}, "fieldsOff": ["cover"], "later": 1}));
        assert_eq!(out["laterKey"], json!({"a": 1}));
        let again = normalize_metadata(Some(&serde_json::from_value(out.clone()).unwrap()));
        assert_eq!(write(&again), out);
        // Only unknown ids: still the user's list.
        let m = read(r#"{"metadata":{"fieldPriority":{"series":["exampledb"]}}}"#);
        assert_eq!(m.field_priority["series"], ["exampledb"]);
        // An empty list falls back to the default, as before.
        let m = read(r#"{"metadata":{"fieldPriority":{"series":[]}}}"#);
        assert_eq!(m.field_priority["series"], default_field_priority("series"));
    }

    #[test]
    fn legacy_switches_stay_where_they_are() {
        let mut m = read("{}");
        set_on(&mut m, ids::STEAM_TAGS, false);
        set_on(&mut m, ids::HOW_LONG_TO_BEAT, false);
        assert!(!m.steam_tags && !m.how_long_to_beat && m.sources.is_empty());
        set_on(&mut m, ids::STEAM_TAGS, true);
        assert!(m.steam_tags && is_on(&m, ids::STEAM_TAGS));

        set_on(&mut m, ids::PROTONDB, false);
        assert_eq!(m.sources[ids::PROTONDB].enabled, Some(false));
        set_on(&mut m, ids::PROTONDB, true);
        assert!(m.sources.is_empty(), "back to the default leaves nothing behind");

        set_field_on(&mut m, "steam", "cover", false);
        set_field_on(&mut m, "steam", "cover", false);
        assert_eq!(m.sources["steam"].fields_off, ["cover"]);
        set_field_on(&mut m, "steam", "cover", true);
        assert!(m.sources.is_empty());
    }

    #[test]
    fn update_metadata_touches_only_the_metadata_section() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.collection_settings(), r#"{"zzz":{"keep":1},"metadata":{"steamTags":true,"other":2}}"#).unwrap();
        let s = settings::update_metadata(&paths, |m| set_on(m, ids::STEAM_TAGS, false)).unwrap();
        assert!(!s.metadata.steam_tags);
        let v = json_store::read_value(&paths.collection_settings()).unwrap();
        assert_eq!(v["zzz"]["keep"], 1);
        assert_eq!(v["metadata"]["steamTags"], false);
        assert_eq!(v["metadata"]["other"], 2);
        assert!(v["metadata"].get("sources").is_none());

        let s = settings::update_metadata(&paths, |m| {
            m.field_priority.insert("cover".into(), vec!["steam".into()]);
        })
        .unwrap();
        assert!(is_field_customised(&s.metadata, "cover"));
        let s = settings::update_metadata(&paths, |m| reset_field(m, "cover")).unwrap();
        assert!(!is_field_customised(&s.metadata, "cover"));
    }
}
