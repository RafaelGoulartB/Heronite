//! Epic cloud saves (legendary's `sync-saves`): where a game keeps its saves
//! on this PC or in its Wine prefix, the saves in the account (one manifest
//! per upload, named by its time, with 1 MiB chunks), and moving them either
//! way.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{json, Value};
use sha1_smol::Sha1;

use crate::auth::Session;
use crate::chunk::{self, CHUNK_SIZE};
use crate::client::Client;
use crate::crypt::Secrets;
use crate::http::{host, Body, Request};
use crate::installed;
use crate::library::GameMeta;
use crate::manifest::{ChunkPart, FileManifest, Guid, Manifest};

/// Manifest names are the upload's time: `2026.11.03-12.30.00.manifest`.
const MANIFEST_TIME: &str = "%Y.%m.%d-%H.%M.%S";
/// Saves this close in time count as the same (legendary's guess).
const SAME_AGE_SECS: i64 = 60;

/// Where the game's Windows folders are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place<'a> {
    /// This PC is Windows.
    Windows,
    /// A Wine prefix (the folder with `user.reg` and `drive_c`).
    Prefix(&'a Path),
    /// A Mac build on a Mac.
    Mac,
}

/// Lexically removes `.` and `..`.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `path` with each folder that exists under another case written as it is
/// on disk (legendary's `case_insensitive_path_search`; Windows games name
/// folders in any case).
pub fn case_insensitive(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut parts = path.components().peekable();
    while let Some(c) = parts.next() {
        let candidate = out.join(c.as_os_str());
        if matches!(c, Component::RootDir | Component::Prefix(_)) || candidate.exists() {
            out = candidate;
            continue;
        }
        let wanted = c.as_os_str().to_string_lossy().to_lowercase();
        let found = std::fs::read_dir(&out)
            .ok()
            .and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).find(|n| n.to_string_lossy().to_lowercase() == wanted));
        match found {
            Some(name) => out.push(name),
            None => {
                out.push(c.as_os_str());
                for rest in parts {
                    out.push(rest.as_os_str());
                }
                break;
            }
        }
    }
    out
}

/// Resolves a save folder template of the catalog (`{AppData}/Remedy/Control`,
/// `{InstallDir}/Saves`, `{UserDir}/My Games/...`) for `place` (legendary's
/// `get_save_path`). A name left unresolved stays as `{Name}`.
pub fn resolve_save_path(template: &str, install_path: &str, account_id: &str, place: Place) -> Result<PathBuf> {
    let mut template = template.replace('\\', "/");
    let mut vars: HashMap<String, PathBuf> = HashMap::new();
    vars.insert("{installdir}".into(), PathBuf::from(install_path));
    vars.insert("{epicid}".into(), PathBuf::from(account_id));
    match place {
        Place::Windows => {
            let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
            let profile = env("USERPROFILE").or_else(heronite_platform_home).unwrap_or_default();
            vars.insert("{appdata}".into(), env("LOCALAPPDATA").unwrap_or_else(|| profile.join("AppData/Local")));
            vars.insert("{userdir}".into(), profile.join("Documents"));
            vars.insert("{userprofile}".into(), profile.clone());
            vars.insert("{usersavedgames}".into(), profile.join("Saved Games"));
        }
        Place::Mac => {
            let home = heronite_platform_home().unwrap_or_default();
            vars.insert("{appdata}".into(), home.join("Library/Application Support"));
            vars.insert("{userdir}".into(), home.join("Documents"));
            vars.insert("{userlibrary}".into(), home.join("Library"));
        }
        Place::Prefix(prefix) => {
            let folders = crate::wine_reg::shell_folders(prefix)?;
            let get = |k: &str| folders.get(k).cloned().ok_or_else(|| anyhow!("the prefix has no {k} folder"));
            vars.insert("{appdata}".into(), get("Local AppData")?);
            // The Documents folder's name depends on the language: follow Wine's link.
            let personal = get("Personal")?;
            vars.insert("{userdir}".into(), std::fs::canonicalize(&personal).unwrap_or(personal));
            if let Ok(saved) = get("{4C5C32FF-BB9D-43B0-B5B4-2D72E54EAAA4}") {
                vars.insert("{usersavedgames}".into(), saved);
            }
            // Old prefixes keep AppData outside `Roaming`: Epic's paths relative
            // to %LOCALAPPDATA% do not reach `LocalLow` / `Roaming` there.
            let appdata = get("AppData")?;
            if !appdata.to_string_lossy().to_lowercase().contains("roaming") {
                if let Ok(low) = get("{A520A1A4-1780-4FF6-BD18-167343C5AF16}") {
                    vars.insert("{locallow}".into(), low);
                }
                vars.insert("{roaming}".into(), appdata);
                let lower = template.to_lowercase();
                if let Some(i) = lower.find("locallow/") {
                    template = format!("{{locallow}}/{}", &template[i + 9..]);
                } else if let Some(i) = lower.find("roaming/") {
                    template = format!("{{roaming}}/{}", &template[i + 8..]);
                }
            }
        }
    }
    let mut path = PathBuf::new();
    for part in template.split('/') {
        match vars.get(&part.to_lowercase()) {
            Some(v) => path.push(v),
            None if part.is_empty() && path.as_os_str().is_empty() => path.push("/"),
            None => path.push(part),
        }
    }
    let path = normalize(&path);
    Ok(match place {
        Place::Prefix(_) => case_insensitive(&path),
        _ => path,
    })
}

