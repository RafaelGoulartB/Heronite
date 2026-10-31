//! Install information from `gogdl info <id> --os <platform>`: sizes per
//! language, languages, DLCs, build and folder name.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use heronite_data::DataPaths;
use heronite_epic::cache_store::CacheStore;
use heronite_i18n::tr;
use heronite_launch::host::{run_mode, Os, RunMode};
use serde_json::Value;

use crate::cmd::{last_json_object, Gogdl};
use crate::gamesdb::id_text;
use crate::library;

const CACHE: &str = "gog_install_info";
/// Hours-in-minutes lifespan of a saved answer.
const LIFESPAN_MINUTES: u64 = 60 * 6;

/// Bytes per language key; `"*"` is shared by every language.
pub type Sizes = BTreeMap<String, (u64, u64)>;

/// A DLC the account owns for the game.
#[derive(Debug, Clone, PartialEq)]
pub struct Dlc {
    pub id: String,
    pub title: String,
    /// `(download, disk)` bytes per language key.
    pub sizes: Sizes,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstallInfo {
    pub platform: String,
    pub languages: Vec<String>,
    pub dlcs: Vec<Dlc>,
    /// Empty for Linux installers.
    pub build_id: String,
    pub version_name: String,
    pub folder_name: String,
    /// `ETag` of the build manifest; empty for Linux installers.
    pub version_etag: String,
    /// Raw size table; Linux has just `"*"`.
    pub sizes: Sizes,
}

fn bytes(v: Option<&Value>) -> u64 {
    v.and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.) as u64))).unwrap_or(0)
}

/// `{"*": {download_size, disk_size}, "en-US": {...}}`
fn parse_sizes(v: Option<&Value>) -> Sizes {
    let Some(Value::Object(map)) = v else { return Sizes::new() };
    map.iter().map(|(lang, s)| (lang.clone(), (bytes(s.get("download_size")), bytes(s.get("disk_size"))))).collect()
}

fn text(doc: &Value, key: &str) -> String {
    id_text(doc.get(key)).unwrap_or_default()
}

impl InstallInfo {
    /// Reads the JSON `gogdl info` prints.
    pub fn parse(platform: &str, doc: &Value) -> Self {
        let linux = platform == "linux";
        let sizes = if linux {
            Sizes::from([("*".to_string(), (bytes(doc.get("download_size")), bytes(doc.get("disk_size"))))])
        } else {
            parse_sizes(doc.get("size"))
        };
        let dlcs = doc
            .get("dlcs")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|d| Some(Dlc { id: id_text(d.get("id"))?, title: text(d, "title"), sizes: parse_sizes(d.get("size")) }))
                    .collect()
            })
            .unwrap_or_default();
        let languages = doc
            .get("languages")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        Self {
            platform: platform.to_string(),
            languages,
            dlcs,
            build_id: text(doc, "buildId"),
            version_name: text(doc, "versionName"),
            folder_name: text(doc, "folder_name"),
            version_etag: text(doc, "versionEtag"),
            sizes,
        }
    }

    /// `(download, disk)` bytes for `language` with the given DLC ids.
    pub fn size_for(&self, language: &str, dlcs: &[String]) -> (u64, u64) {
        let add = |table: &Sizes| {
            let shared = table.get("*").copied().unwrap_or_default();
            let own = table.get(language).copied().unwrap_or_default();
            (shared.0 + own.0, shared.1 + own.1)
        };
        let mut total = add(&self.sizes);
        for dlc in self.dlcs.iter().filter(|d| dlcs.contains(&d.id)) {
            let s = add(&dlc.sizes);
            total = (total.0 + s.0, total.1 + s.1);
        }
        total
    }

    /// The language to preselect: the first of `languages` matching `ui_lang`
    /// (`pt-BR` exactly, else any `pt-*`), else `en-US`, else the first.
    pub fn default_language(&self, ui_lang: &str) -> String {
        let ui = ui_lang.replace('_', "-");
        let primary = ui.split('-').next().unwrap_or("").to_ascii_lowercase();
        let by_primary = |l: &&String| !primary.is_empty() && l.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(&primary));
        self.languages
            .iter()
            .find(|l| l.eq_ignore_ascii_case(&ui))
            .or_else(|| self.languages.iter().find(by_primary))
            .or_else(|| self.languages.iter().find(|l| l.eq_ignore_ascii_case("en-US")))
            .or_else(|| self.languages.first())
            .cloned()
            .unwrap_or_else(|| "en-US".to_string())
    }
}

