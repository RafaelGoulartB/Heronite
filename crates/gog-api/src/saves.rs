//! Cloud saves, as gogdl's `save-sync` did them: GOG's cloud storage keeps
//! a game's files under its own client id, each location in a folder of its
//! own (`__default` unless the game names others). Which way to sync is
//! decided by the time of the last sync: files changed since then on one
//! side only go to the other; changes on both sides are a conflict left to
//! the user (a forced upload or download).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use md5::{Digest, Md5};
use serde_json::Value;

use crate::api::{self, Client};
use crate::auth::Credentials;

/// The hash the cloud gives a file it deleted.
const DELETED: &str = "aadd86936a80ee8a369579c3926f1b3c";

/// The game's own client, which its cloud saves belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameClient {
    pub client_id: String,
    pub client_secret: String,
}

impl Client {
    /// The client of `game_id`, from the meta of its newest `os` build.
    pub fn game_client(&self, game_id: &str, os: &str) -> Result<GameClient> {
        let builds = self.get_json(&api::builds_url(game_id, os, 2))?;
        let link = builds.pointer("/items/0/link").and_then(Value::as_str).ok_or_else(|| anyhow!("the game has no builds for {os}"))?;
        let (meta, _) = self.get_zlib_json(link)?;
        let text = |k: &str| meta.get(k).and_then(api::id_text);
        match (text("clientId"), text("clientSecret")) {
            (Some(client_id), Some(client_secret)) => Ok(GameClient { client_id, client_secret }),
            _ => Err(anyhow!("the game has no cloud saves")),
        }
    }
}

/// Which way the user wants a sync to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preference {
    /// Whichever side changed (`""`).
    Both,
    /// Only upload (`--skip-download`).
    Upload,
    /// Only download (`--skip-upload`).
    Download,
    /// Replace the cloud's files (`--force-upload`).
    ForceUpload,
    /// Replace the local files (`--force-download`).
    ForceDownload,
}

impl Preference {
    /// From gogdl's flag.
    pub fn from_flag(flag: Option<&str>) -> Option<Self> {
        Some(match flag.unwrap_or("") {
            "" => Self::Both,
            "--skip-download" => Self::Upload,
            "--skip-upload" => Self::Download,
            "--force-upload" => Self::ForceUpload,
            "--force-download" => Self::ForceDownload,
            _ => return None,
        })
    }
}

/// What a sync decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Download,
    Upload,
    Conflict,
    None,
}

/// A save file on either side.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveFile {
    /// `/` separated, below the location.
    pub relative: String,
    /// Seconds since the epoch.
    pub updated: f64,
    /// The cloud's md5 (of the gzip data); empty for local files.
    pub md5: String,
}

/// The files to move one way and the files to delete on the other side.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Classified {
    pub updated_local: Vec<SaveFile>,
    pub updated_cloud: Vec<SaveFile>,
    pub missing_locally: Vec<SaveFile>,
    pub missing_remotely: Vec<SaveFile>,
}

impl Classified {
    pub fn action(&self) -> Action {
        match (self.updated_local.is_empty(), self.updated_cloud.is_empty()) {
            (true, false) => Action::Download,
            (false, true) => Action::Upload,
            (true, true) => Action::None,
            (false, false) => Action::Conflict,
        }
    }
}

/// gogdl's classification against the time of the last sync.
pub fn classify(local: &[SaveFile], cloud: &[SaveFile], last_sync: f64) -> Classified {
    let mut out = Classified::default();
    for f in local {
        if !cloud.iter().any(|c| c.relative == f.relative) {
            out.missing_remotely.push(f.clone());
        }
        if f.updated > last_sync {
            out.updated_local.push(f.clone());
        }
    }
    for f in cloud.iter().filter(|c| c.md5 != DELETED) {
        if !local.iter().any(|l| l.relative == f.relative) {
            out.missing_locally.push(f.clone());
        }
        if f.updated > last_sync {
            out.updated_cloud.push(f.clone());
        }
    }
    out
}

