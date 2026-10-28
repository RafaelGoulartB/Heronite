//! Making, listing and removing backups in a backup folder.
//!
//! A backup is a folder `heronite-YYYY-MM-DD_HHMMSS` holding `data/` (from
//! the data folder), `config/` (from the app state) and `manifest.json`.
//! Files unchanged since the previous backup are hard links to it, so every
//! backup is a complete copy to browse or restore while only what changed
//! takes new space. Where links cannot be made (another file system, FAT),
//! files are copied. A folder without a manifest is not a backup: it is
//! never listed, linked to, restored or removed.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use heronite_data::{AppPaths, DataPaths};
use sha2::{Digest, Sha256};

use crate::catalog::{self, Class, Part, Side};
use crate::manifest::{self, FileEntry, Manifest, Reason, Summary, FORMAT, MANIFEST_FILE};

pub const PREFIX: &str = "heronite-";
const PARTIAL: &str = ".partial";

/// A backup as the list shows it (the manifest without its file list).
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub path: PathBuf,
    /// RFC 3339.
    pub created_at: String,
    pub reason: Reason,
    pub app_version: String,
    pub parts: Vec<Part>,
    pub summary: Summary,
}

impl Info {
    pub fn created(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.created_at).ok().map(|d| d.with_timezone(&Utc))
    }
}

/// What [`create`] made.
#[derive(Clone, Debug)]
pub struct Created {
    pub path: PathBuf,
    pub summary: Summary,
}

/// The backup can never land inside a folder it copies (it would copy itself).
pub fn check_destination(folder: &Path, data: &DataPaths, app: &AppPaths) -> Result<()> {
    let folder = normalize(folder);
    for src in [&data.root, &app.config] {
        let src = normalize(src);
        if folder == src || folder.starts_with(&src) {
            bail!(crate::Error::InsideData);
        }
    }
    Ok(())
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Copies the files of `parts` into a new backup in `folder` (which must
/// exist). The copy is made under a `.partial` name and renamed once
/// complete, so a full disk or a crash never leaves a backup that looks whole.
pub fn create(data: &DataPaths, app: &AppPaths, folder: &Path, parts: &[Part], reason: Reason, now: DateTime<Local>) -> Result<Created> {
    if !data.root.is_dir() {
        bail!(crate::Error::NoData);
    }
    check_destination(folder, data, app)?;
    sweep_partials(folder);
    let name = free_name(folder, now);
    let partial = folder.join(format!("{name}{PARTIAL}"));
    let result = fill(data, app, &partial, parts, reason, now, previous(folder));
    let summary = match result {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::remove_dir_all(&partial);
            return Err(e);
        }
    };
    let path = folder.join(&name);
    fs::rename(&partial, &path).with_context(|| format!("renaming {}", partial.display()))?;
    Ok(Created { path, summary })
}

/// Files of the newest backup, by side and path, to link to.
fn previous(folder: &Path) -> Option<(PathBuf, HashMap<(Side, String), FileEntry>)> {
    let newest = list(folder).into_iter().next()?;
    let m = Manifest::read(&newest.path).ok()?;
    Some((newest.path, m.files.into_iter().map(|f| ((f.side, f.path.clone()), f)).collect()))
}

fn fill(
    data: &DataPaths,
    app: &AppPaths,
    partial: &Path,
    parts: &[Part],
    reason: Reason,
    now: DateTime<Local>,
    prev: Option<(PathBuf, HashMap<(Side, String), FileEntry>)>,
) -> Result<Summary> {
    fs::create_dir(partial).with_context(|| format!("creating {}", partial.display()))?;
    // Sign-ins and the play history are nobody else's business.
    set_private(partial);
    let mut files = Vec::new();
    let (mut bytes, mut new_bytes) = (0u64, 0u64);
    for (side, root) in [(Side::Data, &data.root), (Side::Config, &app.config)] {
        for found in catalog::walk(root, side, false) {
            let Class::In(part) = found.class else { continue };
            if !parts.contains(&part) {
                continue;
            }
            let dest = partial.join(side.dir_name()).join(&found.rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let linked = prev.as_ref().and_then(|(dir, map)| {
                let old = map.get(&(side, found.rel.clone()))?;
                (old.size == found.size && old.mtime_ms == found.mtime_ms).then_some(())?;
                let src = dir.join(side.dir_name()).join(&found.rel);
                (fs::metadata(&src).ok()?.len() == old.size).then_some(())?;
                fs::hard_link(&src, &dest).ok()?;
                Some(old.sha256.clone())
            });
            let (sha256, size) = match linked {
                Some(sha) => (sha, found.size),
                None => match copy_hashed(&found.path, &dest) {
                    Ok((sha, size)) => {
                        new_bytes += size;
                        (sha, size)
                    }
                    // Deleted since the listing (a temp file the app replaced).
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(anyhow!(e).context(format!("copying {}", found.path.display()))),
                },
            };
            bytes += size;
            files.push(FileEntry { side, path: found.rel, size, mtime_ms: found.mtime_ms, sha256, part });
        }
    }
    let mut summary = manifest::summarize(&partial.join(Side::Data.dir_name()));
    summary.files = files.len();
    summary.bytes = bytes;
    summary.new_bytes = new_bytes;
    let manifest = Manifest {
        format: FORMAT,
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at: now.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true),
        reason,
        host: manifest::hostname(),
        home: dirs_home(),
        data_root: data.root.to_string_lossy().into_owned(),
        config_root: app.config.to_string_lossy().into_owned(),
        parts: parts.to_vec(),
        summary: summary.clone(),
        files,
    };
    manifest.write(partial)?;
    Ok(summary)
}

pub(crate) fn dirs_home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// Copies `src` to `dst`, returning the hex SHA-256 and the bytes copied.
pub(crate) fn copy_hashed(src: &Path, dst: &Path) -> io::Result<(String, u64)> {
    let mut input = fs::File::open(src)?;
    let mut output = fs::File::create(dst)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        output.write_all(&buf[..n])?;
        total += n as u64;
    }
    output.sync_all()?;
    Ok((hex(&hasher.finalize()), total))
}

