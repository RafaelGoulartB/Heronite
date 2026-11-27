//! The Epic library (port of `LegendaryLibraryManager`): `legendary list`
//! refreshes legendary's own files (`metadata/*.json`, `assets.json`), from
//! which `store_cache/legendary_library.json` is rebuilt in Heroic's format.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use serde_json::{json, Map, Value};

use crate::cache_store::CacheStore;
use crate::cmd::Legendary;
use crate::user;
use crate::util::{epic_store_url, file_size};

pub fn metadata_dir(data_paths: &DataPaths) -> PathBuf {
    data_paths.legendary_config_dir().join("metadata")
}

/// `installed.json` plus `third-party-installed.json` (read only), by app name.
pub fn installed_games(data_paths: &DataPaths) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let dir = data_paths.legendary_config_dir();
    if let Ok(Value::Object(map)) = json_store::read_value(&dir.join("installed.json")) {
        for (k, v) in map {
            out.insert(k, v);
        }
    }
    if let Ok(Value::Array(items)) = json_store::read_value(&dir.join("third-party-installed.json")) {
        for item in items {
            if let (Some(app), Some(platform)) = (item.get(0).and_then(Value::as_str), item.get(1).and_then(Value::as_str)) {
                out.insert(app.to_string(), json!({"app_name": app, "platform": platform}));
            }
        }
    }
    out
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn attr<'a>(meta: &'a Value, name: &str) -> Option<&'a str> {
    meta.get("customAttributes")?.get(name)?.get("value")?.as_str()
}

fn key_image(meta: &Value, types: &[&str]) -> Option<String> {
    meta.get("keyImages")?
        .as_array()?
        .iter()
        .find(|i| i.get("type").and_then(Value::as_str).is_some_and(|t| types.contains(&t)))
        .and_then(|i| text(i, "url"))
}

fn put(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        map.insert(key.to_string(), v);
    }
}

