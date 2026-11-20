//! A source's options as data (what Settings draws for it, what an
//! extension's manifest declares) and the one accessor that reads and
//! writes them where each is really kept.

use anyhow::{bail, Result};
use heronite_data::models::MetadataSettings;
use heronite_data::paths::DataPaths;
use heronite_data::settings::{self, EffectiveSettings};
use heronite_data::source_prefs::{self, ids};
use heronite_data::{game_config, steam_tags};
use serde::{Deserialize, Serialize};

use super::SourceDescriptor;
use crate::covers;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OptionKind {
    Text,
    /// Text shown masked (keys, passwords).
    Secret,
    /// One of [`SourceOption::choices`].
    Select,
    Switch,
}

/// A value of an option: text (also a select's choice) or a switch.
/// JSON: a string or a bool.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OptionValue {
    Text(String),
    Switch(bool),
}

impl OptionValue {
    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(s) => s,
            Self::Switch(_) => "",
        }
    }

    pub fn as_bool(&self) -> bool {
        matches!(self, Self::Switch(true))
    }

    fn is_blank(&self) -> bool {
        matches!(self, Self::Text(s) if s.trim().is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionChoice {
    pub value: String,
    /// Shown as is when it is a number or a proper noun, else a `tr!` key.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceOption {
    /// Stable key, unique within the source.
    pub key: String,
    /// English `tr!` key.
    pub label: String,
    pub kind: OptionKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<OptionChoice>,
    /// `None`: empty text, the first choice, or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<OptionValue>,
    /// English `tr!` key shown under the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Where to get the value (a sign-up or API key page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The source cannot be asked while this is empty (its key).
    #[serde(default)]
    pub required: bool,
}

impl SourceOption {
    /// The value an option has when nothing is stored.
    pub fn default_value(&self) -> OptionValue {
        match (&self.default, self.kind) {
            (Some(v), _) => v.clone(),
            (None, OptionKind::Switch) => OptionValue::Switch(false),
            (None, OptionKind::Select) => OptionValue::Text(self.choices.first().map(|c| c.value.clone()).unwrap_or_default()),
            (None, _) => OptionValue::Text(String::new()),
        }
    }

    /// Whether this required option is missing.
    pub(crate) fn missing(&self, value: &OptionValue) -> bool {
        self.required && value.is_blank()
    }
}

/// The options the built-in sources keep outside `sources.<id>.options`.
enum Legacy {
    IgdbClientId,
    IgdbClientSecret,
    SteamTagLimit,
    /// `steamGridDbApiKey` of the global settings.
    SteamGridKey,
}

fn legacy(id: &str, key: &str) -> Option<Legacy> {
    Some(match (id, key) {
        (ids::IGDB, "clientId") => Legacy::IgdbClientId,
        (ids::IGDB, "clientSecret") => Legacy::IgdbClientSecret,
        (ids::STEAM_TAGS, "limit") => Legacy::SteamTagLimit,
        (ids::STEAMGRIDDB, "apiKey") => Legacy::SteamGridKey,
        _ => return None,
    })
}

/// The current value of option `opt` of source `d`. Reads the global
/// settings file for SteamGridDB's key: call it on a change, not per frame.
pub fn option_value(paths: &DataPaths, m: &MetadataSettings, d: &SourceDescriptor, opt: &SourceOption) -> OptionValue {
    let text = OptionValue::Text;
    match legacy(&d.id, &opt.key) {
        Some(Legacy::IgdbClientId) => text(m.igdb_client_id.clone()),
        Some(Legacy::IgdbClientSecret) => text(m.igdb_client_secret.clone()),
        Some(Legacy::SteamGridKey) => text(covers::steamgrid_api_key(paths).unwrap_or_default()),
        // The first choice that shows at least the saved count.
        Some(Legacy::SteamTagLimit) => {
            let limit = steam_tags::normalize_limit(m.steam_tag_limit);
            let at_least = opt.choices.iter().find(|c| c.value.parse::<u32>().is_ok_and(|n| n >= limit)).or(opt.choices.last());
            at_least.map_or_else(|| opt.default_value(), |c| text(c.value.clone()))
        }
        None => source_prefs::option(m, &d.id, &opt.key)
            .and_then(|v| serde_json::from_value::<OptionValue>(v).ok())
            .unwrap_or_else(|| opt.default_value()),
    }
}

/// Every option of `d` with its value, in the descriptor's order.
pub fn option_values(paths: &DataPaths, m: &MetadataSettings, d: &SourceDescriptor) -> Vec<(SourceOption, OptionValue)> {
    d.options.iter().map(|o| (o.clone(), option_value(paths, m, d, o))).collect()
}

/// Saves option `key` of source `d`. Text is trimmed but a secret's (as
/// the IGDB secret always was). A select takes only one of its choices.
pub fn set_option_value(paths: &DataPaths, d: &SourceDescriptor, key: &str, value: OptionValue) -> Result<EffectiveSettings> {
    let Some(opt) = d.options.iter().find(|o| o.key == key) else { bail!("{} has no option {key}", d.id) };
    let value = match (opt.kind, value) {
        (OptionKind::Switch, v @ OptionValue::Switch(_)) => v,
        (OptionKind::Secret, OptionValue::Text(s)) => OptionValue::Text(s),
        (OptionKind::Text, OptionValue::Text(s)) => OptionValue::Text(s.trim().to_owned()),
        (OptionKind::Select, OptionValue::Text(s)) if opt.choices.iter().any(|c| c.value == s) => OptionValue::Text(s),
        (kind, v) => bail!("{v:?} is not a value of the {kind:?} option {}.{key}", d.id),
    };
    match legacy(&d.id, key) {
        Some(Legacy::IgdbClientId) => settings::update_metadata(paths, |m| m.igdb_client_id = value.as_text().to_owned()),
        Some(Legacy::IgdbClientSecret) => settings::update_metadata(paths, |m| m.igdb_client_secret = value.as_text().to_owned()),
        Some(Legacy::SteamTagLimit) => {
            let limit = value.as_text().parse::<u32>().unwrap_or(0);
            settings::update_metadata(paths, |m| m.steam_tag_limit = limit)
        }
        Some(Legacy::SteamGridKey) => {
            // Windows sets no HOME. The home folder only seeds the default
            // settings when the file has none yet.
            let home = heronite_platform::dirs::home().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into());
            game_config::set_global_setting(paths, &home, "steamGridDbApiKey", value.as_text().trim().into())?;
            settings::load(paths)
        }
        None => {
            let stored = (value != opt.default_value()).then(|| serde_json::to_value(&value).expect("plain value"));
            settings::update_metadata(paths, |m| source_prefs::set_option(m, &d.id, key, stored))
        }
    }
}

#[cfg(test)]
mod tests {
    use heronite_data::json_store;
    use heronite_data::settings::normalize_metadata;
    use serde_json::json;

    use super::*;
    use crate::sources::SourceRegistry;

    fn setup() -> (tempfile::TempDir, DataPaths, SourceRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths, SourceRegistry::for_tests())
    }

    fn value(paths: &DataPaths, d: &SourceDescriptor, key: &str) -> OptionValue {
        let m = settings::load(paths).unwrap().metadata;
        option_value(paths, &m, d, d.options.iter().find(|o| o.key == key).unwrap())
    }

    #[test]
    fn built_in_options_live_in_their_old_keys() {
        let (_dir, paths, reg) = setup();
        let igdb = reg.get("igdb").unwrap();
        let s = set_option_value(&paths, igdb, "clientId", OptionValue::Text(" id ".into())).unwrap();
        assert_eq!(s.metadata.igdb_client_id, "id");
        let s = set_option_value(&paths, igdb, "clientSecret", OptionValue::Text(" s ".into())).unwrap();
        assert_eq!(s.metadata.igdb_client_secret, " s ", "a secret is kept as typed");
        assert!(s.metadata.sources.is_empty());

        let tags = reg.get("steamtags").unwrap();
        assert_eq!(value(&paths, tags, "limit"), OptionValue::Text("10".into()));
        let s = set_option_value(&paths, tags, "limit", OptionValue::Text("15".into())).unwrap();
        assert_eq!(s.metadata.steam_tag_limit, 15);
        assert_eq!(value(&paths, tags, "limit"), OptionValue::Text("15".into()));
        assert!(set_option_value(&paths, tags, "limit", OptionValue::Text("7".into())).is_err(), "not a choice");
        assert!(set_option_value(&paths, tags, "nope", OptionValue::Text("5".into())).is_err());
        // A count saved by hand shows as the first choice that shows it all.
        let mut m = normalize_metadata(None);
        m.steam_tag_limit = 12;
        assert_eq!(option_value(&paths, &m, tags, &tags.options[0]), OptionValue::Text("15".into()));

        let sgdb = reg.get("steamgriddb").unwrap();
        set_option_value(&paths, sgdb, "apiKey", OptionValue::Text(" key ".into())).unwrap();
        let global = json_store::read_value(&paths.global_config()).unwrap();
        assert_eq!(global["defaultSettings"]["steamGridDbApiKey"], "key");
        // The first write seeds the defaults from the user's home folder,
        // also where no HOME is set (Windows).
        let home = heronite_platform::dirs::home().unwrap().to_string_lossy().into_owned();
        assert_eq!(global["defaultSettings"]["defaultSteamPath"], format!("{}/.steam/steam", home.trim_end_matches('/')));
        assert_eq!(value(&paths, sgdb, "apiKey"), OptionValue::Text("key".into()));
        let file = json_store::read_value(&paths.collection_settings()).unwrap();
        assert!(file["metadata"].get("sources").is_none());
    }

    #[test]
    fn other_options_go_under_the_source() {
        let (_dir, paths, _) = setup();
        let d: SourceDescriptor = serde_json::from_value(json!({
            "id": "exampledb", "name": "ExampleDB", "kind": "details",
            "options": [
                {"key": "lang", "label": "Language", "kind": "select",
                 "choices": [{"value": "en", "label": "English"}, {"value": "ja", "label": "Japanese"}]},
                {"key": "nsfw", "label": "Adult images", "kind": "switch", "default": true}
            ]
        }))
        .unwrap();
        assert_eq!(value(&paths, &d, "lang"), OptionValue::Text("en".into()));
        assert_eq!(value(&paths, &d, "nsfw"), OptionValue::Switch(true));
        let s = set_option_value(&paths, &d, "lang", OptionValue::Text("ja".into())).unwrap();
        assert_eq!(s.metadata.sources["exampledb"].options["lang"], json!("ja"));
        set_option_value(&paths, &d, "nsfw", OptionValue::Switch(false)).unwrap();
        assert_eq!(value(&paths, &d, "nsfw"), OptionValue::Switch(false));
        assert!(set_option_value(&paths, &d, "nsfw", OptionValue::Text("x".into())).is_err());
        // Back to the defaults: nothing left in the file.
        set_option_value(&paths, &d, "nsfw", OptionValue::Switch(true)).unwrap();
        let s = set_option_value(&paths, &d, "lang", OptionValue::Text("en".into())).unwrap();
        assert!(s.metadata.sources.is_empty());
    }
}