fn heronite_platform_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The save folder template of `game` for `platform` (`CloudSaveFolder`, or
/// `CloudSaveFolder_MAC`).
pub fn save_folder_template<'a>(game: &'a GameMeta, platform: &str) -> Option<&'a str> {
    let key = if platform == "Mac" { "CloudSaveFolder_MAC" } else { "CloudSaveFolder" };
    game.custom_attribute(key).filter(|s| !s.is_empty())
}

/// The default save folder of an installed game, or why there is none.
pub fn default_save_path(config_dir: &Path, app_name: &str, account_id: &str, place: Place) -> Result<PathBuf> {
    let install = installed::get(config_dir, app_name).ok_or_else(|| anyhow!("{app_name} is not installed"))?;
    let game = GameMeta::load(config_dir, app_name).ok_or_else(|| anyhow!("the metadata of {app_name} is missing"))?;
    let template = save_folder_template(&game, &install.platform).ok_or_else(|| anyhow!("{app_name} has no cloud saves"))?;
    resolve_save_path(template, &install.install_path, account_id, place)
}

/// Whether a computed path still has names nothing resolved.
pub fn has_unresolved(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains('{') || s.contains('%')
}

/// One save in the account.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudSave {
    pub app_name: String,
    /// The manifest's path in the account.
    pub filename: String,
    /// `<time>.manifest`.
    pub manifest_name: String,
    /// When it was uploaded.
    pub time: Option<DateTime<Utc>>,
    /// Where it can be downloaded (signed, short-lived).
    pub read_link: String,
}

impl CloudSave {
    /// The upload time its name gives.
    pub fn name_time(&self) -> Option<DateTime<Utc>> {
        let stem = self.manifest_name.strip_suffix(".manifest")?;
        NaiveDateTime::parse_from_str(stem, MANIFEST_TIME).ok().map(|t| t.and_utc())
    }
}

/// How the local saves compare with the newest one in the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    LocalNewer,
    RemoteNewer,
    SameAge,
    NoSave,
    /// Both changed since the last sync (only known with a sync time).
    Conflict,
}

/// What a sync should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Upload or download, whichever side is newer.
    Both,
    ForceDownload,
    ForceUpload,
    /// Never upload (`--skip-upload`).
    DownloadOnly,
    /// Never download (`--skip-download`).
    UploadOnly,
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    Uploaded,
    Downloaded,
    UpToDate,
    NoSave,
    /// The newer side is the one the mode does not move.
    Skipped(SaveState),
    /// Both sides changed; a forced mode picks one.
    Conflict,
}

impl Client {
    fn savesync_url(&self, session: &Session, app_name: &str, manifests: bool) -> String {
        let mut url = format!("{}/api/v1/access/egstore/savesync/{}/", self.endpoints.datastorage, session.account_id());
        if !app_name.is_empty() {
            url.push_str(app_name);
            url.push_str(if manifests { "/manifests/" } else { "/" });
        }
        url
    }

