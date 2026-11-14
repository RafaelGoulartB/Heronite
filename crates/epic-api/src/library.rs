//! Refreshing the account's games (what `legendary list --third-party`
//! leaves on disk): `assets.json` with the builds of each platform, and one
//! `metadata/<app>.json` per game and DLC with its catalog entry. Catalog
//! entries are fetched again only for builds that changed.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::api::Asset;
use crate::auth::Session;
use crate::client::Client;
use crate::files;

/// Catalog requests at once (legendary's thread pool).
const WORKERS: usize = 16;
/// An app name of a broken old entry that every account's library has.
const BROKEN_APP: &str = "1";

pub fn metadata_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("metadata")
}

pub fn metadata_file(config_dir: &Path, app_name: &str) -> PathBuf {
    metadata_dir(config_dir).join(format!("{app_name}.json"))
}

pub fn assets_file(config_dir: &Path) -> PathBuf {
    config_dir.join("assets.json")
}

/// `assets.json`: platform → builds.
pub fn read_assets(config_dir: &Path) -> BTreeMap<String, Vec<Asset>> {
    files::read_json(&assets_file(config_dir)).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

/// The stored metadata of a game (legendary's `Game`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameMeta {
    pub app_name: String,
    pub app_title: String,
    /// Platform → build.
    pub asset_infos: BTreeMap<String, Asset>,
    pub base_urls: Vec<String>,
    /// The catalog entry.
    pub metadata: Value,
    /// `{"config": {...}, "rev": n}`.
    pub sidecar: Option<Value>,
    /// Keys this type does not name (kept on every rewrite).
    pub rest: Map<String, Value>,
}

impl GameMeta {
    pub fn from_value(v: Value) -> Option<Self> {
        let Value::Object(mut doc) = v else { return None };
        let take_text =
            |doc: &mut Map<String, Value>, k: &str| doc.remove(k).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        let app_name = take_text(&mut doc, "app_name");
        if app_name.is_empty() {
            return None;
        }
        let app_title = take_text(&mut doc, "app_title");
        let mut asset_infos: BTreeMap<String, Asset> =
            doc.remove("asset_infos").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        // Files of old legendary versions have one `asset_info` (Windows).
        if let Some(old) = doc.remove("asset_info").and_then(|v| serde_json::from_value::<Asset>(v).ok()) {
            asset_infos.entry("Windows".into()).or_insert(old);
        }
        let base_urls = doc.remove("base_urls").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let metadata = doc.remove("metadata").unwrap_or_else(|| json!({}));
        let sidecar = doc.remove("sidecar").filter(|v| !v.is_null());
        Some(Self { app_name, app_title, asset_infos, base_urls, metadata, sidecar, rest: doc })
    }

    pub fn to_value(&self) -> Value {
        let mut doc = self.rest.clone();
        doc.insert("app_name".into(), json!(self.app_name));
        doc.insert("app_title".into(), json!(self.app_title));
        doc.insert("asset_infos".into(), serde_json::to_value(&self.asset_infos).unwrap_or_else(|_| json!({})));
        doc.insert("base_urls".into(), json!(self.base_urls));
        doc.insert("metadata".into(), self.metadata.clone());
        doc.insert("sidecar".into(), self.sidecar.clone().unwrap_or(Value::Null));
        Value::Object(doc)
    }

    pub fn load(config_dir: &Path, app_name: &str) -> Option<Self> {
        files::read_json(&metadata_file(config_dir, app_name)).and_then(Self::from_value)
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        files::write_json(&metadata_file(config_dir, &self.app_name), &self.to_value(), false)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.metadata.get("customAttributes")?.get(name)?.get("value")?.as_str()
    }

    /// A custom attribute of the catalog entry (`CloudSaveFolder`, `FolderName`...).
    pub fn custom_attribute(&self, name: &str) -> Option<&str> {
        self.attr(name)
    }

    pub fn namespace(&self) -> &str {
        self.metadata.get("namespace").and_then(Value::as_str).unwrap_or_default()
    }

