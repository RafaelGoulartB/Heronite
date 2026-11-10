//! Undo of the last bulk job.
//!
//! Before a job writes a game's record it keeps how the record was (or that
//! there was none) and what it wrote. Undo puts every record back in one
//! write of `metadata.json`, part by part: a field with its recorded source,
//! or any other key of the record. A part goes back only when it still holds
//! what the job wrote, so whatever changed after the job (a hand edit, a
//! fetch) stays. A record the job created and nothing touched since is
//! removed.
//!
//! Pictures in `metadata-art`: before the job may write a game's pictures,
//! the files already there are hard-linked (copied where links fail) into
//! `metadata-undo/`; a picture is always written as a new file renamed over
//! the old one, so the link keeps the old picture. After the write the job
//! notes which of those files it replaced with other bytes, and how it left
//! each (size, time, file id). Undo then removes the pictures the job
//! created that the record no longer points at, and puts back each picture
//! the job replaced that the record points at and that is still as the job
//! left it: a picture written since (a fetch, one chosen by hand; the file
//! names never change) stays. No other file is removed. Steam tags are not
//! undone: they are a cache of what Steam lists.
//!
//! The review queue: the entries the job added are removed and those it
//! took out (games it saved) put back, each only while it is as the job
//! left it (a game the user reviewed since stays reviewed).
//!
//! The undo lives in memory: it is offered until the next job starts or the
//! app closes. Writing it to disk as a job runs would add a write per game,
//! and an undo that comes back after a restart, over records other things
//! changed meanwhile, surprises more than it helps. The picture copies left
//! by a closed app are removed when the next job starts.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use heronite_data::cover_family::CoverFamily;
use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use heronite_data::paths::DataPaths;
use heronite_data::settings::ALL_FIELDS;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::MetadataService;
use crate::job::UndoReport;
use crate::review::ReviewEntry;
use crate::types::ArtKind;
use crate::util::{blank_metadata, folder_name, local_art_url};

const FIELD_SOURCES: &str = "fieldSources";

/// One game's pictures around a job's write.
pub(super) struct Pictures {
    folder: String,
    /// The folder existed before the job.
    existed: bool,
    /// Files there before the job, kept in the backup folder.
    kept: Vec<String>,
    /// Files the job added.
    created: Vec<String>,
    /// Files of `kept` the job replaced with other bytes, as it left them.
    replaced: Vec<(String, Stamp)>,
}

/// How a file was left. Every write here goes through a new file, so a
/// later write changes it even when the size stays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    /// The inode on Unix; 0 elsewhere.
    id: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok()?;
    #[cfg(unix)]
    let id = std::os::unix::fs::MetadataExt::ino(&m);
    #[cfg(not(unix))]
    let id = 0;
    Some(Stamp { len: m.len(), modified: m.modified().ok(), id })
}

impl Pictures {
    /// Call after the write: notes the files the job added and those it
    /// replaced.
    pub(super) fn after(mut self, paths: &DataPaths) -> Self {
        let dir = paths.metadata_art_dir().join(&self.folder);
        let backup = paths.metadata_undo_dir().join(&self.folder);
        let before: HashSet<&String> = self.kept.iter().collect();
        self.created = files_in(&dir).into_iter().filter(|f| !before.contains(f)).collect();
        self.replaced = self
            .kept
            .iter()
            .filter_map(|name| {
                let (now, was) = (dir.join(name), backup.join(name));
                let left = stamp(&now)?;
                // Still the backup's own file (a hard link): untouched.
                if stamp(&was) == Some(left) {
                    return None;
                }
                (fs::read(&now).ok()? != fs::read(&was).ok()?).then(|| (name.clone(), left))
            })
            .collect();
        self
    }

    /// Whether the job replaced the picture `url` points at with another one.
    pub(super) fn rewrote(&self, url: Option<&str>) -> bool {
        url.is_some_and(|url| self.replaced.iter().any(|(name, _)| local_art_url("metadata-art", &[&self.folder, name]) == url))
    }
}

struct Entry {
    before: Option<CollectionGameMetadata>,
    after: CollectionGameMetadata,
    pictures: Option<Pictures>,
}

/// What a job wrote, game by game (`runner:appName` keys).
pub(super) struct UndoLog {
    job: u64,
    entries: IndexMap<String, Entry>,
    /// Review queue entries: as they were before the job, and as it left
    /// them (`None`: none).
    reviews: IndexMap<String, (Option<ReviewEntry>, Option<ReviewEntry>)>,
}

impl UndoLog {
    pub(super) fn new(job: u64) -> Self {
        Self { job, entries: IndexMap::new(), reviews: IndexMap::new() }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.reviews.is_empty()
    }