    /// The account's saves of `app_name` (every game's for `""`).
    pub fn cloud_saves(&self, session: &Session, app_name: &str) -> Result<Vec<CloudSave>> {
        let url = self.savesync_url(session, app_name, !app_name.is_empty());
        let doc = self.http.get_json(&self.get(&url).bearer(session.access_token()))?;
        let mut out = Vec::new();
        for (name, f) in doc.get("files").and_then(Value::as_object).into_iter().flatten() {
            if !name.contains(".manifest") {
                continue;
            }
            let parts: Vec<&str> = name.split('/').collect();
            let (Some(app), Some(manifest)) = (parts.get(2), parts.get(4)) else { continue };
            let time = f
                .get("lastModified")
                .and_then(Value::as_str)
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc));
            out.push(CloudSave {
                app_name: app.to_string(),
                filename: name.clone(),
                manifest_name: manifest.to_string(),
                time,
                read_link: f.get("readLink").and_then(Value::as_str).unwrap_or_default().to_string(),
            });
        }
        out.sort_by_key(|s| s.time);
        Ok(out)
    }

    /// Links for `files` (paths relative to the game's folder in the
    /// account): `readLink` of those that exist, `writeLink` for uploads.
    fn save_links(&self, session: &Session, app_name: &str, files: &[String]) -> Result<serde_json::Map<String, Value>> {
        let url = self.savesync_url(session, app_name, false);
        let body = json!({"files": files});
        let doc = self.http.json(&self.post(&url, Body::Json(&body)).bearer(session.access_token()))?;
        Ok(doc.get("files").and_then(Value::as_object).cloned().unwrap_or_default())
    }

    /// Downloads `save` into `dest`. Once every file of the save is fetched
    /// and checked, the files of `dest` the game's filters cover are
    /// replaced by them, so the folder ends up like the save; a save that
    /// fails leaves `dest` untouched.
    pub fn download_save(&self, session: &Session, game: &GameMeta, save: &CloudSave, dest: &Path) -> Result<usize> {
        let resp = self.http.send(&Request::get(&save.read_link))?;
        if !resp.ok() || resp.body.is_empty() {
            bail!("the save {} could not be downloaded (HTTP {})", save.manifest_name, resp.status);
        }
        let m = Manifest::parse(&resp.body)?;
        let paths: Vec<String> = m.chunks.iter().map(|c| m.chunk_path(c)).collect();
        let links = self.save_links(session, &game.app_name, &paths)?;
        let mut chunks: HashMap<Guid, Vec<u8>> = HashMap::new();
        for (path, info) in paths.iter().zip(&m.chunks) {
            let link = links
                .iter()
                .find(|(k, _)| k.ends_with(path.as_str()))
                .and_then(|(_, v)| v.get("readLink").and_then(Value::as_str))
                .ok_or_else(|| anyhow!("a chunk of the save {} is missing in the account", save.manifest_name))?;
            let raw = self.http.bytes(link).with_context(|| format!("downloading a chunk from {}", host(link)))?;
            chunks.insert(info.guid, chunk::decode(&raw, &Secrets::new())?.data);
        }

        let (include, exclude) = filters(game);
        let mtime = save.name_time().map(SystemTime::from);
        apply_save(&m, &chunks, dest, include.as_deref(), exclude.as_deref(), mtime, &save.manifest_name)?;
        Ok(m.files.len())
    }

    /// Packs the saves in `dir` and uploads them as a new save of `game`.
    /// Returns how many files went (0: nothing matched the filters).
    pub fn upload_save(&self, session: &Session, game: &GameMeta, dir: &Path, time: DateTime<Utc>) -> Result<usize> {
        let (include, exclude) = filters(game);
        let pack = package(
            dir,
            &game.app_name,
            session.account_id(),
            game.custom_attribute("CloudSaveFolder").unwrap_or_default(),
            game.custom_attribute("CloudSaveFolder_MAC").unwrap_or_default(),
            include.as_deref(),
            exclude.as_deref(),
            time,
        )?;
        if pack.files == 0 {
            return Ok(0);
        }
        let names: Vec<String> = pack.blobs.keys().cloned().collect();
        let links = self.save_links(session, &game.app_name, &names)?;
        for (name, data) in &pack.blobs {
            let link = links
                .get(name)
                .and_then(|v| v.get("writeLink"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Epic gave no place to upload {name}"))?;
            let resp = self.http.send(&Request { method: "PUT", ..Request::post(link, Body::Bytes(data)) })?;
            if !resp.ok() {
                bail!("uploading the save to {} failed: HTTP {}", host(link), resp.status);
            }
        }
        Ok(pack.files)
    }

    /// One game's sync, like `legendary sync-saves <app> -y --accept-path`:
    /// the save folder (`save_path`, else the one stored, else the default,
    /// stored then), its state against the newest save in the account, and
    /// the upload or download `mode` allows.
    pub fn sync_saves(
        &self,
        config_dir: &Path,
        session: &Session,
        app_name: &str,
        save_path: Option<&str>,
        mode: SyncMode,
        place: Place,
    ) -> Result<(SyncOutcome, PathBuf)> {
        let mut install = installed::get(config_dir, app_name).ok_or_else(|| anyhow!("{app_name} is not installed"))?;
        let game = GameMeta::load(config_dir, app_name).ok_or_else(|| anyhow!("the metadata of {app_name} is missing"))?;
        if !game.supports_cloud_saves() && !game.supports_mac_cloud_saves() {
            bail!("{app_name} has no cloud saves");
        }
        let path = match save_path.filter(|p| !p.trim().is_empty()) {
            Some(p) => PathBuf::from(p),
            None => match install.save_path.clone().filter(|p| !p.is_empty()) {
                Some(p) => PathBuf::from(p),
                None => {
                    let p = default_save_path(config_dir, app_name, session.account_id(), place)?;
                    if has_unresolved(&p) {
                        bail!("The save folder {} has names that could not be resolved; set it by hand", p.display());
                    }
                    p
                }
            },
        };
        if install.save_path.as_deref() != Some(&*path.to_string_lossy()) {
            install.save_path = Some(path.to_string_lossy().into_owned());
            installed::set(config_dir, &install)?;
        }
        std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;

        let saves = self.cloud_saves(session, app_name)?;
        let latest = saves.iter().filter(|s| s.app_name == app_name).max_by_key(|s| s.time);
        let sync_time = install.rest.get("save_timestamp").and_then(Value::as_f64);
        let (state, local) = save_state(&path, latest, sync_time);
        let forced = matches!(mode, SyncMode::ForceDownload | SyncMode::ForceUpload);
        let outcome = match state {
            SaveState::NoSave if !forced => return Ok((SyncOutcome::NoSave, path)),
            SaveState::SameAge if !forced => return Ok((SyncOutcome::UpToDate, path)),
            SaveState::Conflict if !forced => return Ok((SyncOutcome::Conflict, path)),
            _ if mode == SyncMode::ForceDownload || (state == SaveState::RemoteNewer && mode != SyncMode::ForceUpload) => {
                if mode == SyncMode::UploadOnly {
                    return Ok((SyncOutcome::Skipped(state), path));
                }
                let save = latest.ok_or_else(|| anyhow!("there is no save of {app_name} in the account"))?;
                self.download_save(session, &game, save, &path)?;
                SyncOutcome::Downloaded
            }
            _ => {
                if mode == SyncMode::DownloadOnly {
                    return Ok((SyncOutcome::Skipped(state), path));
                }
                if self.upload_save(session, &game, &path, local.unwrap_or_else(Utc::now))? == 0 {
                    return Ok((SyncOutcome::NoSave, path));
                }
                SyncOutcome::Uploaded
            }
        };
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        install.rest.insert("save_timestamp".into(), json!(now));
        installed::set(config_dir, &install)?;
        Ok((outcome, path))
    }
}

/// The game's include and exclude patterns (`CloudIncludeList`,
/// `CloudExcludeList`, comma-separated).
fn filters(game: &GameMeta) -> (Option<Vec<String>>, Option<Vec<String>>) {
    let list = |k: &str| game.custom_attribute(k).map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
    (list("CloudIncludeList"), list("CloudExcludeList"))
}

/// Python's `fnmatch` (case-sensitive): `*`, `?` and `[...]`.
pub fn fnmatch(name: &str, pattern: &str) -> bool {
    fn go(n: &[char], p: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&n[i..], &p[1..])),
            Some('?') => !n.is_empty() && go(&n[1..], &p[1..]),
            Some('[') => {
                let Some(close) = p.iter().skip(1).position(|c| *c == ']').map(|i| i + 1) else {
                    return n.first() == Some(&'[') && go(&n[1..], &p[1..]);
                };
                let Some(c) = n.first() else { return false };
                let set = &p[1..close];
                let (negate, set) = match set.first() {
                    Some('!') => (true, &set[1..]),
                    _ => (false, set),
                };
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        hit |= set[i] <= *c && *c <= set[i + 2];
                        i += 3;
                    } else {
                        hit |= set[i] == *c;
                        i += 1;
                    }
                }
                hit != negate && go(&n[1..], &p[close + 1..])
            }
            Some(ch) => n.first() == Some(ch) && go(&n[1..], &p[1..]),
        }
    }
    go(&name.chars().collect::<Vec<_>>(), &pattern.chars().collect::<Vec<_>>())
}