    pub fn catalog_item_id(&self) -> &str {
        self.metadata.get("id").and_then(Value::as_str).unwrap_or_default()
    }

    pub fn is_dlc(&self) -> bool {
        self.metadata.get("mainGameItem").is_some_and(|m| !m.is_null())
    }

    /// The store app that installs and runs the game (`Origin`, `the EA app`,
    /// `UbisoftConnect`), when it is one of theirs.
    pub fn third_party_store(&self) -> Option<&str> {
        self.attr("ThirdPartyManagedApp").or_else(|| self.attr("ThirdPartyManagedProvider")).filter(|s| !s.is_empty())
    }

    pub fn is_origin_game(&self) -> bool {
        self.third_party_store().is_some_and(|s| matches!(s.to_lowercase().as_str(), "origin" | "the ea app"))
    }

    pub fn is_ubisoft_game(&self) -> bool {
        self.third_party_store().is_some_and(|s| s.eq_ignore_ascii_case("ubisoftconnect"))
    }

    pub fn supports_cloud_saves(&self) -> bool {
        self.attr("CloudSaveFolder").is_some()
    }

    pub fn supports_mac_cloud_saves(&self) -> bool {
        self.attr("CloudSaveFolder_MAC").is_some()
    }

    /// Arguments the catalog adds to the game's command line.
    pub fn additional_command_line(&self) -> Option<&str> {
        self.attr("AdditionalCommandLine")
    }

    /// A DLC that is started itself (`addons/launchable`).
    pub fn is_launchable_addon(&self) -> bool {
        self.metadata
            .get("categories")
            .and_then(Value::as_array)
            .is_some_and(|c| c.iter().any(|c| c.get("path").and_then(Value::as_str) == Some("addons/launchable")))
    }

    pub fn app_version(&self, platform: &str) -> Option<&str> {
        self.asset_infos.get(platform).map(|a| a.build_version.as_str())
    }
}

/// What a refresh did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefreshReport {
    /// Games and DLCs with assets.
    pub assets: usize,
    /// Catalog entries fetched now.
    pub fetched: usize,
    /// Metadata files deleted (games no longer owned).
    pub pruned: usize,
    /// Games whose catalog entry could not be fetched: `(app, why)`. Their old
    /// metadata, if any, stays.
    pub failures: Vec<(String, String)>,
}

/// Options of [`refresh`].
#[derive(Debug, Clone, Default)]
pub struct RefreshOptions {
    /// Fetch every catalog entry again (`--force-refresh`).
    pub force: bool,
    /// Platforms to list besides Windows and those of installed games.
    pub platforms: Vec<String>,
}

/// Platforms of `installed.json`'s games.
fn installed_platforms(config_dir: &Path) -> BTreeSet<String> {
    crate::installed::read_all(config_dir).values().map(|g| g.platform.clone()).collect()
}