/// The result of a sync.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub action: Action,
    /// The time to store as the last sync (seconds since the epoch, as gogdl
    /// printed it); the old one when nothing was allowed to happen.
    pub timestamp: String,
    /// What was done, one line each, for the user.
    pub log: Vec<String>,
}

/// An ISO 8601 time (with or without a zone; without is UTC) in seconds.
pub fn parse_time(text: &str) -> Option<f64> {
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return Some(t.timestamp_millis() as f64 / 1000.);
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|t| t.and_utc().timestamp_millis() as f64 / 1000.)
}

fn iso(seconds: f64) -> String {
    DateTime::<Utc>::from_timestamp(seconds as i64, 0).unwrap_or_default().to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.)
}

fn mtime(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as f64)
        .unwrap_or(0.)
}

/// Every file below `dir`, relative and `/` separated.
pub fn local_files(dir: &Path) -> Vec<SaveFile> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<SaveFile>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                let relative = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                out.push(SaveFile { relative, updated: mtime(&path), md5: String::new() });
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    out
}

/// The cloud's files of the folder `name` (`[{name, hash, last_modified}]`).
pub fn parse_cloud_list(doc: &Value, name: &str) -> Vec<SaveFile> {
    let prefix = format!("{name}/");
    doc.as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let relative = f.get("name")?.as_str()?.strip_prefix(&prefix)?.to_string();
            if !is_safe_relative(&relative) {
                log::warn!("gog: skipping the cloud save file {relative:?}: not a path inside the save folder");
                return None;
            }
            Some(SaveFile {
                relative,
                updated: f.get("last_modified").and_then(Value::as_str).and_then(parse_time).unwrap_or(0.),
                md5: f.get("hash").and_then(Value::as_str).unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Whether a cloud file name is a plain path below the save folder: not
/// empty, not absolute, no drive, no `.`/`..`, no NUL. The names come from
/// the server, and a save is written where its name says.
pub fn is_safe_relative(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\0')
        && name.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
        && Path::new(name).components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// gzip like Python's `gzip.compress(data, 6, mtime=0)`.
pub fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut z = flate2::GzBuilder::new().mtime(0).operating_system(255).write(Vec::new(), flate2::Compression::new(6));
    z.write_all(data)?;
    Ok(z.finish()?)
}

/// A location's cloud storage, for one game and account.
pub struct Storage<'a> {
    client: Client,
    base: String,
    name: &'a str,
}

impl<'a> Storage<'a> {
    /// The folder `name` of the game's client `game` with its tokens.
    pub fn new(creds: &Credentials, game: &GameClient, name: &'a str) -> Self {
        let client = Client::with_token(&creds.access_token).user_agent(api::GALAXY_AGENT);
        Self { client, base: format!("{}/v1/{}/{}", api::CLOUD_STORAGE, creds.user_id, game.client_id), name }
    }

    fn url(&self, relative: &str) -> String {
        format!("{}/{}/{}", self.base, self.name, api::encode(relative, true))
    }

    pub fn list(&self) -> Result<Vec<SaveFile>> {
        let resp = self.client.get(&self.base, &[("Accept", "application/json")])?;
        if resp.status == 404 {
            return Ok(Vec::new());
        }
        api::check(&resp, &self.base)?;
        Ok(parse_cloud_list(&resp.json()?, self.name))
    }

    pub fn upload(&self, dir: &Path, file: &SaveFile) -> Result<()> {
        let data = std::fs::read(dir.join(&file.relative)).with_context(|| format!("reading {}", file.relative))?;
        let compressed = gzip(&data)?;
        let etag = hex::encode(Md5::digest(&compressed));
        let modified = iso(file.updated);
        let url = self.url(&file.relative);
        let headers = [
            ("X-Object-Meta-LocalLastModified", modified.as_str()),
            ("Etag", etag.as_str()),
            ("Content-Encoding", "gzip"),
            ("X-Object-Meta-User-Agent", api::GALAXY_AGENT),
        ];
        let resp = self.client.put(&url, &compressed, &headers)?;
        api::check(&resp, &url).with_context(|| format!("uploading {}", file.relative))
    }

    pub fn download(&self, dir: &Path, file: &SaveFile) -> Result<PathBuf> {
        let url = self.url(&file.relative);
        let resp = self.client.get(&url, &[])?;
        api::check(&resp, &url).with_context(|| format!("downloading {}", file.relative))?;
        // The body arrives inflated when the server says it is gzip; a stored
        // gzip without that header is inflated here.
        let data = if resp.body.starts_with(&[0x1f, 0x8b]) {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(&resp.body[..]).read_to_end(&mut out)?;
            out
        } else {
            resp.body.clone()
        };
        if !is_safe_relative(&file.relative) {
            return Err(anyhow!("refusing the save file name {:?}", file.relative));
        }
        let path = dir.join(&file.relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &data).with_context(|| format!("writing {}", path.display()))?;
        if let Some(t) = resp.header("x-object-meta-locallastmodified").and_then(parse_time) {
            if let Ok(f) = std::fs::File::options().write(true).open(&path) {
                let _ = f.set_modified(UNIX_EPOCH + Duration::from_secs_f64(t.max(0.)));
            }
        }
        Ok(path)
    }

    pub fn delete(&self, file: &SaveFile) -> Result<()> {
        let url = self.url(&file.relative);
        let resp = self.client.delete(&url)?;
        if resp.status != 404 {
            api::check(&resp, &url)?;
        }
        Ok(())
    }
}

/// Syncs `dir` with the cloud folder of `storage`. `last_sync` is the
/// stored time of the last sync (`"0"` for never).
pub fn sync(storage: &Storage, dir: &Path, last_sync: &str, preference: Preference) -> Result<Outcome> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let local = local_files(dir);
    let cloud = storage.list()?;
    let downloadable: Vec<SaveFile> = cloud.iter().filter(|c| c.md5 != DELETED).cloned().collect();
    let mut log = vec![format!("Local files: {}", local.len()), format!("Files in cloud: {}", cloud.len())];
    let done = |action, log| Outcome { action, timestamp: format!("{:.6}", now()), log };

    if !local.is_empty() && cloud.is_empty() {
        log.push("No files in cloud, uploading".into());
        for f in &local {
            storage.upload(dir, f)?;
        }
        return Ok(done(Action::Upload, log));
    }
    if local.is_empty() && !cloud.is_empty() {
        log.push("No files locally, downloading".into());
        for f in &downloadable {
            storage.download(dir, f)?;
        }
        return Ok(done(Action::Download, log));
    }

    let last: f64 = last_sync.trim().parse().unwrap_or(0.);
    let mut c = classify(&local, &cloud, last);
    let mut action = c.action();
    match preference {
        Preference::ForceUpload => {
            log.push("Forcing upload".into());
            c.updated_local = local.clone();
            action = Action::Upload;
        }
        Preference::ForceDownload => {
            log.push("Forcing download".into());
            c.updated_cloud = downloadable;
            action = Action::Download;
        }
        Preference::Upload if action == Action::Download => {
            log.push("Refused to upload files, newer files in the cloud".into());
            return Ok(Outcome { action: Action::None, timestamp: last_sync.to_string(), log });
        }
        Preference::Download if action == Action::Upload => {
            log.push("Refused to download files, newer files locally".into());
            return Ok(Outcome { action: Action::None, timestamp: last_sync.to_string(), log });
        }
        _ => {}
    }
    match action {
        Action::Upload => {
            log.push("Uploading files".into());
            for f in &c.updated_local {
                storage.upload(dir, f)?;
            }
            for f in &c.missing_locally {
                log.push(format!("Deleting in the cloud: {}", f.relative));
                storage.delete(f)?;
            }
        }
        Action::Download => {
            log.push("Downloading files".into());
            for f in &c.updated_cloud {
                storage.download(dir, f)?;
            }
            for f in &c.missing_remotely {
                log.push(format!("Deleting locally: {}", f.relative));
                let _ = std::fs::remove_file(dir.join(&f.relative));
            }
        }
        Action::Conflict => log.push("Files in conflict: force downloading or uploading of files".into()),
        Action::None => log.push("Nothing to do".into()),
    }
    Ok(done(action, log))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn f(rel: &str, updated: f64) -> SaveFile {
        SaveFile { relative: rel.into(), updated, md5: "x".into() }
    }

    #[test]
    fn cloud_names_outside_the_save_folder_are_skipped() {
        let doc = json!([
            {"name": "__default/ok/a.sav", "hash": "h", "last_modified": "2024-01-01T00:00:00+00:00"},
            {"name": "__default/../../../.bashrc", "hash": "h"},
            {"name": "__default//home/u/.profile", "hash": "h"},
            {"name": "__default/a/./b", "hash": "h"},
            {"name": "__default/", "hash": "h"},
        ]);
        let names: Vec<String> = parse_cloud_list(&doc, "__default").into_iter().map(|f| f.relative).collect();
        assert_eq!(names, ["ok/a.sav"]);
        assert!(!is_safe_relative("C:/x") || cfg!(unix));
        assert!(!is_safe_relative("a\0b"));
    }

    #[test]
    fn changes_on_one_side_go_to_the_other() {
        let local = [f("a", 50.), f("b", 200.)];
        let cloud = [f("a", 60.), f("c", 70.)];
        let c = classify(&local, &cloud, 100.);
        assert_eq!(c.action(), Action::Upload);
        assert_eq!(c.missing_locally.iter().map(|f| f.relative.as_str()).collect::<Vec<_>>(), ["c"]);
        assert_eq!(classify(&local, &[f("a", 300.)], 100.).action(), Action::Conflict);
        assert_eq!(classify(&[f("a", 1.)], &[f("a", 300.)], 100.).action(), Action::Download);
        assert_eq!(classify(&[f("a", 1.)], &[f("a", 2.)], 100.).action(), Action::None);
        let deleted = SaveFile { md5: DELETED.into(), ..f("gone", 500.) };
        assert_eq!(classify(&[f("a", 1.)], &[deleted], 100.).action(), Action::None);
    }

    #[test]
    fn cloud_lists_times_and_flags() {
        let doc = json!([
            {"name": "__default/slot1.sav", "hash": "h1", "last_modified": "2024-01-02T03:04:05+00:00"},
            {"name": "__default/sub/slot2.sav", "hash": "h2", "last_modified": "2024-01-02T03:04:05.250000+00:00"},
            {"name": "other/x", "hash": "h3", "last_modified": "2024-01-02T03:04:05+00:00"}
        ]);
        let files = parse_cloud_list(&doc, "__default");
        assert_eq!(files.iter().map(|f| f.relative.as_str()).collect::<Vec<_>>(), ["slot1.sav", "sub/slot2.sav"]);
        assert_eq!(files[0].updated, 1704164645.);
        assert_eq!(files[1].updated, 1704164645.25);
        assert_eq!(parse_time("2024-01-02T03:04:05"), Some(1704164645.));
        assert_eq!(iso(1704164645.), "2024-01-02T03:04:05+00:00");
        assert_eq!(Preference::from_flag(Some("--skip-upload")), Some(Preference::Download));
        assert_eq!(Preference::from_flag(None), Some(Preference::Both));
        assert_eq!(Preference::from_flag(Some("--bogus")), None);
    }

    #[test]
    fn gzip_is_python_shaped_and_local_files_are_listed() {
        let z = gzip(b"save data").unwrap();
        assert_eq!(&z[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255]);
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&z[..]).read_to_end(&mut out).unwrap();
        assert_eq!(out, b"save data");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b"), "1").unwrap();
        std::fs::write(dir.path().join("a"), "2").unwrap();
        let files = local_files(dir.path());
        assert_eq!(files.iter().map(|f| f.relative.as_str()).collect::<Vec<_>>(), ["a", "sub/b"]);
        assert!(files[0].updated > 1.6e9);
    }
}