/// legendary's `_filename_matches`: a folder prefix (`dir/`), a suffix, or a
/// glob.
fn matches_any(name: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| (p.ends_with('/') && name.starts_with(p.as_str())) || name.ends_with(p.as_str()) || fnmatch(name, p))
}

/// Files below `dir` (relative, with `/`) the filters let through.
fn list_files(dir: &Path, include: Option<&[String]>, exclude: Option<&[String]>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            if include.is_some_and(|i| !i.is_empty() && !matches_any(&rel, i)) {
                continue;
            }
            if exclude.is_some_and(|x| !x.is_empty() && matches_any(&rel, x)) {
                continue;
            }
            out.push(rel);
        }
    }
    out.sort_by_key(|s| s.to_lowercase());
    Ok(out)
}

/// A relative path of a save manifest that stays inside the save folder.
/// Writes the files of the save `m` (its decoded `chunks`) into `dest`.
/// Every file is put together and checked first; then the files of `dest`
/// the filters cover are deleted (legendary's clean folder) and the save's
/// moved in, so a broken save leaves `dest` as it was.
fn apply_save(
    m: &Manifest,
    chunks: &HashMap<Guid, Vec<u8>>,
    dest: &Path,
    include: Option<&[String]>,
    exclude: Option<&[String]>,
    mtime: Option<SystemTime>,
    name: &str,
) -> Result<()> {
    // Every file is put together and checked before anything local
    // changes: a save that is broken or does not fit leaves the saves on
    // disk as they were.
    if m.files.is_empty() {
        bail!("the save {} has no files", name);
    }
    let mut assembled = Vec::with_capacity(m.files.len());
    for f in &m.files {
        let rel = safe_relative(&f.filename)?;
        let mut data = Vec::with_capacity(f.size() as usize);
        for p in &f.chunk_parts {
            let c = chunks.get(&p.guid).ok_or_else(|| anyhow!("a chunk of {} is missing", f.filename))?;
            let (start, end) = (p.offset as usize, (p.offset + p.size) as usize);
            data.extend_from_slice(c.get(start..end).ok_or_else(|| anyhow!("a chunk of {} is too short", f.filename))?);
        }
        if f.hash != [0; 20] && Sha1::from(&data).digest().bytes() != f.hash {
            bail!("{} in the save {} is damaged", f.filename, name);
        }
        assembled.push((rel, data));
    }

    // Written beside their place first, then the files of `dest` the
    // game's filters cover are deleted (legendary's clean folder) and the
    // new ones moved in.
    std::fs::create_dir_all(dest)?;
    let old_files = list_files(dest, include, exclude)?;
    let mut written: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(assembled.len());
    let staged = (|| -> Result<()> {
        for (rel, data) in &assembled {
            let path = dest.join(rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut part = path.clone().into_os_string();
            part.push(".heronite-part");
            let part = PathBuf::from(part);
            std::fs::write(&part, data).with_context(|| format!("writing {}", part.display()))?;
            written.push((part, path));
        }
        Ok(())
    })();
    if let Err(err) = staged {
        for (part, _) in &written {
            let _ = std::fs::remove_file(part);
        }
        return Err(err);
    }
    for old in old_files {
        let _ = std::fs::remove_file(dest.join(&old));
    }
    for (part, path) in &written {
        std::fs::rename(part, path).with_context(|| format!("writing {}", path.display()))?;
        if let Some(t) = mtime {
            if let Ok(file) = std::fs::File::options().write(true).open(path) {
                let _ = file.set_modified(t);
            }
        }
    }
    Ok(())
}

fn safe_relative(name: &str) -> Result<PathBuf> {
    let p = Path::new(name);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) || name.is_empty() {
        bail!("refusing the save file name {name:?}");
    }
    Ok(p.to_path_buf())
}