/// Refreshes `assets.json` and the metadata files from Epic. Fails only when
/// the Windows builds cannot be listed (offline, refused sign-in); a catalog
/// entry that fails is reported and the rest goes on.
pub fn refresh(client: &Client, config_dir: &Path, session: &Session, opts: &RefreshOptions) -> Result<RefreshReport> {
    let mut report = RefreshReport::default();
    let mut platforms: BTreeSet<String> = installed_platforms(config_dir);
    platforms.insert("Windows".into());
    platforms.extend(opts.platforms.iter().cloned());

    let old_assets = read_assets(config_dir);
    let mut assets = old_assets.clone();
    for platform in &platforms {
        match client.assets(session, platform, "Live") {
            Ok(list) => {
                assets.insert(platform.clone(), list);
            }
            Err(err) if platform == "Windows" => return Err(err).context("listing the Epic games"),
            Err(err) => report.failures.push((format!("<{platform}>"), format!("{err:#}"))),
        }
    }
    if assets != old_assets {
        files::write_json(&assets_file(config_dir), &serde_json::to_value(&assets)?, false)?;
    }

    // App → platform → build, without Unreal Engine assets.
    let mut by_app: BTreeMap<String, BTreeMap<String, Asset>> = BTreeMap::new();
    for (platform, list) in &assets {
        for a in list {
            by_app.entry(a.app_name.clone()).or_default().insert(platform.clone(), a.clone());
        }
    }
    by_app.retain(|_, builds| !builds.values().any(|a| a.namespace == "ue"));
    report.assets = by_app.len();

    let mut jobs = Vec::new();
    for (app, builds) in &by_app {
        let old = GameMeta::load(config_dir, app);
        let windows_rev = builds.get("Windows").map_or(0, |a| a.sidecar_rev);
        let sidecar_changed = windows_rev > 0
            && old.as_ref().is_none_or(|g| g.sidecar.as_ref().and_then(|s| s.get("rev")).and_then(Value::as_i64) != Some(windows_rev));
        let build_changed = old.as_ref().is_none_or(|g| builds.iter().any(|(p, a)| g.app_version(p) != Some(a.build_version.as_str())));
        if opts.force || old.is_none() || build_changed || sidecar_changed {
            jobs.push(Job { app: app.clone(), builds: builds.clone(), old, sidecar: sidecar_changed || windows_rev > 0 && opts.force });
        }
    }
    let fetched = run_jobs(client, config_dir, session, jobs, &mut report.failures);
    report.fetched += fetched;

    // Games the library has without a build here: those of the EA app and
    // Ubisoft Connect, which their store installs.
    let windows: HashSet<&str> = assets.get("Windows").map(|l| l.iter().map(|a| a.app_name.as_str()).collect()).unwrap_or_default();
    match client.library_items(session) {
        Ok(items) => {
            for item in items {
                let text = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                let (app, namespace, catalog) = (text("appName"), text("namespace"), text("catalogItemId"));
                if app.is_empty()
                    || app == BROKEN_APP
                    || windows.contains(app.as_str())
                    || namespace == "ue"
                    || text("sandboxType") == "PRIVATE"
                {
                    continue;
                }
                if !opts.force && metadata_file(config_dir, &app).exists() {
                    continue;
                }
                match client.catalog_item(session, &namespace, &catalog) {
                    Ok(meta) => {
                        let meta = meta.unwrap_or_else(|| json!({"title": "Unknown"}));
                        let game = GameMeta {
                            app_name: app.clone(),
                            app_title: meta.get("title").and_then(Value::as_str).unwrap_or("Unknown").to_string(),
                            metadata: meta,
                            ..GameMeta::load(config_dir, &app).unwrap_or_default()
                        };
                        match game.save(config_dir) {
                            Ok(()) => report.fetched += 1,
                            Err(e) => report.failures.push((app, format!("{e:#}"))),
                        }
                    }
                    Err(e) => report.failures.push((app, format!("{e:#}"))),
                }
            }
        }
        Err(err) => report.failures.push(("<library>".into(), format!("{err:#}"))),
    }

    if report.fetched > 0 {
        report.pruned = prune(config_dir, &assets, &platforms);
    }
    Ok(report)
}

struct Job {
    app: String,
    builds: BTreeMap<String, Asset>,
    old: Option<GameMeta>,
    sidecar: bool,
}

/// Fetches the catalog entries of `jobs` on [`WORKERS`] threads and writes
/// their metadata files. Returns how many were written.
fn run_jobs(client: &Client, config_dir: &Path, session: &Session, jobs: Vec<Job>, failures: &mut Vec<(String, String)>) -> usize {
    if jobs.is_empty() {
        return 0;
    }
    log::info!("epic: fetching the catalog entries of {} game(s)", jobs.len());
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..WORKERS.min(jobs.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(job) = jobs.get(i) else { break };
                match fetch_one(client, config_dir, session, job) {
                    Ok(()) => {
                        done.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(e) => errors.lock().push((job.app.clone(), format!("{e:#}"))),
                }
            });
        }
    });
    failures.extend(errors.into_inner());
    done.into_inner()
}

