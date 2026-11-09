//! What this PC sends to the cloud (Settings → Cloud), measured here only:
//! the account is never asked. The bytes of each kind by game or part, by
//! the same rules as the uploads, how much the screenshots grow a month, so
//! the user knows how much room the account needs, and whether each kind is
//! all sent, partly or not at all (against `sent`, what went up last).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use heronite_backup::catalog::{self, Class, Side};
use heronite_backup::Part;
use heronite_data::{AppPaths, DataPaths};

use crate::sent::{self, Status};

/// A PNG becomes a WebP about this big (lossless) before it goes up.
const PNG_TO_WEBP: f64 = 0.6;
/// A backup's zip against the Collection and settings it holds (measured:
/// mostly JSON, already dense).
const ZIP_RATIO: f64 = 0.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Library,
    Backups,
    Saves,
    Captures,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Library, Kind::Backups, Kind::Saves, Kind::Captures];
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    /// Bytes by kind, then by game (screenshots, saves) or part (backups:
    /// `zip`, `art`).
    pub shares: BTreeMap<Kind, BTreeMap<String, u64>>,
    /// Bytes of screenshots taken a month (the last year's average).
    pub monthly: u64,
    /// How much of each kind is in the cloud.
    pub status: BTreeMap<Kind, Status>,
}

impl Usage {
    pub fn total(&self, kind: Kind) -> u64 {
        self.shares.get(&kind).map(|m| m.values().sum()).unwrap_or(0)
    }

    fn add(&mut self, kind: Kind, name: &str, bytes: u64) {
        *self.shares.entry(kind).or_default().entry(name.to_owned()).or_default() += bytes;
    }
}

/// Where the things that go up are on this PC (the app's settings say).
#[derive(Debug, Clone, Default)]
pub struct Local<'a> {
    pub captures: Option<&'a Path>,
    /// PNGs still wait for their compression.
    pub pending_png: bool,
    /// With the screenshots, the replays (videos).
    pub replays: bool,
    pub saves: Option<&'a Path>,
    /// Backups kept (0: all, counted as one).
    pub backups_kept: u32,
    /// The newest backup's folder name, if there is one.
    pub latest_backup: Option<String>,
}

/// Measures what each kind would take in the cloud (blocking: walks the
/// folders; no network).
pub fn measure(data: &DataPaths, app: &AppPaths, local: &Local) -> Usage {
    let mut usage = Usage::default();
    let destination = crate::prefs::load(app).map(|p| p.destination()).unwrap_or_default();
    let sent = sent::load(app, &destination);
    usage.status.insert(Kind::Library, library_status(data, app));
    // The library: this PC's own file, mostly its play sessions.
    let own = app.sync_mirror_dir().join("outbox").join(crate::format::STATE_FILE);
    let library = fs::metadata(&own).or_else(|_| fs::metadata(data.sessions())).map(|m| m.len()).unwrap_or(0);
    usage.add(Kind::Library, "", library);

    let zip = backup_zip_estimate(data, app);
    usage.add(Kind::Backups, "zip", zip * u64::from(local.backups_kept.max(1)));
    let mut art = Vec::new();
    for dir in ["art", "metadata-art"] {
        for f in walk(&data.root.join("local_library").join(dir)) {
            usage.add(Kind::Backups, "art", f.size);
            art.push((format!("{dir}/{}", f.rel), f.size));
        }
    }
    let backups = match &local.latest_backup {
        None => Status::Empty,
        Some(_) if sent.backups.is_empty() => Status::Nothing,
        Some(latest) => {
            let zip_left = !sent.backups.contains(latest);
            match (sent::compare(art.iter().map(|(p, b)| (p.as_str(), *b)), &sent.art), zip_left) {
                (Status::Behind { files, bytes }, _) => {
                    Status::Behind { files: files + u64::from(zip_left), bytes: bytes + if zip_left { zip } else { 0 } }
                }
                (_, true) => Status::Behind { files: 1, bytes: zip },
                _ => Status::Done,
            }
        }
    };
    usage.status.insert(Kind::Backups, backups);

    let mut saves = Vec::new();
    if let Some(folder) = local.saves {
        for f in walk(folder).into_iter().filter(|f| !f.rel.ends_with(".tmp")) {
            usage.add(Kind::Saves, first(&f.rel), f.size);
            saves.push((f.rel, f.size));
        }
    }
    usage.status.insert(Kind::Saves, sent::compare(saves.iter().map(|(p, b)| (p.as_str(), *b)), &sent.saves));

    let mut captures = Vec::new();
    if let Some(folder) = local.captures {
        let year_ago = SystemTime::now() - Duration::from_secs(365 * 24 * 3600);
        let mut last_year = 0u64;
        for f in walk(folder) {
            let Some(size) = capture_size(&f.rel, f.size, local.replays, local.pending_png) else { continue };
            if f.mtime.is_some_and(|t| t >= year_ago) {
                last_year += size;
            }
            usage.add(Kind::Captures, first(&f.rel), size);
            // A PNG waiting for its compression is not up yet either.
            captures.push((f.rel, f.size));
        }
        usage.monthly = last_year / 12;
    }
    usage.status.insert(Kind::Captures, sent::compare(captures.iter().map(|(p, b)| (p.as_str(), *b)), &sent.captures));
    usage
}