/// The platform argument of gogdl for an install platform name.
pub fn os_name(platform: &str) -> &'static str {
    match platform.to_ascii_lowercase().as_str() {
        "linux" => "linux",
        "osx" | "mac" | "macos" => "osx",
        _ => "windows",
    }
}

fn cache_key(app_name: &str, platform: &str) -> String {
    format!("{app_name}_{platform}")
}

/// Runs `gogdl info` and returns its JSON. Network.
fn fetch(gogdl: &Gogdl, app_name: &str, platform: &str) -> Result<Value> {
    let out = gogdl.run(&["info", app_name, "--os", platform])?;
    if out.stderr.contains("Game doesn't support content system api") {
        bail!(tr!("This game cannot be installed for {platform}", platform = platform));
    }
    match last_json_object(&out.stdout) {
        Some(doc) if out.success() => Ok(doc),
        _ => {
            let why = out.error_lines();
            Err(anyhow!(if why.is_empty() {
                tr!("gogdl could not read the install information").to_string()
            } else {
                tr!("gogdl could not read the install information: {error}", error = why)
            }))
        }
    }
}

/// Install information, from the 6 hour cache when it has it.
pub fn install_info(paths: &DataPaths, gogdl: &Gogdl, app_name: &str, platform: &str) -> Result<InstallInfo> {
    let platform = os_name(platform);
    let cache = CacheStore::new(paths, CACHE, Some(LIFESPAN_MINUTES));
    let key = cache_key(app_name, platform);
    if let Some(doc) = cache.get(&key) {
        return Ok(InstallInfo::parse(platform, &doc));
    }
    install_info_fresh(paths, gogdl, app_name, platform)
}

/// [`install_info`] asking gogdl again (after an update).
pub fn install_info_fresh(paths: &DataPaths, gogdl: &Gogdl, app_name: &str, platform: &str) -> Result<InstallInfo> {
    let platform = os_name(platform);
    let doc = fetch(gogdl, app_name, platform)?;
    if let Err(err) = CacheStore::new(paths, CACHE, Some(LIFESPAN_MINUTES)).set(&cache_key(app_name, platform), doc.clone()) {
        log::warn!("saving the install information: {err:#}");
    }
    Ok(InstallInfo::parse(platform, &doc))
}