fn fetch_one(client: &Client, config_dir: &Path, session: &Session, job: &Job) -> Result<()> {
    let first = job.builds.get("Windows").or_else(|| job.builds.values().next()).expect("an app has a build");
    let meta = client.catalog_item(session, &first.namespace, &first.catalog_item_id)?.unwrap_or_else(|| {
        log::warn!("epic: {} has no catalog entry", job.app);
        json!({"title": "Unknown"})
    });
    let mut game = job.old.clone().unwrap_or_default();
    game.app_name = job.app.clone();
    game.app_title = meta.get("title").and_then(Value::as_str).unwrap_or("Unknown").to_string();
    game.metadata = meta;
    game.asset_infos = job.builds.clone();
    if job.sidecar {
        match client.manifest_info(session.access_token(), &first.namespace, &first.catalog_item_id, &job.app, "Windows", "Live") {
            Ok(info) => game.sidecar = info.sidecar,
            Err(e) => log::warn!("epic: the sidecar of {}: {e:#}", job.app),
        }
    }
    game.save(config_dir)
}

/// Deletes the metadata of games no longer owned, but those installed and
/// those of a third-party store (legendary's `_prune_metadata`).
fn prune(config_dir: &Path, assets: &BTreeMap<String, Vec<Asset>>, platforms: &BTreeSet<String>) -> usize {
    let available: HashSet<&str> =
        platforms.iter().filter_map(|p| assets.get(p)).flat_map(|l| l.iter().map(|a| a.app_name.as_str())).collect();
    let installed = crate::installed::read_all(config_dir);
    let mut removed = 0;
    for app in stored_apps(config_dir) {
        if available.contains(app.as_str()) || installed.contains_key(&app) {
            continue;
        }
        if GameMeta::load(config_dir, &app).is_some_and(|g| g.third_party_store().is_some()) {
            continue;
        }
        if std::fs::remove_file(metadata_file(config_dir, &app)).is_ok() {
            log::debug!("epic: dropped the metadata of {app}");
            removed += 1;
        }
    }
    removed
}