/// The newest change of any file below `dir`.
fn newest_change(dir: &Path) -> Option<DateTime<Utc>> {
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(e.path());
            } else if let Ok(t) = meta.modified() {
                newest = Some(newest.map_or(t, |n| n.max(t)));
            }
        }
    }
    newest.map(DateTime::<Utc>::from)
}

/// The local saves in `dir` against the newest one in the account
/// (legendary's `check_savegame_state`); also the local time.
pub fn save_state(dir: &Path, save: Option<&CloudSave>, last_sync: Option<f64>) -> (SaveState, Option<DateTime<Utc>>) {
    let local = newest_change(dir);
    let remote = save.and_then(|s| s.name_time().or(s.time));
    match (local, remote) {
        (None, None) => (SaveState::NoSave, None),
        (Some(_), None) => (SaveState::LocalNewer, local),
        (None, Some(_)) => (SaveState::RemoteNewer, None),
        (Some(l), Some(r)) => {
            if let Some(sync) = last_sync.filter(|t| *t > 0.0).and_then(|t| DateTime::from_timestamp(t as i64, 0)) {
                if (r - sync).num_seconds() > SAME_AGE_SECS && (l - sync).num_seconds() > SAME_AGE_SECS {
                    return (SaveState::Conflict, local);
                }
            }
            let state = if (l - r).num_seconds().abs() < SAME_AGE_SECS {
                SaveState::SameAge
            } else if l > r {
                SaveState::LocalNewer
            } else {
                SaveState::RemoteNewer
            };
            (state, local)
        }
    }
}