/// The library: never synced, changed here since the last sync (or it
/// failed), or synced.
fn library_status(data: &DataPaths, app: &AppPaths) -> Status {
    let Ok(p) = crate::prefs::load(app) else { return Status::Nothing };
    let Some(last) = p.last_sync_at.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) else {
        return Status::Nothing;
    };
    let last = SystemTime::from(last);
    // The files of what syncs that the app writes for nothing else (the
    // favourites' `config.json` is written for the window too).
    let files = [data.sessions(), data.timestamps(), data.local_meta(), data.ratings()];
    let changed = files.iter().filter_map(|f| fs::metadata(f).and_then(|m| m.modified()).ok()).any(|t| t > last);
    if p.last_error.is_some() || changed {
        Status::Behind { files: 0, bytes: 0 }
    } else {
        Status::Done
    }
}

/// Whether the capture at `rel` goes up as it is now (the rules of
/// `cloud::upload_captures`). A PNG waiting for its compression does not:
/// its WebP will.
pub(crate) fn capture_goes(rel: &str, replays: bool, pending_png: bool) -> bool {
    let lower = rel.to_ascii_lowercase();
    if lower.starts_with(".heronite-replay/") || [".tmp", ".partial", ".part"].iter().any(|e| lower.ends_with(e)) {
        return false;
    }
    let video = [".mp4", ".mkv", ".webm", ".mov"].iter().any(|e| lower.ends_with(e)) || lower.contains("/replays/");
    !(video && !replays) && !(pending_png && lower.ends_with(".png"))
}

/// What a capture at `rel` takes once it goes up, or `None` when it never
/// does.
fn capture_size(rel: &str, size: u64, replays: bool, pending_png: bool) -> Option<u64> {
    if capture_goes(rel, replays, false) && pending_png && rel.to_ascii_lowercase().ends_with(".png") {
        return Some((size as f64 * PNG_TO_WEBP) as u64);
    }
    capture_goes(rel, replays, pending_png).then_some(size)
}

/// About what one backup zip of this PC takes (its Collection and settings).
fn backup_zip_estimate(data: &DataPaths, app: &AppPaths) -> u64 {
    let mut bytes = 0;
    for (side, root) in [(Side::Data, &data.root), (Side::Config, &app.config)] {
        for found in catalog::walk(root, side, false) {
            if matches!(found.class, Class::In(Part::Collection | Part::Settings)) {
                bytes += found.size;
            }
        }
    }
    (bytes as f64 * ZIP_RATIO) as u64
}

/// The first part of a `/`-separated path (the game's folder).
fn first(rel: &str) -> &str {
    rel.split('/').next().unwrap_or(rel)
}

pub(crate) struct Found {
    /// `/`-separated, relative to the walked folder.
    pub rel: String,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

/// Every file under `root` (links not followed).
pub(crate) fn walk(root: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(meta) = e.path().symlink_metadata() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if meta.is_dir() {
                stack.push((e.path(), rel));
            } else if meta.is_file() {
                out.push(Found { rel, size: meta.len(), mtime: meta.modified().ok() });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_count_like_the_upload_sends_them() {
        assert_eq!(capture_size("Elden Ring/a.webp", 100, false, true), Some(100));
        assert_eq!(capture_size("Elden Ring/a.png", 100, false, true), Some(60));
        assert_eq!(capture_size("Elden Ring/a.png", 100, false, false), Some(100));
        assert_eq!(capture_size("Elden Ring/replays/c.mp4", 100, false, true), None);
        assert_eq!(capture_size("Elden Ring/replays/c.mp4", 100, true, true), Some(100));
        assert_eq!(capture_size(".heronite-replay/buf.mp4", 100, true, true), None);
    }

    #[test]
    fn what_this_pc_sends_is_measured_here() {
        let dir = tempfile::tempdir().unwrap();
        let caps = dir.path().join("caps");
        let saves = dir.path().join("saves");
        for (path, bytes) in
            [("caps/Hades/a.webp", 10usize), ("caps/Hades/b.webp", 30), ("caps/Celeste/c.png", 100), ("saves/Hades/backup.zip", 7)]
        {
            let p = dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![0u8; bytes]).unwrap();
        }
        let data = DataPaths::at(dir.path().join("data"));
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let local = Local { captures: Some(&caps), pending_png: true, saves: Some(&saves), ..Default::default() };
        let usage = measure(&data, &app, &local);
        assert_eq!(usage.status[&Kind::Captures], Status::Nothing);
        assert_eq!(usage.status[&Kind::Backups], Status::Empty);
        let destination = crate::prefs::load(&app).unwrap().destination();
        sent::update(&app, &destination, |s| {
            s.captures.insert("Hades/a.webp".into(), 10);
            s.captures.insert("Hades/b.webp".into(), 30);
        })
        .unwrap();
        // The PNG still waits for its compression: not up yet.
        assert_eq!(measure(&data, &app, &local).status[&Kind::Captures], Status::Behind { files: 1, bytes: 100 });
        assert_eq!(usage.total(Kind::Captures), 10 + 30 + 60);
        assert_eq!(usage.shares[&Kind::Captures]["Celeste"], 60);
        assert_eq!(usage.total(Kind::Saves), 7);
        assert_eq!(usage.monthly, 100 / 12);
    }
}