pub(crate) fn hash_file(path: &Path) -> io::Result<String> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(unix)]
fn set_private(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
}

/// `heronite-YYYY-MM-DD_HHMMSS`, with `-2`, `-3`... when taken.
fn free_name(folder: &Path, now: DateTime<Local>) -> String {
    let base = format!("{PREFIX}{}", now.format("%Y-%m-%d_%H%M%S"));
    let mut name = base.clone();
    let mut n = 2;
    while folder.join(&name).exists() || folder.join(format!("{name}{PARTIAL}")).exists() {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Removes copies left half-made by a crash. Only called while this process
/// holds the backup lock, so none of them is being written.
fn sweep_partials(folder: &Path) {
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(PREFIX) && name.ends_with(PARTIAL) && entry.path().is_dir() {
            log::info!("removing an unfinished backup: {}", entry.path().display());
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The backups in `folder`, newest first.
pub fn list(folder: &Path) -> Vec<Info> {
    let mut out: Vec<Info> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.starts_with(PREFIX) && !name.ends_with(PARTIAL)).then(|| read_info(&e.path()))?
        })
        .collect();
    out.sort_by_key(|i| std::cmp::Reverse(i.created()));
    out
}

/// One backup folder, or `None` when it is not one.
pub fn read_info(path: &Path) -> Option<Info> {
    if !path.join(MANIFEST_FILE).is_file() {
        return None;
    }
    let m = Manifest::read(path).map_err(|e| log::warn!("{e:#}")).ok()?;
    Some(Info {
        path: path.to_path_buf(),
        created_at: m.created_at,
        reason: m.reason,
        app_version: m.app_version,
        parts: m.parts,
        summary: m.summary,
    })
}

/// A backup made before a restore stays this long (the newest one stays
/// whatever its age), so the user can still undo the restore.
const BEFORE_RESTORE_KEPT: chrono::Duration = chrono::Duration::days(30);

/// Keeps the newest `keep` backups of `folder` (0 = all), plus the ones
/// [`to_remove`] protects, and removes the others; returns what was removed.
pub fn prune(folder: &Path, keep: u32) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for info in to_remove(list(folder), keep, Utc::now()) {
        match fs::remove_dir_all(&info.path) {
            Ok(()) => removed.push(info.path),
            Err(e) => log::warn!("could not remove the old backup {}: {e}", info.path.display()),
        }
    }
    removed
}