/// Platforms the game can be installed for and run here: a Linux build is
/// not offered on Windows.
pub fn platforms(paths: &DataPaths, app_name: &str) -> Vec<String> {
    let mut out = vec!["windows".to_string()];
    let linux = library::game_info(paths, app_name).and_then(|g| g.get("is_linux_native").and_then(Value::as_bool)).unwrap_or(false);
    if linux {
        out.push("linux".into());
    }
    let runs = |p: &String| Os::from_name(p).is_some_and(|os| run_mode(os) != RunMode::Unavailable);
    if out.iter().any(runs) {
        out.retain(runs);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::*;
    use serde_json::json;

    fn windows_doc() -> Value {
        json!({
            "size": {"*": {"download_size": 1000, "disk_size": 3000}, "en-US": {"download_size": 200, "disk_size": 400}, "pt-BR": {"download_size": 50, "disk_size": 80}},
            "dlcs": [
                {"title": "Soundtrack", "id": "111", "size": {"*": {"download_size": 10, "disk_size": 20}}},
                {"title": "Expansion", "id": 222, "size": {"*": {"download_size": 100, "disk_size": 300}, "en-US": {"download_size": 1, "disk_size": 2}}}
            ],
            "buildId": "5555", "languages": ["en-US", "de-DE", "pt-BR"], "folder_name": "The Game",
            "dependencies": ["MSVC2010"], "versionEtag": "\"abc\"", "versionName": "2.1", "available_branches": [null], "builds": {}
        })
    }

    #[test]
    fn windows_info_is_parsed() {
        let info = InstallInfo::parse("windows", &windows_doc());
        assert_eq!((info.build_id.as_str(), info.version_name.as_str(), info.folder_name.as_str()), ("5555", "2.1", "The Game"));
        assert_eq!(info.version_etag, "\"abc\"");
        assert_eq!(info.dlcs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["111", "222"]);
        assert_eq!(info.dlcs[1].title, "Expansion");
        assert_eq!(info.size_for("en-US", &[]), (1200, 3400));
        assert_eq!(info.size_for("pt-BR", &[]), (1050, 3080));
        assert_eq!(info.size_for("fr-FR", &[]), (1000, 3000));
        assert_eq!(info.size_for("en-US", &["222".into()]), (1301, 3702));
        assert_eq!(info.size_for("en-US", &["111".into(), "222".into(), "999".into()]), (1311, 3722));
    }

    #[test]
    fn linux_info_has_a_flat_size() {
        let doc = json!({
            "download_size": 700, "disk_size": 900, "dlcs": [{"title": "Extra", "id": "7", "languages": [["en"]]}],
            "languages": ["en-US"], "folder_name": "Linux Game", "dependencies": [], "versionName": "1.2"
        });
        let info = InstallInfo::parse("linux", &doc);
        assert_eq!(info.size_for("en-US", &[]), (700, 900));
        assert_eq!(info.size_for("en-US", &["7".into()]), (700, 900));
        assert_eq!((info.build_id.as_str(), info.version_etag.as_str(), info.version_name.as_str()), ("", "", "1.2"));
        assert_eq!(info.dlcs[0].id, "7");
    }

    #[test]
    fn language_preselection() {
        let info = InstallInfo::parse("windows", &windows_doc());
        assert_eq!(info.default_language("pt-BR"), "pt-BR");
        assert_eq!(info.default_language("pt_BR"), "pt-BR");
        assert_eq!(info.default_language("de"), "de-DE");
        assert_eq!(info.default_language("ja"), "en-US");
        let mut no_english = info.clone();
        no_english.languages = vec!["fr-FR".into(), "de-DE".into()];
        assert_eq!(no_english.default_language("ja"), "fr-FR");
        assert_eq!(no_english.default_language("de-AT"), "de-DE");
        no_english.languages.clear();
        assert_eq!(no_english.default_language("ja"), "en-US");
        let mut pt = info;
        pt.languages = vec!["en-US".into(), "pt-PT".into()];
        assert_eq!(pt.default_language("pt-BR"), "pt-PT");
    }

    #[test]
    fn info_runs_gogdl_and_is_cached() {
        let (dir, paths) = setup();
        let tool = fake_tool(dir.path()).otherwise(Reply::new().err("[INFO] noise").out(&windows_doc().to_string()));
        let g = gogdl(&tool, &paths);
        let info = install_info(&paths, &g, "42", "Windows").unwrap();
        assert_eq!(info.build_id, "5555");
        assert_eq!(calls_of(&tool), ["info 42 --os windows"]);
        // The second call is answered from the cache.
        install_info(&paths, &g, "42", "windows").unwrap();
        assert_eq!(calls_of(&tool).len(), 1);
        install_info_fresh(&paths, &g, "42", "windows").unwrap();
        assert_eq!(calls_of(&tool).len(), 2);
    }

    #[test]
    fn unsupported_platforms_give_a_clear_error() {
        let (dir, paths) = setup();
        let tool = fake_tool(dir.path()).otherwise(
            Reply::new()
                .err("[GENERIC DOWNLOAD_MANAGER] ERROR: Game doesn't support content system api, unable to proceed using platform linux")
                .exit(1),
        );
        let err = install_info(&paths, &gogdl(&tool, &paths), "42", "linux").unwrap_err().to_string();
        assert!(err.contains("cannot be installed for linux"), "{err}");
        let broken = fake_tool(dir.path()).otherwise(Reply::new().err("[X] ERROR: boom").exit(1));
        let err = install_info(&paths, &gogdl(&broken, &paths), "43", "windows").unwrap_err().to_string();
        assert!(err.contains("boom"), "{err}");
    }

    #[test]
    fn platforms_follow_the_library_entry() {
        let (_dir, paths) = setup();
        assert_eq!(platforms(&paths, "1"), ["windows"]);
        let game = |native: bool| json!([{"app_name": "1", "is_linux_native": native}]);
        CacheStore::new(&paths, "gog_library", None).set("games", game(true)).unwrap();
        // Linux builds do not run on Windows.
        let both: &[&str] = if cfg!(windows) { &["windows"] } else { &["windows", "linux"] };
        assert_eq!(platforms(&paths, "1"), both);
        CacheStore::new(&paths, "gog_library", None).set("games", game(false)).unwrap();
        assert_eq!(platforms(&paths, "1"), ["windows"]);
    }
}