    /// The job changed the queue entry of `key` from `before` to `after`.
    pub(super) fn record_review(&mut self, key: String, before: Option<ReviewEntry>, after: Option<ReviewEntry>) {
        self.reviews.entry(key).and_modify(|e| e.1 = after.clone()).or_insert((before, after));
    }

    /// Before the job may write the pictures of a game: keeps the files
    /// there. A game met twice keeps its first copies.
    pub(super) fn pictures_before(&self, paths: &DataPaths, runner: &str, app_name: &str) -> Pictures {
        let folder = folder_name(runner, app_name);
        let dir = paths.metadata_art_dir().join(&folder);
        let kept = files_in(&dir);
        let backup = paths.metadata_undo_dir().join(&folder);
        for name in &kept {
            let to = backup.join(name);
            if to.exists() {
                continue;
            }
            let linked = fs::create_dir_all(&backup)
                .and_then(|_| fs::hard_link(dir.join(name), &to).or_else(|_| fs::copy(dir.join(name), &to).map(|_| ())));
            if let Err(e) = linked {
                log::warn!("Undo cannot keep {}: {e}", dir.join(name).display());
            }
        }
        Pictures { folder, existed: dir.is_dir(), kept, created: Vec::new(), replaced: Vec::new() }
    }

    /// The job wrote `after` over `before`.
    pub(super) fn record(
        &mut self,
        key: String,
        before: Option<CollectionGameMetadata>,
        after: CollectionGameMetadata,
        pictures: Option<Pictures>,
    ) {
        match self.entries.get_mut(&key) {
            Some(entry) => {
                entry.after = after;
                let Some(again) = pictures else { return };
                if let Some(first) = &mut entry.pictures {
                    let new: Vec<String> =
                        again.created.into_iter().filter(|f| !first.kept.contains(f) && !first.created.contains(f)).collect();
                    first.created.extend(new);
                    // As the last write left them.
                    for (name, left) in again.replaced {
                        match first.replaced.iter_mut().find(|(n, _)| *n == name) {
                            Some(r) => r.1 = left,
                            None => first.replaced.push((name, left)),
                        }
                    }
                } else {
                    entry.pictures = Some(again);
                }
            }
            None => {
                self.entries.insert(key, Entry { before, after, pictures });
            }
        }
    }
}

/// The file names in `dir` (none when it does not exist).
fn files_in(dir: &Path) -> Vec<String> {
    let Ok(read) = fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.ends_with(".part"))
        .collect();
    names.sort();
    names
}

/// Removes the picture copies of an earlier job.
pub(super) fn clear_backups(paths: &DataPaths) {
    let dir = paths.metadata_undo_dir();
    if dir.exists() {
        if let Err(e) = fs::remove_dir_all(&dir) {
            log::warn!("Could not remove {}: {e}", dir.display());
        }
    }
}

fn to_map(md: &CollectionGameMetadata) -> Result<Map<String, Value>> {
    match serde_json::to_value(md)? {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("a metadata record is not an object"),
    }
}

/// The JSON key of an `ALL_FIELDS` entry.
fn json_key(field: &str) -> &str {
    match field {
        "hero" => "heroUrl",
        f => match CoverFamily::from_field(f) {
            Some(family) => family.json_key(),
            None => f,
        },
    }
}

/// A part of a record that goes back as one.
enum Part {
    /// A field and its recorded source.
    Field(&'static str),
    /// Any other key of the record.
    Key(String),
    /// A recorded source of something that is not a field.
    Source(String),
}

impl Part {
    fn get(&self, map: &Map<String, Value>) -> (Option<Value>, Option<Value>) {
        let source = |name: &str| map.get(FIELD_SOURCES).and_then(|s| s.get(name)).cloned();
        match self {
            Self::Field(f) => (map.get(json_key(f)).cloned(), source(f)),
            Self::Key(k) => (map.get(k).cloned(), None),
            Self::Source(name) => (None, source(name)),
        }
    }