/// The backups [`prune`] removes. `infos` is newest first. Kept: the newest
/// `keep` (0 = all), and, beyond that limit,
/// - every backup made before a restore that is younger than
///   [`BEFORE_RESTORE_KEPT`], and the newest one of them at any age (the
///   restore's undo), so a run of scheduled backups never pushes it out;
/// - the newest backup of each set of parts that no newer backup covers, so a
///   partial backup (Collection only) never pushes out the last full one,
///   while an old partial one goes once a newer backup has all its parts.
fn to_remove(infos: Vec<Info>, keep: u32, now: DateTime<Utc>) -> Vec<Info> {
    if keep == 0 {
        return Vec::new();
    }
    let mut seen_sets: Vec<Vec<Part>> = Vec::new();
    let mut seen_before_restore = false;
    let mut out = Vec::new();
    for (i, info) in infos.into_iter().enumerate() {
        let mut parts = info.parts.clone();
        parts.sort();
        parts.dedup();
        let newest_of_set = !seen_sets.iter().any(|newer| parts.iter().all(|p| newer.contains(p)));
        seen_sets.push(parts);
        let before_restore = info.reason == Reason::BeforeRestore;
        let undo = before_restore && (!seen_before_restore || info.created().is_some_and(|t| now - t < BEFORE_RESTORE_KEPT));
        seen_before_restore |= before_restore;
        if i >= keep as usize && !newest_of_set && !undo {
            out.push(info);
        }
    }
    out
}

/// Removes one backup. Refuses a folder that is not a backup.
pub fn delete(path: &Path) -> Result<()> {
    if read_info(path).is_none() {
        bail!("{} is not a backup", path.display());
    }
    fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
}