/// App names with a metadata file.
pub fn stored_apps(config_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(metadata_dir(config_dir))
        .map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// The DLCs of a game among the stored metadata (by the game's catalog id).
pub fn dlcs_of(config_dir: &Path, game: &GameMeta) -> Vec<GameMeta> {
    let id = game.catalog_item_id();
    stored_apps(config_dir)
        .into_iter()
        .filter_map(|a| GameMeta::load(config_dir, &a))
        .filter(|g| g.metadata.pointer("/mainGameItem/id").and_then(Value::as_str) == Some(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Endpoints;
    use serde_json::json;

    #[test]
    fn metadata_files_keep_legendarys_shape_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let doc = json!({
            "app_name": "Calluna", "app_title": "Control", "base_urls": ["https://cdn/x"],
            "asset_infos": {"Windows": {"app_name": "Calluna", "asset_id": "Calluna", "build_version": "1.33", "catalog_item_id": "c",
                "label_name": "Live-Windows", "metadata": {}, "namespace": "calluna", "sidecar_rev": 0}},
            "metadata": {"id": "c", "namespace": "calluna", "title": "Control",
                "customAttributes": {"CloudSaveFolder": {"type": "STRING", "value": "{AppData}/Remedy"}, "ThirdPartyManagedApp": {"value": ""}}},
            "sidecar": null, "achievements": {"total_achievements": 3}
        });
        let g = GameMeta::from_value(doc.clone()).unwrap();
        assert_eq!(g.app_version("Windows"), Some("1.33"));
        assert!(g.supports_cloud_saves());
        assert_eq!(g.third_party_store(), None);
        assert_eq!((g.namespace(), g.catalog_item_id()), ("calluna", "c"));
        g.save(dir.path()).unwrap();
        let back = files::read_json(&metadata_file(dir.path(), "Calluna")).unwrap();
        assert_eq!(back, doc);
        assert_eq!(stored_apps(dir.path()), ["Calluna"]);
    }

    #[test]
    fn old_single_asset_files_are_read() {
        let g = GameMeta::from_value(json!({"app_name": "A", "asset_info": {"app_name": "A", "build_version": "2"}})).unwrap();
        assert_eq!(g.app_version("Windows"), Some("2"));
        assert!(GameMeta::from_value(json!({"app_title": "x"})).is_none());
    }

    #[test]
    fn third_party_and_dlc_flags() {
        let g = GameMeta::from_value(json!({"app_name": "A", "metadata": {
            "customAttributes": {"ThirdPartyManagedProvider": {"value": "UbisoftConnect"}},
            "mainGameItem": {"id": "base"}, "categories": [{"path": "addons/launchable"}]
        }}))
        .unwrap();
        assert!(g.is_ubisoft_game() && !g.is_origin_game());
        assert!(g.is_dlc() && g.is_launchable_addon());
    }

    /// A refresh against a local server that answers by path.
    #[test]
    fn refresh_writes_assets_and_metadata_and_prunes_what_is_gone() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let calls = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or_default().to_string();
                seen.lock().push(path.clone());
                let body = if path.starts_with("/launcher/api/public/assets/Windows") {
                    json!([{"appName": "Game", "assetId": "Game", "buildVersion": "2", "catalogItemId": "cg", "labelName": "Live-Windows", "namespace": "ns"},
                           {"appName": "Engine", "buildVersion": "1", "catalogItemId": "ce", "namespace": "ue"}])
                    .to_string()
                } else if path.contains("/bulk/items") && path.contains("id=cg") {
                    json!({"cg": {"id": "cg", "namespace": "ns", "title": "The Game"}}).to_string()
                } else if path.contains("/bulk/items") {
                    json!({"cx": {"id": "cx", "namespace": "nx", "title": "EA Game", "customAttributes": {"ThirdPartyManagedApp": {"value": "Origin"}}}}).to_string()
                } else if path.starts_with("/library/api/public/items") {
                    json!({"records": [{"appName": "EAGame", "namespace": "nx", "catalogItemId": "cx", "sandboxType": "PUBLIC"},
                                       {"appName": "Game", "namespace": "ns", "catalogItemId": "cg"},
                                       {"appName": "Secret", "namespace": "np", "catalogItemId": "cp", "sandboxType": "PRIVATE"}],
                           "responseMetadata": {}})
                    .to_string()
                } else {
                    "{}".to_string()
                };
                let mut s = stream;
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        // A game no longer owned, and one still owned at an older build.
        GameMeta { app_name: "Gone".into(), app_title: "Gone".into(), metadata: json!({}), ..Default::default() }.save(cfg).unwrap();
        let session = Session::from_value(json!({"access_token": "t", "account_id": "a"})).unwrap();
        let client = Client::new(Endpoints::all_at(&url));
        let report = refresh(&client, cfg, &session, &RefreshOptions::default()).unwrap();
        assert_eq!(report.assets, 1, "Unreal Engine assets are skipped");
        assert_eq!(report.fetched, 2);
        assert_eq!(report.pruned, 1);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        let game = GameMeta::load(cfg, "Game").unwrap();
        assert_eq!((game.app_title.as_str(), game.app_version("Windows")), ("The Game", Some("2")));
        assert!(GameMeta::load(cfg, "EAGame").unwrap().is_origin_game());
        assert!(!metadata_file(cfg, "Gone").exists());
        assert!(!metadata_file(cfg, "Secret").exists());
        assert_eq!(read_assets(cfg)["Windows"].len(), 2);

        // Nothing changed: only the lists are asked again.
        calls.lock().clear();
        let report = refresh(&client, cfg, &session, &RefreshOptions::default()).unwrap();
        assert_eq!(report.fetched, 0);
        assert!(calls.lock().iter().all(|p| !p.contains("/bulk/items")), "{:?}", calls.lock());
    }
}