    fn set(&self, map: &mut Map<String, Value>, (value, source): (Option<Value>, Option<Value>)) {
        let set = |map: &mut Map<String, Value>, key: &str, v: Option<Value>| match v {
            Some(v) => {
                map.insert(key.to_owned(), v);
            }
            None => {
                map.remove(key);
            }
        };
        let set_source = |map: &mut Map<String, Value>, name: &str, v: Option<Value>| {
            if let Value::Object(sources) = map.entry(FIELD_SOURCES).or_insert_with(|| Value::Object(Map::new())) {
                set(sources, name, v);
            }
        };
        match self {
            Self::Field(f) => {
                set(map, json_key(f), value);
                set_source(map, f, source);
            }
            Self::Key(k) => set(map, k, value),
            Self::Source(name) => set_source(map, name, source),
        }
    }
}

/// The parts of the three records.
fn parts(maps: [&Map<String, Value>; 3]) -> Vec<Part> {
    let field_keys: HashSet<&str> = ALL_FIELDS.iter().map(|f| json_key(f)).collect();
    let mut out: Vec<Part> = ALL_FIELDS.into_iter().map(Part::Field).collect();
    let mut keys: Vec<&String> = maps.iter().flat_map(|m| m.keys()).collect();
    keys.sort();
    keys.dedup();
    out.extend(keys.into_iter().filter(|k| k.as_str() != FIELD_SOURCES && !field_keys.contains(k.as_str())).map(|k| Part::Key(k.clone())));
    let mut sources: Vec<&String> =
        maps.iter().filter_map(|m| m.get(FIELD_SOURCES).and_then(Value::as_object)).flat_map(|s| s.keys()).collect();
    sources.sort();
    sources.dedup();
    out.extend(sources.into_iter().filter(|s| !ALL_FIELDS.contains(&s.as_str())).map(|s| Part::Source(s.clone())));
    out
}

/// `now` with every part the job changed and that still holds the job's
/// value put back as in `before`. Returns it and how many changed parts
/// were left because they changed after the job.
fn revert(
    before: &CollectionGameMetadata,
    after: &CollectionGameMetadata,
    now: &CollectionGameMetadata,
) -> Result<(CollectionGameMetadata, usize)> {
    let (b, a, mut n) = (to_map(before)?, to_map(after)?, to_map(now)?);
    let mut kept = 0;
    for part in parts([&b, &a, &n]) {
        let (was, wrote) = (part.get(&b), part.get(&a));
        if was == wrote {
            continue;
        }
        if part.get(&n) == wrote {
            part.set(&mut n, was);
        } else {
            kept += 1;
        }
    }
    Ok((serde_json::from_value(Value::Object(n))?, kept))
}

impl MetadataService {
    /// Whether the last finished or stopped job can be undone.
    pub fn can_undo_bulk(&self) -> bool {
        let jobs = self.jobs.lock();
        jobs.undo.is_some() && !jobs.progress.is_running()
    }

    /// Undoes the last finished or stopped job (see the module docs): every
    /// record it changed, as one write. `None` when there is nothing to undo
    /// (no job yet, undone already, or a job runs or waits). Blocking.
    pub fn undo_bulk_metadata(&self) -> Result<Option<UndoReport>> {
        let _gate = self.undo_gate.lock();
        let log = {
            let mut jobs = self.jobs.lock();
            if jobs.running {
                return Ok(None);
            }
            let Some(log) = jobs.undo.take() else { return Ok(None) };
            jobs.progress.can_undo = false;
            log
        };
        match undo(&self.paths, &log) {
            Ok(mut report) => {
                clear_backups(&self.paths);
                let reviews: Vec<_> = log.reviews.iter().map(|(k, (b, a))| (k.clone(), b.clone(), a.clone())).collect();
                match self.restore_queue(&reviews) {
                    Ok(n) => report.reviews = n,
                    Err(e) => log::warn!("Undo could not restore the review queue: {e:#}"),
                }
                Ok(Some(report))
            }
            // Nothing was written (one write of the file): it can be tried again.
            Err(e) => {
                let mut jobs = self.jobs.lock();
                jobs.progress.can_undo = !jobs.running;
                jobs.undo = Some(log);
                Err(e)
            }
        }
    }
}

fn undo(paths: &DataPaths, log: &UndoLog) -> Result<UndoReport> {
    let mut report = UndoReport { job: log.job, ..Default::default() };
    // Each game's record once undone (`None`: there is none).
    let records: Vec<(&String, Option<CollectionGameMetadata>)> = json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
        let mut out = Vec::new();
        for (key, entry) in &log.entries {
            // Removed since (the game was): nothing to put back.
            let Some(now) = file.games.get(key) else { continue };
            let untouched = to_map(now)? == to_map(&entry.after)?;
            let restored = match (&entry.before, untouched) {
                (None, true) => None,
                (Some(before), true) => Some(before.clone()),
                (before, false) => {
                    let blank = blank_metadata(&entry.after.app_name, &entry.after.runner);
                    let (restored, kept) = revert(before.as_ref().unwrap_or(&blank), &entry.after, now)?;
                    report.kept += kept;
                    Some(restored)
                }
            };
            match &restored {
                Some(md) => file.games.insert(key.clone(), md.clone()),
                None => file.games.shift_remove(key),
            };
            report.games += 1;
            out.push((key, restored));
        }
        Ok(out)
    })?;
    let by_key: IndexMap<&String, Option<CollectionGameMetadata>> = records.into_iter().collect();
    for (key, entry) in &log.entries {
        let (Some(pictures), Some(record)) = (&entry.pictures, by_key.get(key)) else { continue };
        restore_pictures(paths, pictures, record.as_ref(), &mut report);
    }
    Ok(report)
}