fn platforms_of(release_info: &Value) -> Option<Vec<String>> {
    let platform = release_info.get(0)?.get("platform")?.as_array()?;
    Some(platform.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
}

/// `loadFile`: the `GameInfo` for one `metadata/<app>.json` (and the game's
/// entry of `installed.json`, if installed). `None` for what Heroic leaves
/// out of the library: Unreal Engine assets, mods and mobile-only titles.
pub fn game_info(app_name: &str, file: &Value, installed: Option<&Value>) -> Option<Value> {
    let metadata = file.get("metadata")?;
    let namespace = text(metadata, "namespace");
    let categories: Vec<&str> = metadata
        .get("categories")
        .and_then(Value::as_array)
        .map(|c| c.iter().filter_map(|c| c.get("path").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    let ue = ["assets", "asset-format", "plugins", "projects"];
    if namespace.as_deref() == Some("ue") || categories.iter().any(|c| ue.contains(c)) {
        return None;
    }
    if categories.contains(&"mods") {
        return None;
    }
    let release_info = metadata.get("releaseInfo")?;
    let infos = release_info.as_array()?;
    // `releaseInfo.every(info => info.platform?.every(p => Android | iOS))`
    let mobile_only = infos.iter().all(|i| {
        i.get("platform").and_then(Value::as_array).is_some_and(|p| p.iter().all(|p| matches!(p.as_str(), Some("Android") | Some("iOS"))))
    });
    if mobile_only {
        return None;
    }

    let title = text(metadata, "title").unwrap_or_default();
    let folder_name = attr(metadata, "FolderName");
    let can_run_offline = attr(metadata, "CanRunOffline") == Some("true");
    let third_party = attr(metadata, "ThirdPartyManagedApp")
        .filter(|s| !s.is_empty())
        .or_else(|| attr(metadata, "ThirdPartyManagedProvider").filter(|s| !s.is_empty()))
        .map(str::to_string);

    let info = installed;
    let platform = info.and_then(|i| text(i, "platform"));
    let save_folder =
        if platform.as_deref() == Some("Mac") { attr(metadata, "CloudSaveFolder_MAC") } else { attr(metadata, "CloudSaveFolder") }
            .unwrap_or("")
            .to_string();
    let install_folder = folder_name.map(str::to_string).unwrap_or_else(|| app_name.to_string());

    let game_box = key_image(metadata, &["DieselGameBox", "OfferImageWide"]);
    let game_box_tall = key_image(metadata, &["DieselGameBoxTall", "OfferImageTall"]);
    let game_box_store = key_image(metadata, &["DieselStoreFrontTall"]);
    let logo = key_image(metadata, &["DieselGameBoxLogo"]);
    let art_cover = game_box.clone();
    let art_square = game_box_tall;

    let install_size = info
        .and_then(|i| i.get("install_size"))
        .and_then(|s| s.as_f64().or_else(|| s.as_str().and_then(|s| s.parse().ok())))
        .filter(|s| *s != 0.)
        .map(file_size)
        .unwrap_or_else(|| "0".to_string());

    let mut metadata_platform: Vec<String> = platforms_of(release_info).unwrap_or_default();
    if platforms_of(release_info).is_none() {
        if let Some(main) = metadata.get("mainGameItem") {
            if let Some(p) = main.get("releaseInfo").and_then(platforms_of) {
                metadata_platform = p;
            }
        }
    }

    let store_url = epic_store_url(&title);
    let mut about = Map::new();
    put(&mut about, "description", text(metadata, "description").map(Value::String));
    about.insert("shortDescription".into(), Value::String(text(metadata, "shortDescription").unwrap_or_default()));

    let mut g = Map::new();
    g.insert("app_name".into(), json!(app_name));
    g.insert("art_cover".into(), json!(art_cover.clone().or(art_square.clone()).unwrap_or_else(|| "fallback".into())));
    put(&mut g, "art_logo", logo.map(Value::String));
    g.insert("art_square".into(), json!(art_square.clone().or(game_box_store).or(art_cover).unwrap_or_else(|| "fallback".into())));
    g.insert("cloud_save_enabled".into(), json!(!save_folder.is_empty()));
    put(&mut g, "developer", text(metadata, "developer").map(Value::String));
    g.insert("extra".into(), json!({"about": Value::Object(about), "reqs": [], "storeUrl": store_url}));
    put(&mut g, "dlcList", metadata.get("dlcItemList").cloned());
    g.insert("folder_name".into(), json!(install_folder));

    let mut install = Map::new();
    put(&mut install, "executable", info.and_then(|i| i.get("executable")).cloned());
    put(&mut install, "install_path", info.and_then(|i| i.get("install_path")).cloned());
    install.insert("install_size".into(), json!(install_size));
    install.insert("is_dlc".into(), json!(metadata.get("mainGameItem").is_some_and(|m| !m.is_null())));
    put(&mut install, "version", info.and_then(|i| i.get("version")).cloned());
    put(&mut install, "platform", platform.clone().map(Value::String));
    g.insert("install".into(), Value::Object(install));
    g.insert("is_installed".into(), json!(info.is_some()));
    put(&mut g, "namespace", namespace.map(Value::String));
    g.insert(
        "is_mac_native".into(),
        json!(if info.is_some() { platform.as_deref() == Some("Mac") } else { metadata_platform.iter().any(|p| p == "Mac") }),
    );
    g.insert("save_folder".into(), json!(save_folder));
    put(&mut g, "save_path", info.and_then(|i| i.get("save_path")).cloned());
    g.insert("title".into(), json!(title));
    g.insert("canRunOffline".into(), json!(can_run_offline));
    put(&mut g, "thirdPartyManagedApp", third_party.clone().map(Value::String));
    let tp = third_party.as_deref().map(str::to_lowercase);
    g.insert("isEAManaged".into(), json!(matches!(tp.as_deref(), Some("origin") | Some("the ea app"))));
    g.insert("isUbisoftManaged".into(), json!(tp.as_deref() == Some("ubisoftconnect")));
    g.insert("is_linux_native".into(), json!(false));
    g.insert("runner".into(), json!("legendary"));
    g.insert("store_url".into(), json!(store_url));
    Some(Value::Object(g))
}

/// Every game of the account in Heroic's `GameInfo` format.
pub fn build_library(data_paths: &DataPaths) -> Vec<Value> {
    let installed = installed_games(data_paths);
    let mut names: Vec<String> = std::fs::read_dir(metadata_dir(data_paths))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter_map(|n| n.strip_suffix(".json").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut out = Vec::with_capacity(names.len());
    for app in names {
        let path = metadata_dir(data_paths).join(format!("{app}.json"));
        match json_store::read_value(&path) {
            Ok(file) => {
                if let Some(game) = game_info(&app, &file, installed.get(&app)) {
                    out.push(game);
                }
            }
            Err(err) => log::error!("Failed to parse metadata for {app}: {err:#}"),
        }
    }
    out
}

/// Rebuilds `store_cache/legendary_library.json` from legendary's files (no
/// network). Returns the number of games.
pub fn write_library(data_paths: &DataPaths) -> Result<usize> {
    let games = build_library(data_paths);
    let n = games.len();
    CacheStore::library(data_paths).set("library", Value::Array(games))?;
    Ok(n)
}

#[derive(Debug, Clone, Default)]
pub struct RefreshResult {
    pub games: usize,
    /// `legendary list` failed (offline, expired login ...); the library was
    /// still rebuilt from what legendary has on disk.
    pub list_error: Option<String>,
    /// Windows: the Epic Games Launcher had new games, taken in now.
    pub launcher_import: Option<crate::ops::OpResult>,
}

/// Seconds legendary waits for an answer of Epic's servers while it lists
/// the library (`--api-timeout`, since 0.20.23; its own default is 10). The
/// list of a big account, or any answer on a slow connection, takes longer
/// than that, and legendary ends on the first answer that does not come in
/// time. Its look-up of one game in the catalog keeps its own 10 seconds.
const API_TIMEOUT: &str = "60";

/// Lines of what legendary printed on stderr that go to the log when the
/// list fails: how far it got ("Fetching metadata for 778 app(s).") and the
/// traceback, which the message leaves out.
const LOGGED_LINES: usize = 40;

/// `legendary list --third-party`: `None` when it worked, else why not.
fn list_games(legendary: &Legendary) -> Option<String> {
    match legendary.run(&["--api-timeout", API_TIMEOUT, "list", "--third-party"], &[]) {
        Ok(out) if out.success() => None,
        Ok(out) => {
            let said = last_lines(&out.stderr, LOGGED_LINES);
            if !said.is_empty() {
                log::warn!("legendary list failed, the end of what it said:\n{said}");
            }
            Some(list_failure(&out.error_lines()))
        }
        Err(err) => Some(format!("{err:#}")),
    }
}

/// The message of a failed list, from its [`Output::error_lines`](crate::cmd::Output::error_lines).
fn list_failure(why: &str) -> String {
    if why.is_empty() {
        return "legendary list failed".to_string();
    }
    if rate_limited(why) {
        return format!("{}\n{why}", tr!("Epic is limiting requests for now. Try again in a few minutes."));
    }
    why.to_string()
}

/// Epic (its store's Cloudflare, which answers the achievements legendary
/// asks for each game) turned the requests away for a while: an HTTP 429 is
/// what legendary ended on (the last line; an error it logged and went on
/// from is not the reason).
fn rate_limited(why: &str) -> bool {
    why.lines().last().is_some_and(|end| end.contains("429 Client Error") || end.contains("Too Many Requests"))
}

/// The last `n` lines of `text`.
fn last_lines(text: &str, n: usize) -> &str {
    if n == 0 {
        return "";
    }
    let text = text.trim_end();
    match text.match_indices('\n').nth_back(n - 1) {
        Some((i, _)) => &text[i + 1..],
        None => text,
    }
}

/// The connection to Epic broke before the answer was whole (the errors of
/// `requests` legendary ends on). Not a server that cannot be reached, or
/// that did not answer in [`API_TIMEOUT`]: asking again would fail again,
/// after as long a wait.
fn connection_broke(why: &str) -> bool {
    ["ChunkedEncodingError", "ContentDecodingError", "Connection aborted", "Connection broken"].iter().any(|e| why.contains(e))
}

/// `refresh`: `legendary list --third-party`, then rebuild the library file.
pub fn refresh(data_paths: &DataPaths, legendary: &Legendary) -> Result<RefreshResult> {
    if !user::is_logged_in(data_paths) {
        return Ok(RefreshResult::default());
    }
    log::info!("Refreshing Epic Games...");
    let mut list_error = list_games(legendary);
    // legendary keeps what it fetched before the connection broke, so a
    // second run goes on from there.
    if let Some(e) = list_error.as_deref().filter(|e| connection_broke(e)) {
        log::warn!("legendary list lost the connection to Epic, trying once more: {e}");
        list_error = list_games(legendary);
    }
    if let Some(e) = &list_error {
        log::error!("Failed to refresh library: {e}");
    }
    // Windows: the games the Epic Games Launcher updated since, then the ones
    // it installed (with the metadata the list just fetched).
    if list_error.is_none() {
        crate::egl::retake_updated(data_paths, legendary);
    }
    let launcher_import = if list_error.is_none() { crate::egl::import_new(data_paths, legendary) } else { None };
    if let Some(r) = launcher_import.as_ref().filter(|r| !r.ok) {
        log::warn!("taking in the Epic Games Launcher's games: {}", r.message);
    }
    let games = write_library(data_paths)?;
    log::info!("Game list updated, got {games} games & DLCs");
    // With a fresh list legendary's token is fresh too. A failure here only
    // costs the play time, never the refresh.
    if list_error.is_none() {
        match crate::playtime::import(data_paths) {
            Ok(0) => {}
            Ok(n) => log::info!("Epic play time taken in for {n} game(s) never played here"),
            Err(err) => log::warn!("epic: reading the play time Epic counted: {err:#}"),
        }
    }
    Ok(RefreshResult { games, list_error, launcher_import })
}

/// App names whose installed version differs from the latest one in
/// `assets.json` (`listUpdateableGames` after `list` has refreshed it). Local
/// only; `assets.json` is as fresh as the last refresh.
pub fn updates_available(data_paths: &DataPaths) -> Vec<String> {
    let dir = data_paths.legendary_config_dir();
    let installed = json_store::read_value(&dir.join("installed.json")).unwrap_or(Value::Null);
    let assets = json_store::read_value(&dir.join("assets.json")).unwrap_or(Value::Null);
    let (Some(installed), Some(assets)) = (installed.as_object(), assets.as_object()) else { return Vec::new() };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (platform, list) in assets {
        let Some(list) = list.as_array() else { continue };
        for (app, data) in installed {
            if data.get("platform").and_then(Value::as_str) != Some(platform.as_str()) {
                continue;
            }
            let Some(asset) = list.iter().find(|a| a.get("app_name").and_then(Value::as_str) == Some(app)) else {
                continue;
            };
            let current = data.get("version").and_then(Value::as_str);
            let latest = asset.get("build_version").and_then(Value::as_str);
            if current != latest && seen.insert(app.clone()) {
                out.push(app.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::{fake_tool, legendary_of, Reply};
    use serde_json::json;

    fn meta(extra: Value) -> Value {
        let mut m = json!({
            "title": "Control", "namespace": "calluna", "developer": "Remedy", "description": "Desc",
            "categories": [{"path": "games"}, {"path": "applications"}],
            "keyImages": [
                {"type": "DieselGameBox", "url": "https://x/wide.jpg"},
                {"type": "DieselGameBoxTall", "url": "https://x/tall.jpg"},
                {"type": "DieselGameBoxLogo", "url": "https://x/logo.png"}
            ],
            "customAttributes": {
                "FolderName": {"type": "STRING", "value": "Control"},
                "CanRunOffline": {"type": "STRING", "value": "true"},
                "CloudSaveFolder": {"type": "STRING", "value": "{AppData}/Remedy/"}
            },
            "releaseInfo": [{"appId": "Calluna", "platform": ["Windows"]}]
        });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        json!({"app_name": "Calluna", "metadata": m})
    }

    #[test]
    fn not_installed_game_in_heroics_shape() {
        let g = game_info("Calluna", &meta(json!({})), None).unwrap();
        assert_eq!(g["art_cover"], "https://x/wide.jpg");
        assert_eq!(g["art_square"], "https://x/tall.jpg");
        assert_eq!(g["art_logo"], "https://x/logo.png");
        assert_eq!(g["cloud_save_enabled"], true);
        assert_eq!(g["folder_name"], "Control");
        assert_eq!(g["is_installed"], false);
        assert_eq!(g["install"], json!({"install_size": "0", "is_dlc": false}));
        assert_eq!(g["is_mac_native"], false);
        assert_eq!(g["store_url"], "https://www.epicgames.com/store/product/control");
        assert_eq!(g["extra"]["about"], json!({"description": "Desc", "shortDescription": ""}));
        assert_eq!(g["runner"], "legendary");
        assert!(g.get("save_path").is_none() && g.get("thirdPartyManagedApp").is_none());
        let keys: Vec<&String> = g.as_object().unwrap().keys().collect();
        assert_eq!(keys[..4], ["app_name", "art_cover", "art_logo", "art_square"]);
    }

    #[test]
    fn installed_game_takes_install_fields() {
        let inst = json!({"executable": "Control.exe", "install_path": "/g/Control", "install_size": 45010292103u64,
            "version": "1.33", "platform": "Windows", "save_path": null});
        let g = game_info("Calluna", &meta(json!({})), Some(&inst)).unwrap();
        assert_eq!(g["is_installed"], true);
        assert_eq!(g["install"]["install_size"], "41.92 GiB");
        assert_eq!(g["install"]["executable"], "Control.exe");
        assert_eq!(g["install"]["platform"], "Windows");
        assert!(g["save_path"].is_null());
        assert_eq!(g["is_mac_native"], false);
    }

    #[test]
    fn filtered_titles() {
        assert!(game_info("a", &meta(json!({"namespace": "ue"})), None).is_none());
        assert!(game_info("a", &meta(json!({"categories": [{"path": "plugins"}]})), None).is_none());
        assert!(game_info("a", &meta(json!({"categories": [{"path": "mods"}]})), None).is_none());
        assert!(game_info("a", &meta(json!({"releaseInfo": [{"platform": ["Android", "iOS"]}]})), None).is_none());
        assert!(game_info("a", &meta(json!({"releaseInfo": []})), None).is_none());
        // A release without platforms is not mobile-only.
        assert!(game_info("a", &meta(json!({"releaseInfo": [{"appId": "a"}]})), None).is_some());
    }

    #[test]
    fn dlc_and_third_party_flags() {
        let g = game_info(
            "d",
            &meta(json!({"mainGameItem": {"releaseInfo": [{"platform": ["Mac", "Windows"]}]},
                "releaseInfo": [{"appId": "d"}],
                "customAttributes": {"ThirdPartyManagedApp": {"type": "STRING", "value": "The EA App"}}})),
            None,
        )
        .unwrap();
        assert_eq!(g["install"]["is_dlc"], true);
        assert_eq!(g["is_mac_native"], true);
        assert_eq!(g["isEAManaged"], true);
        assert_eq!(g["isUbisoftManaged"], false);
        assert_eq!(g["thirdPartyManagedApp"], "The EA App");
        assert_eq!(g["folder_name"], "d");
    }

    fn setup(data_paths: &DataPaths) {
        let dir = data_paths.legendary_config_dir();
        std::fs::create_dir_all(dir.join("metadata")).unwrap();
        std::fs::write(dir.join("metadata/Calluna.json"), meta(json!({})).to_string()).unwrap();
        std::fs::write(dir.join("metadata/ue1.json"), meta(json!({"namespace": "ue"})).to_string()).unwrap();
        std::fs::write(dir.join("metadata/broken.json"), "{").unwrap();
        std::fs::write(
            dir.join("installed.json"),
            json!({"Calluna": {"app_name": "Calluna", "version": "1.0", "platform": "Windows", "install_path": "/g/Control", "executable": "c.exe", "install_size": 100}}).to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("user.json"), json!({"account_id": "1", "displayName": "P"}).to_string()).unwrap();
    }

    #[test]
    fn library_file_is_written_like_the_cache_store() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup(&data_paths);
        assert_eq!(write_library(&data_paths).unwrap(), 1);
        let raw = std::fs::read_to_string(data_paths.legendary_library()).unwrap();
        assert!(raw.starts_with("{\n\t\"library\": ["), "{raw}");
        let doc: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(doc["library"][0]["app_name"], "Calluna");
        assert_eq!(doc["library"][0]["is_installed"], true);
        assert!(doc["__timestamp"]["library"].is_string());
    }

    #[test]
    fn updates_compare_installed_and_assets() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup(&data_paths);
        let cfg = data_paths.legendary_config_dir();
        std::fs::write(cfg.join("assets.json"), json!({"Windows": [{"app_name": "Calluna", "build_version": "1.1"}, {"app_name": "Other", "build_version": "9"}], "Mac": [{"app_name": "Calluna", "build_version": "7"}]}).to_string()).unwrap();
        assert_eq!(updates_available(&data_paths), ["Calluna"]);
        std::fs::write(cfg.join("assets.json"), json!({"Windows": [{"app_name": "Calluna", "build_version": "1.0"}]}).to_string()).unwrap();
        assert!(updates_available(&data_paths).is_empty());
    }

    #[test]
    fn refresh_runs_list_then_rebuilds_even_when_list_fails() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup(&data_paths);
        let tool = fake_tool(dir.path()).otherwise(Reply::new().err("ERROR: offline").exit(1));
        let lg = legendary_of(&tool, dir.path());
        let r = refresh(&data_paths, &lg).unwrap();
        assert_eq!(r.games, 1);
        assert!(r.list_error.unwrap().contains("offline"));
        assert_eq!(tool.calls(), ["--api-timeout 60 list --third-party"]);
        // Logged out: nothing runs.
        std::fs::remove_file(data_paths.legendary_config_dir().join("user.json")).unwrap();
        assert_eq!(refresh(&data_paths, &lg).unwrap().games, 0);
        assert_eq!(tool.calls().len(), 1);
    }

    /// A crash of legendary: Python tracebacks, and no `ERROR` line.
    fn crash(exception: &str) -> Reply {
        Reply::new()
            .err("Traceback (most recent call last):")
            .err("  File \"urllib3/connectionpool.py\", line 534, in _make_request")
            .err("TimeoutError: The read operation timed out")
            .err("")
            .err("During handling of the above exception, another exception occurred:")
            .err("")
            .err("Traceback (most recent call last):")
            .err("  File \"legendary/api/egs.py\", line 173, in get_game_assets")
            .err(exception)
            .exit(1)
    }

    /// #449: Epic did not answer in time, and all the app said was
    /// "Traceback (most recent call last):".
    #[test]
    fn refresh_says_which_exception_ended_legendary() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup(&data_paths);
        let timeout =
            "requests.exceptions.ReadTimeout: HTTPSConnectionPool(host='x.epicgames.com', port=443): Read timed out. (read timeout=60)";
        let tool = fake_tool(dir.path()).otherwise(crash(timeout));
        let r = refresh(&data_paths, &legendary_of(&tool, dir.path())).unwrap();
        assert_eq!(r.list_error.as_deref(), Some(timeout));
        // Not asked again: that would be another minute for the same answer.
        assert_eq!(tool.calls().len(), 1);
        // What it had on disk is still the library.
        assert_eq!(r.games, 1);
    }

    #[test]
    fn refresh_asks_once_more_when_the_connection_broke() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup(&data_paths);
        let aborted = "requests.exceptions.ConnectionError: ('Connection aborted.', RemoteDisconnected('Remote end closed connection without response'))";
        let tool = fake_tool(dir.path()).otherwise(crash(aborted));
        let r = refresh(&data_paths, &legendary_of(&tool, dir.path())).unwrap();
        assert_eq!(r.list_error.as_deref(), Some(aborted));
        assert_eq!(tool.calls(), ["--api-timeout 60 list --third-party"; 2]);
    }

    /// The second run gets the list: nothing is reported.
    #[cfg(unix)]
    #[test]
    fn refresh_is_fine_when_the_second_list_works() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup(&data_paths);
        let once = dir.path().join("failed-once");
        let body = format!(
            "[ -e '{once}' ] && exit 0\ntouch '{once}'\necho 'Traceback (most recent call last):' >&2\necho \"requests.exceptions.ChunkedEncodingError: ('Connection broken: IncompleteRead(5 bytes read)')\" >&2\nexit 1",
            once = once.display()
        );
        let lg = crate::cmd::testing::fake(dir.path(), &body);
        let r = refresh(&data_paths, &lg).unwrap();
        assert_eq!(r.list_error, None);
        assert_eq!(crate::cmd::testing::calls(dir.path()).len(), 2);
    }

    #[test]
    fn only_a_connection_that_broke_is_worth_a_second_list() {
        assert!(connection_broke(
            "requests.exceptions.ConnectionError: ('Connection aborted.', ConnectionResetError(104, 'Connection reset by peer'))"
        ));
        assert!(connection_broke(
            "requests.exceptions.ContentDecodingError: ('Received response with content-encoding: gzip, but failed to decode it.')"
        ));
        // Epic cannot be reached, did not answer in time, or refused the login.
        assert!(!connection_broke(
            "requests.exceptions.ConnectionError: HTTPSConnectionPool(host='x', port=443): Max retries exceeded (Caused by NameResolutionError(\"Failed to resolve 'x'\"))"
        ));
        assert!(!connection_broke(
            "requests.exceptions.ReadTimeout: HTTPSConnectionPool(host='x', port=443): Read timed out. (read timeout=60)"
        ));
        assert!(!connection_broke("[cli] ERROR: Login failed, cannot continue!"));
    }

    /// Cloudflare's 429 on the achievements of a big library, right after the
    /// login: the message says to wait, then what legendary ended on.
    #[test]
    fn refresh_says_to_wait_when_epic_limits_the_requests() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup(&data_paths);
        let limited =
            "requests.exceptions.HTTPError: 429 Client Error: Too Many Requests for url: https://launcher.store.epicgames.com/graphql";
        let tool = fake_tool(dir.path()).otherwise(
            Reply::new()
                .err("[Core] INFO: Fetching metadata for 778 app(s).")
                .err("Traceback (most recent call last):")
                .err("  File \"legendary/api/egs.py\", line 266, in get_game_achievements")
                .err(limited)
                .exit(1),
        );
        let r = refresh(&data_paths, &legendary_of(&tool, dir.path())).unwrap();
        let why = r.list_error.unwrap();
        assert_eq!(why, format!("Epic is limiting requests for now. Try again in a few minutes.\n{limited}"));
        assert_eq!(tool.calls().len(), 1);
    }

    #[test]
    fn only_a_429_is_epic_limiting_the_requests() {
        assert!(rate_limited("requests.exceptions.HTTPError: 429 Client Error: Too Many Requests for url: https://x"));
        // No reason given (some proxies), or a reason alone.
        assert!(rate_limited("requests.exceptions.HTTPError: 429 Client Error:  for url: https://x"));
        assert!(rate_limited("requests.exceptions.HTTPError: Too Many Requests"));
        // A 429 legendary logged and went on from is not what ended it.
        assert!(!rate_limited(
            "[Core] ERROR: 429 Client Error: Too Many Requests\nrequests.exceptions.ReadTimeout: HTTPSConnectionPool(host='x', port=443): Read timed out. (read timeout=60)"
        ));
        assert!(!rate_limited("requests.exceptions.HTTPError: 504 Server Error: Gateway Timeout for url: https://x"));
        assert!(!rate_limited(
            "requests.exceptions.ReadTimeout: HTTPSConnectionPool(host='x', port=443): Read timed out. (read timeout=60)"
        ));
        assert_eq!(list_failure(""), "legendary list failed");
        assert_eq!(list_failure("[cli] ERROR: Login failed"), "[cli] ERROR: Login failed");
    }

    #[test]
    fn the_log_gets_the_last_lines() {
        assert_eq!(last_lines("a\nb\nc\n", 2), "b\nc");
        assert_eq!(last_lines("a\nb\nc", 3), "a\nb\nc");
        assert_eq!(last_lines("a\nb", 10), "a\nb");
        assert_eq!(last_lines("a\nb", 0), "");
        assert_eq!(last_lines("", 5), "");
        assert_eq!(last_lines("a\r\nb\r\nc\r\n", 2), "b\r\nc");
    }

    /// Compares the rebuilt library with the file Heronite wrote. Read-only:
    /// `HERONITE_EPIC_REAL=~/.local/share/heronite cargo test -p heronite-epic real_library -- --nocapture`
    #[test]
    fn real_library_matches_heronites_cache() {
        let Some(root) = std::env::var_os("HERONITE_EPIC_REAL") else { return };
        let data_paths = DataPaths::at(root);
        let cached = json_store::read_value(&data_paths.legendary_library()).unwrap();
        let cached = cached["library"].as_array().expect("library array").clone();
        let built = build_library(&data_paths);
        assert_eq!(built.len(), cached.len(), "game count");
        let by_name = |list: &[Value]| -> BTreeMap<String, Value> {
            list.iter().map(|g| (g["app_name"].as_str().unwrap().to_string(), g.clone())).collect()
        };
        let (b, c) = (by_name(&built), by_name(&cached));
        for (app, game) in &c {
            assert_eq!(b.get(app), Some(game), "{app} differs");
            // Same key order too (the map comparison ignores it).
            assert_eq!(serde_json::to_string(&b[app]).unwrap(), serde_json::to_string(game).unwrap(), "{app}: key order");
        }
    }
}