/// A save packed for upload.
#[derive(Debug, Default)]
pub struct Package {
    /// Path in the account (`manifests/<time>.manifest`, chunk paths) → bytes.
    pub blobs: BTreeMap<String, Vec<u8>>,
    pub files: usize,
}

/// Packs the files of `dir` into 1 MiB chunks and a manifest the way
/// legendary's `SaveGameHelper` does (one stream of every file, sorted).
pub fn package(
    dir: &Path,
    app_name: &str,
    account_id: &str,
    cloud_folder: &str,
    cloud_folder_mac: &str,
    include: Option<&[String]>,
    exclude: Option<&[String]>,
    time: DateTime<Utc>,
) -> Result<Package> {
    let mut out = Package::default();
    let files = list_files(dir, include, exclude)?;
    if files.is_empty() {
        return Ok(out);
    }
    let build = time.format(MANIFEST_TIME).to_string();
    let mut m = Manifest::new(&format!("{app_name}{account_id}"), &build);
    m.set_custom_field("CloudSaveFolder", cloud_folder);
    if !cloud_folder_mac.is_empty() {
        m.set_custom_field("CloudSaveFolder_MAC", cloud_folder_mac);
    }
    let mut buf: Vec<u8> = Vec::with_capacity(CHUNK_SIZE);
    let mut guid = Guid::random();
    let flush = |buf: &mut Vec<u8>, guid: &mut Guid, m: &mut Manifest, out: &mut Package| -> Result<()> {
        let (raw, info) = chunk::encode(*guid, buf)?;
        out.blobs.insert(info.path(18), raw);
        m.chunks.push(info);
        buf.clear();
        *guid = Guid::random();
        Ok(())
    };
    for rel in &files {
        let data = std::fs::read(dir.join(rel)).with_context(|| format!("reading {rel}"))?;
        let mut f = FileManifest { filename: rel.clone(), hash: Sha1::from(&data).digest().bytes(), ..Default::default() };
        let mut at = 0;
        while at < data.len() {
            let take = (CHUNK_SIZE - buf.len()).min(data.len() - at);
            f.chunk_parts.push(ChunkPart { guid, offset: buf.len() as u32, size: take as u32 });
            buf.extend_from_slice(&data[at..at + take]);
            at += take;
            if buf.len() >= CHUNK_SIZE {
                flush(&mut buf, &mut guid, &mut m, &mut out)?;
            }
        }
        m.files.push(f);
    }
    if !buf.is_empty() {
        flush(&mut buf, &mut guid, &mut m, &mut out)?;
    }
    out.blobs.insert(format!("manifests/{build}.manifest"), m.to_bytes()?);
    out.files = files.len();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wine prefixes live on Unix hosts, whose file names are case-sensitive.
    #[cfg(unix)]
    #[test]
    fn templates_resolve_in_a_wine_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let pfx = dir.path();
        std::fs::write(crate::wine_reg::user_reg(pfx), crate::wine_reg::tests::USER_REG).unwrap();
        std::fs::create_dir_all(pfx.join("drive_c/users/steamuser/AppData/Local/Remedy")).unwrap();
        let p = resolve_save_path("{AppData}/remedy/Control/{EpicID}", "/games/x", "acc", Place::Prefix(pfx)).unwrap();
        assert_eq!(p, pfx.join("drive_c/users/steamuser/AppData/Local/Remedy/Control/acc"), "existing folders keep their case");
        let p = resolve_save_path("{AppData}/../LocalLow/Dev/Game", "", "", Place::Prefix(pfx)).unwrap();
        assert_eq!(p, pfx.join("drive_c/users/steamuser/AppData/LocalLow/Dev/Game"));
        let p = resolve_save_path("{InstallDir}\\Saves", "/games/x", "", Place::Prefix(pfx)).unwrap();
        assert_eq!(p, PathBuf::from("/games/x/Saves"));
        let p = resolve_save_path("{Unknown}/x", "", "", Place::Prefix(pfx)).unwrap();
        assert!(has_unresolved(&p));
        let p = resolve_save_path("{UserSavedGames}/G", "", "", Place::Prefix(pfx)).unwrap();
        assert_eq!(p, pfx.join("drive_c/users/steamuser/Saved Games/G"));
    }

    fn save_of(dir: &Path) -> (Manifest, HashMap<Guid, Vec<u8>>) {
        let pack = package(dir, "App", "acc", "{AppData}/G", "", None, None, Utc::now()).unwrap();
        let (_, bytes) = pack.blobs.iter().find(|(k, _)| k.starts_with("manifests/")).unwrap();
        let m = Manifest::parse(bytes).unwrap();
        let chunks = m.chunks.iter().map(|c| (c.guid, chunk::decode(&pack.blobs[&c.path(18)], &Secrets::new()).unwrap().data)).collect();
        (m, chunks)
    }

    #[test]
    fn a_broken_save_leaves_the_local_saves_alone() {
        let cloud = tempfile::tempdir().unwrap();
        std::fs::write(cloud.path().join("slot1.sav"), b"cloud one").unwrap();
        std::fs::create_dir_all(cloud.path().join("sub")).unwrap();
        std::fs::write(cloud.path().join("sub/slot2.sav"), b"cloud two").unwrap();
        let (m, mut chunks) = save_of(cloud.path());
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(dest.path().join("slot1.sav"), b"local one").unwrap();
        std::fs::write(dest.path().join("old.sav"), b"local old").unwrap();

        // A damaged chunk: nothing local changes.
        let mut bad = chunks.clone();
        for data in bad.values_mut() {
            data[0] ^= 0xff;
        }
        assert!(apply_save(&m, &bad, dest.path(), None, None, None, "x").is_err());
        assert_eq!(std::fs::read(dest.path().join("slot1.sav")).unwrap(), b"local one");
        assert!(dest.path().join("old.sav").exists());
        // A missing chunk: the same.
        let mut missing = chunks.clone();
        missing.clear();
        assert!(apply_save(&m, &missing, dest.path(), None, None, None, "x").is_err());
        assert!(dest.path().join("old.sav").exists());
        // A save with no files never cleans the folder.
        let mut empty = m.clone();
        empty.files.clear();
        assert!(apply_save(&empty, &chunks, dest.path(), None, None, None, "x").is_err());
        assert!(dest.path().join("old.sav").exists());
        assert_eq!(list_files(dest.path(), None, None).unwrap(), ["old.sav", "slot1.sav"]);

        // A good save replaces the folder's saves.
        apply_save(&m, &std::mem::take(&mut chunks), dest.path(), None, None, None, "x").unwrap();
        assert_eq!(std::fs::read(dest.path().join("slot1.sav")).unwrap(), b"cloud one");
        assert_eq!(std::fs::read(dest.path().join("sub/slot2.sav")).unwrap(), b"cloud two");
        assert!(!dest.path().join("old.sav").exists());
        assert_eq!(list_files(dest.path(), None, None).unwrap().len(), 2);
    }

    #[test]
    fn globs_like_fnmatch() {
        assert!(fnmatch("save1.sav", "*.sav"));
        assert!(fnmatch("a/b.cfg", "a/?.cfg"));
        assert!(fnmatch("slot3", "slot[0-9]"));
        assert!(!fnmatch("slotx", "slot[0-9]"));
        assert!(fnmatch("slotx", "slot[!0-9]"));
        assert!(!fnmatch("x.SAV", "*.sav"));
        assert!(matches_any("Config/x.ini", &["Config/".into()]));
        assert!(matches_any("deep/x.ini", &[".ini".into()]));
    }

    #[test]
    fn saves_pack_into_chunks_and_a_manifest_that_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Slots")).unwrap();
        std::fs::write(dir.path().join("Slots/a.sav"), vec![1u8; CHUNK_SIZE + 10]).unwrap();
        std::fs::write(dir.path().join("b.cfg"), b"cfg").unwrap();
        std::fs::write(dir.path().join("skip.log"), b"log").unwrap();
        let time = DateTime::parse_from_rfc3339("2026-11-03T12:30:00Z").unwrap().with_timezone(&Utc);
        let exclude = vec!["*.log".to_string()];
        let pack = package(dir.path(), "Calluna", "acc", "{AppData}/Remedy", "", None, Some(&exclude), time).unwrap();
        assert_eq!(pack.files, 2);
        let manifest = &pack.blobs["manifests/2026.11.03-12.30.00.manifest"];
        let m = Manifest::parse(manifest).unwrap();
        assert_eq!(m.meta.app_name, "Callunaacc");
        assert_eq!(m.custom_field("CloudSaveFolder"), Some("{AppData}/Remedy"));
        assert_eq!(m.files.iter().map(|f| f.filename.as_str()).collect::<Vec<_>>(), ["b.cfg", "Slots/a.sav"]);
        assert_eq!(m.chunks.len(), 2, "1 MiB + 13 bytes need two chunks");
        assert_eq!(pack.blobs.len(), 3);
        // Every file reads back from its chunks.
        let mut chunks = HashMap::new();
        for c in &m.chunks {
            chunks.insert(c.guid, chunk::decode(&pack.blobs[&m.chunk_path(c)], &Secrets::new()).unwrap().data);
        }
        for f in &m.files {
            let mut data = Vec::new();
            for p in &f.chunk_parts {
                data.extend_from_slice(&chunks[&p.guid][p.offset as usize..(p.offset + p.size) as usize]);
            }
            assert_eq!(Sha1::from(&data).digest().bytes(), f.hash, "{}", f.filename);
        }
        let empty = package(dir.path(), "C", "a", "", "", Some(&["*.nothing".to_string()]), None, time).unwrap();
        assert_eq!(empty.files, 0);
    }

    #[test]
    fn states_by_time() {
        let dir = tempfile::tempdir().unwrap();
        let save = |name: &str| CloudSave {
            app_name: "A".into(),
            filename: String::new(),
            manifest_name: name.into(),
            time: None,
            read_link: String::new(),
        };
        assert_eq!(save_state(dir.path(), None, None).0, SaveState::NoSave);
        assert_eq!(save_state(dir.path(), Some(&save("2020.01.01-00.00.00.manifest")), None).0, SaveState::RemoteNewer);
        std::fs::write(dir.path().join("s"), b"x").unwrap();
        assert_eq!(save_state(dir.path(), None, None).0, SaveState::LocalNewer);
        assert_eq!(save_state(dir.path(), Some(&save("2020.01.01-00.00.00.manifest")), None).0, SaveState::LocalNewer);
        assert_eq!(save_state(dir.path(), Some(&save("2099.01.01-00.00.00.manifest")), None).0, SaveState::RemoteNewer);
        let now = Utc::now().format(MANIFEST_TIME).to_string();
        assert_eq!(save_state(dir.path(), Some(&save(&format!("{now}.manifest"))), None).0, SaveState::SameAge);
        assert_eq!(save_state(dir.path(), Some(&save("2099.01.01-00.00.00.manifest")), Some(1.0)).0, SaveState::Conflict);
        assert!(safe_relative("../x").is_err());
        assert!(safe_relative("a/b").is_ok());
    }
}