/// Disk space the backups of `folder` take: each file counted once, however
/// many backups link to it.
pub fn folder_usage(folder: &Path) -> u64 {
    use std::collections::HashSet;
    use std::os::unix::fs::MetadataExt;
    let mut seen = HashSet::new();
    let mut total = 0;
    let mut stack: Vec<PathBuf> = list(folder).into_iter().map(|i| i.path).collect();
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() && seen.insert((meta.dev(), meta.ino())) {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::TimeZone;

    pub(crate) fn fixture(root: &Path) -> (DataPaths, AppPaths) {
        let data = DataPaths::at(root.join("data"));
        let app = AppPaths { config: root.join("config"), cache: root.join("cache") };
        for (p, c) in [
            ("data/config.json", "{}"),
            ("data/local_library/library.json", r#"{"games":{"a":{},"b":{}}}"#),
            ("data/local_library/sessions.json", r#"{"sessions":[1,2,3]}"#),
            ("data/local_library/journal.json", r#"{"entries":{"a":[1,2]}}"#),
            ("data/local_library/art/x/hero-1.png", "art"),
            ("data/local_library/heroes/1.jpg", "hero"),
            ("data/legendaryConfig/legendary/user.json", "token"),
            ("data/tools/proton/bin", "x"),
            ("data/store_cache/gog_library.json", "[]"),
            ("data/store_cache/humble_deals.json", "[]"),
            ("config/ui-state.json", "{}"),
            ("config/logs/a.log", "x"),
        ] {
            let f = root.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, c).unwrap();
        }
        (data, app)
    }

    fn at(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 19, h, m, 0).unwrap()
    }

    #[test]
    fn creates_links_lists_and_prunes() {
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let folder = d.path().join("backups");
        fs::create_dir(&folder).unwrap();

        let first = create(&data, &app, &folder, &Part::ALL, Reason::Manual, at(10, 0)).unwrap();
        assert!(first.path.ends_with("heronite-2026-10-19_100000"));
        let b = &first.path;
        for kept in [
            "data/config.json",
            "data/local_library/art/x/hero-1.png",
            "data/legendaryConfig/legendary/user.json",
            "data/store_cache/gog_library.json",
            "config/ui-state.json",
        ] {
            assert!(b.join(kept).is_file(), "{kept}");
        }
        for left in ["data/tools", "data/local_library/heroes", "data/store_cache/humble_deals.json", "config/logs"] {
            assert!(!b.join(left).exists(), "{left}");
        }
        assert_eq!((first.summary.games, first.summary.sessions, first.summary.journal), (2, 3, 2));
        assert_eq!(first.summary.new_bytes, first.summary.bytes);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(b).unwrap().permissions().mode() & 0o777, 0o700);

        // Unchanged files are links to the first backup; changed ones are copies.
        fs::write(data.root.join("config.json"), r#"{"x":1}"#).unwrap();
        let second = create(&data, &app, &folder, &[Part::Collection], Reason::Scheduled, at(11, 0)).unwrap();
        assert_eq!(second.summary.new_bytes, 7);
        use std::os::unix::fs::MetadataExt;
        let ino = |p: &Path| fs::metadata(p).unwrap().ino();
        assert_eq!(ino(&b.join("data/local_library/library.json")), ino(&second.path.join("data/local_library/library.json")));
        assert_ne!(ino(&b.join("data/config.json")), ino(&second.path.join("data/config.json")));
        assert!(!second.path.join("config").exists());
        assert!(!second.path.join("data/local_library/art").exists());

        let listed = list(&folder);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].path, second.path);
        assert_eq!(listed[0].parts, [Part::Collection]);
        assert_eq!(listed[1].reason, Reason::Manual);

        let third = create(&data, &app, &folder, &[Part::Collection], Reason::Scheduled, at(12, 0)).unwrap();
        // The first backup is the last full one: the limit does not remove it.
        assert!(prune(&folder, 2).is_empty());
        assert_eq!(list(&folder).len(), 3);

        let full = create(&data, &app, &folder, &Part::ALL, Reason::Scheduled, at(13, 0)).unwrap();
        let removed = prune(&folder, 2);
        assert_eq!(removed, [second.path.clone(), first.path.clone()]);
        assert_eq!(list(&folder).len(), 2);
        // The remaining backups still hold the files the removed ones shared.
        for kept in [&third.path, &full.path] {
            assert_eq!(fs::read_to_string(kept.join("data/local_library/library.json")).unwrap(), r#"{"games":{"a":{},"b":{}}}"#);
        }
        assert!(prune(&folder, 0).is_empty());
    }

    #[test]
    fn folders_without_a_manifest_are_not_backups() {
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let folder = d.path().join("backups");
        // A copy of a data folder made by an older version.
        let old = folder.join("heronite-2026-10-13");
        fs::create_dir_all(old.join("local_library")).unwrap();
        fs::write(old.join("local_library/library.json"), r#"{"games":{"a":{}}}"#).unwrap();
        fs::create_dir_all(folder.join("unrelated")).unwrap();
        assert!(read_info(&old).is_none());
        let made = create(&data, &app, &folder, &[Part::Collection], Reason::Manual, at(10, 0)).unwrap();
        assert_eq!(list(&folder).len(), 1);
        assert_eq!(prune(&folder, 1), Vec::<PathBuf>::new());
        assert!(old.exists());
        assert!(delete(&old).is_err());
        assert!(delete(&folder.join("unrelated")).is_err());
        delete(&made.path).unwrap();
        assert!(list(&folder).is_empty());
    }

    #[test]
    fn refuses_a_folder_inside_the_data_and_cleans_partials() {
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let inside = data.root.join("b");
        fs::create_dir(&inside).unwrap();
        let err = create(&data, &app, &inside, &Part::ALL, Reason::Manual, at(10, 0)).unwrap_err();
        assert_eq!(err.downcast_ref::<crate::Error>(), Some(&crate::Error::InsideData));
        assert!(check_destination(&app.config.join("x"), &data, &app).is_err());
        assert!(check_destination(&d.path().join("datax"), &data, &app).is_ok());

        let folder = d.path().join("backups");
        fs::create_dir_all(folder.join("heronite-2026-01-01_000000.partial/data")).unwrap();
        let made = create(&data, &app, &folder, &Part::ALL, Reason::Manual, at(10, 0)).unwrap();
        assert!(!folder.join("heronite-2026-01-01_000000.partial").exists());
        // Same second: a second name.
        let again = create(&data, &app, &folder, &Part::ALL, Reason::Manual, at(10, 0)).unwrap();
        assert_ne!(made.path, again.path);
        assert!(again.path.to_string_lossy().ends_with("-2"));
        // Linked files count once.
        let naive: u64 = [&made.path, &again.path].iter().flat_map(|p| catalog::walk(p, Side::Data, true)).map(|f| f.size).sum();
        assert_eq!(folder_usage(&folder), naive - made.summary.bytes);
    }

    fn info(name: &str, days_old: i64, reason: Reason, parts: &[Part], now: DateTime<Utc>) -> Info {
        Info {
            path: PathBuf::from(name),
            created_at: (now - chrono::Duration::days(days_old)).to_rfc3339(),
            reason,
            app_version: String::new(),
            parts: parts.to_vec(),
            summary: Summary::default(),
        }
    }

    fn removed(infos: Vec<Info>, keep: u32, now: DateTime<Utc>) -> Vec<String> {
        to_remove(infos, keep, now).iter().map(|i| i.path.display().to_string()).collect()
    }

    #[test]
    fn the_limit_alone_removes_the_oldest() {
        let now = Utc::now();
        let all = Part::ALL;
        let infos = (0..5).map(|d| info(&format!("b{d}"), d, Reason::Scheduled, &all, now)).collect::<Vec<_>>();
        assert_eq!(removed(infos.clone(), 3, now), ["b3", "b4"]);
        assert!(removed(infos.clone(), 5, now).is_empty());
        assert!(removed(infos, 0, now).is_empty());
    }

    #[test]
    fn the_backup_before_a_restore_outlives_the_scheduled_ones() {
        let now = Utc::now();
        let all = Part::ALL;
        // The safety backup is 3 days old; three scheduled ones came after.
        let infos = vec![
            info("s1", 0, Reason::Scheduled, &all, now),
            info("s2", 1, Reason::Scheduled, &all, now),
            info("s3", 2, Reason::Scheduled, &all, now),
            info("undo", 3, Reason::BeforeRestore, &all, now),
            info("s0", 4, Reason::Scheduled, &all, now),
        ];
        assert_eq!(removed(infos, 2, now), ["s3", "s0"]);
    }

    #[test]
    fn only_the_newest_old_safety_backup_stays_past_thirty_days() {
        let now = Utc::now();
        let all = Part::ALL;
        let infos = vec![
            info("new", 0, Reason::Scheduled, &all, now),
            info("young", 29, Reason::BeforeRestore, &all, now),
            info("old-a", 40, Reason::BeforeRestore, &all, now),
            info("old-b", 90, Reason::BeforeRestore, &all, now),
        ];
        assert_eq!(removed(infos.clone(), 1, now), ["old-a", "old-b"]);
        // With no young one, the newest old one stays.
        let infos = infos.into_iter().filter(|i| i.path != Path::new("young")).collect();
        assert_eq!(removed(infos, 1, now), ["old-b"]);
    }

    #[test]
    fn a_partial_backup_never_pushes_out_the_last_full_one() {
        let now = Utc::now();
        let infos = vec![
            info("c1", 0, Reason::Scheduled, &[Part::Collection], now),
            info("c2", 1, Reason::Scheduled, &[Part::Collection], now),
            info("c3", 2, Reason::Scheduled, &[Part::Collection], now),
            info("full-new", 5, Reason::Manual, &Part::ALL, now),
            info("full-old", 6, Reason::Manual, &Part::ALL, now),
            // The same set in another order, and a set of its own.
            info("art", 7, Reason::Manual, &[Part::Art, Part::Collection], now),
            info("art-old", 8, Reason::Manual, &[Part::Collection, Part::Art], now),
        ];
        // `art` goes too: the newer full backup has its parts.
        assert_eq!(removed(infos, 2, now), ["c3", "full-old", "art", "art-old"]);
    }

    #[test]
    fn an_old_partial_backup_goes_once_a_newer_one_has_all_its_parts() {
        let now = Utc::now();
        let infos = vec![
            info("full-1", 0, Reason::Scheduled, &Part::ALL, now),
            info("full-2", 1, Reason::Scheduled, &Part::ALL, now),
            info("art", 400, Reason::Manual, &[Part::Art, Part::Collection], now),
            info("collection", 500, Reason::Manual, &[Part::Collection], now),
        ];
        assert_eq!(removed(infos, 2, now), ["art", "collection"]);
        // A newer partial backup of other parts covers nothing.
        let infos = vec![
            info("logins", 0, Reason::Manual, &[Part::Logins], now),
            info("settings", 1, Reason::Manual, &[Part::Settings], now),
            info("collection", 500, Reason::Manual, &[Part::Collection], now),
        ];
        assert!(removed(infos, 1, now).is_empty());
    }

    #[test]
    fn a_date_that_does_not_parse_counts_as_old() {
        let now = Utc::now();
        let mut odd = info("odd", 0, Reason::BeforeRestore, &Part::ALL, now);
        odd.created_at = "yesterday".into();
        let infos = vec![info("new", 0, Reason::Scheduled, &Part::ALL, now), odd.clone(), Info { path: "odd2".into(), ..odd }];
        assert_eq!(removed(infos, 1, now), ["odd2"]);
    }
}