fn restore_pictures(paths: &DataPaths, p: &Pictures, record: Option<&CollectionGameMetadata>, report: &mut UndoReport) {
    let dir = paths.metadata_art_dir().join(&p.folder);
    let backup: PathBuf = paths.metadata_undo_dir().join(&p.folder);
    let used = |name: &str| {
        let url = local_art_url("metadata-art", &[&p.folder, name]);
        record.is_some_and(|r| ArtKind::ALL.iter().any(|k| k.of_record(r) == Some(url.as_str())))
    };
    for name in p.created.iter().filter(|n| !used(n)) {
        match fs::remove_file(dir.join(name)) {
            Ok(()) => report.files_removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("Undo could not remove {}: {e}", dir.join(name).display()),
        }
    }
    for (name, left) in p.replaced.iter().filter(|(n, _)| p.kept.contains(n) && used(n)) {
        let (from, to) = (backup.join(name), dir.join(name));
        // Written again after the job: that picture stays.
        if stamp(&to) != Some(*left) {
            continue;
        }
        let Ok(old) = fs::read(&from) else { continue };
        let part = dir.join(format!("{name}.part"));
        match fs::create_dir_all(&dir)
            .and_then(|_| fs::write(&part, &old))
            .and_then(|_| heronite_platform::fs::rename_replacing(&part, &to))
        {
            Ok(()) => report.files_restored += 1,
            Err(e) => log::warn!("Undo could not put back {}: {e}", to.display()),
        }
    }
    if !p.existed && files_in(&dir).is_empty() {
        let _ = fs::remove_dir(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(f: impl FnOnce(&mut CollectionGameMetadata)) -> CollectionGameMetadata {
        let mut m = blank_metadata("a", "sideload");
        f(&mut m);
        m
    }

    #[test]
    fn a_part_goes_back_only_while_it_holds_the_jobs_value() {
        let before = md(|m| {
            m.genres = vec!["Old".into()];
            m.field_sources.insert("genres".into(), "steam".into());
            m.fetched_at = Some("then".into());
        });
        let after = md(|m| {
            m.description = Some("job".into());
            m.field_sources.insert("description".into(), "igdb".into());
            m.genres = vec!["New".into()];
            m.field_sources.insert("genres".into(), "igdb".into());
            m.fetched_at = Some("now".into());
            m.igdb_id = Some(heronite_data::num::JsNum(7.0));
        });
        // After the job the user typed genres by hand and wrote notes.
        let mut now = after.clone();
        now.genres = vec!["Mine".into()];
        now.field_sources.insert("genres".into(), "manual".into());
        now.notes = Some("n".into());

        let (out, kept) = revert(&before, &after, &now).unwrap();
        assert!(out.description.is_none() && !out.field_sources.contains_key("description"));
        assert_eq!((out.genres.as_slice(), out.field_sources["genres"].as_str()), (["Mine".to_string()].as_slice(), "manual"));
        assert_eq!(out.notes.as_deref(), Some("n"));
        assert_eq!(out.fetched_at.as_deref(), Some("then"));
        assert!(out.igdb_id.is_none());
        assert_eq!(kept, 1);
    }

    #[test]
    fn same_value_typed_by_hand_is_a_later_edit() {
        let before = md(|_| {});
        let after = md(|m| {
            m.series = Some("S".into());
            m.field_sources.insert("series".into(), "igdb".into());
        });
        let mut now = after.clone();
        now.field_sources.insert("series".into(), "manual".into());
        let (out, kept) = revert(&before, &after, &now).unwrap();
        assert_eq!((out.series.as_deref(), out.field_sources["series"].as_str()), (Some("S"), "manual"));
        assert_eq!(kept, 1);
    }

    #[test]
    fn unknown_keys_are_parts_too() {
        let before = md(|m| {
            m.extra.insert("futureField".into(), Value::from(1));
        });
        let after = md(|m| {
            m.extra.insert("futureField".into(), Value::from(2));
            m.extra.insert("added".into(), Value::from(true));
        });
        let (out, kept) = revert(&before, &after, &after).unwrap();
        assert_eq!(out.extra.get("futureField"), Some(&Value::from(1)));
        assert!(!out.extra.contains_key("added"));
        assert_eq!(kept, 0);
    }
}
