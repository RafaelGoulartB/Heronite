//! Putting a backup back.
//!
//! The app holds most of its data in memory and writes it back as it runs,
//! so a restore never copies under a running app. [`prepare`] checks every
//! file of the backup, backs up what is there now and leaves a note
//! ([`PENDING_FILE`] in the app state folder); the app then restarts and
//! [`apply_pending`] copies the files at the very start, before anything
//! reads them.
//!
//! A restored part replaces that part as a whole: its files that are not in
//! the backup are removed (a game added after the backup goes too). What a
//! backup leaves out (Wine builds, caches, logs) is never touched. Paths into
//! the data folder, the app state folder and home are rewritten when the
//! backup came from other folders (another user, another machine, another
//! system: a backup made on Linux restores on Windows).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use heronite_data::migrate::replace_path_prefix;
use heronite_data::{temps, AppPaths, DataPaths};
use serde::{Deserialize, Serialize};

use crate::catalog::{self, Class, Part, Side};
use crate::manifest::{Manifest, Reason, FORMAT};
use crate::run::Lock;
use crate::snapshot;
use crate::Error;

/// The note [`prepare`] leaves for the next start (in `AppPaths::config`).
pub const PENDING_FILE: &str = "restore-pending.json";
/// A note older than this is dropped: the restart it was made for did not
/// happen, and a restore days later would surprise.
const PENDING_TTL: chrono::Duration = chrono::Duration::minutes(15);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pending {
    snapshot: PathBuf,
    parts: Vec<Part>,
    requested_at: String,
}

/// What a restore did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Restored {
    pub snapshot: PathBuf,
    /// RFC 3339 time of the backup.
    pub created_at: String,
    pub parts: Vec<Part>,
    pub written: usize,
    pub removed: usize,
    /// Files whose paths pointed at the backup's folders and were rewritten.
    pub rewritten: usize,
}

/// One file of a backup to put back.
struct Source {
    side: Side,
    rel: String,
    path: PathBuf,
    sha256: String,
    size: u64,
}

/// The files of `parts` in a backup, checked against its manifest.
fn sources(snapshot: &Path, parts: &[Part]) -> Result<(Vec<Source>, Manifest)> {
    let manifest = Manifest::read(snapshot).with_context(|| format!("{} is not a backup", snapshot.display()))?;
    if manifest.format > FORMAT {
        bail!(Error::TooNew);
    }
    let files = manifest
        .files
        .iter()
        .filter(|f| parts.contains(&f.part))
        .map(|f| Source {
            side: f.side,
            rel: f.path.clone(),
            path: snapshot.join(f.side.dir_name()).join(&f.path),
            sha256: f.sha256.clone(),
            size: f.size,
        })
        .collect();
    Ok((files, manifest))
}

/// Every file is there with the content the manifest names.
fn verify(files: &[Source]) -> Result<()> {
    for f in files {
        let ok = fs::metadata(&f.path).is_ok_and(|m| m.len() == f.size) && snapshot::hash_file(&f.path).is_ok_and(|got| got == f.sha256);
        if !ok {
            bail!(Error::Damaged(f.rel.clone()));
        }
    }
    Ok(())
}

/// The parts of `parts` the backup holds.
pub fn available(info: &snapshot::Info, parts: &[Part]) -> Vec<Part> {
    parts.iter().copied().filter(|p| info.parts.contains(p)).collect()
}

/// Checks the backup, backs up the current data (every part) into
/// `safety_folder` and leaves the note for the next start. Returns the
/// safety backup. The caller restarts the app.
pub fn prepare(data: &DataPaths, app: &AppPaths, snapshot: &Path, parts: &[Part], safety_folder: &Path) -> Result<PathBuf> {
    let Some(_lock) = Lock::acquire() else { bail!("A backup is being made. Try again in a moment.") };
    let (files, _) = sources(snapshot, parts)?;
    verify(&files)?;
    let safety = snapshot::create(data, app, safety_folder, &Part::ALL, Reason::BeforeRestore, Local::now())?;
    let pending = Pending {
        snapshot: snapshot.to_path_buf(),
        parts: parts.to_vec(),
        requested_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    fs::create_dir_all(&app.config)?;
    temps::write_atomic(&app.config.join(PENDING_FILE), serde_json::to_string_pretty(&pending)?.as_bytes(), None)?;
    log::info!("restore of {} prepared; current data backed up to {}", snapshot.display(), safety.path.display());
    Ok(safety.path)
}

/// Drops the note of [`prepare`] (the restart was refused).
pub fn cancel(app: &AppPaths) {
    let _ = fs::remove_file(app.config.join(PENDING_FILE));
}

/// Runs the restore [`prepare`] asked for, if any. Called first thing at
/// start. The note is removed before the copy, so a restore that crashes the
/// app is not tried again at every start (the safety backup is there).
pub fn apply_pending(data: &DataPaths, app: &AppPaths) -> Option<Result<Restored>> {
    let note = app.config.join(PENDING_FILE);
    let text = fs::read_to_string(&note).ok()?;
    let _ = fs::remove_file(&note);
    let pending: Pending = match serde_json::from_str(&text) {
        Ok(p) => p,
        Err(e) => return Some(Err(anyhow::anyhow!("unreadable {PENDING_FILE}: {e}"))),
    };
    let fresh = DateTime::parse_from_rfc3339(&pending.requested_at).is_ok_and(|t| Utc::now() - t.with_timezone(&Utc) < PENDING_TTL);
    if !fresh {
        log::warn!("ignoring a restore of {} asked at {}", pending.snapshot.display(), pending.requested_at);
        return None;
    }
    Some(restore(data, app, &pending.snapshot, &pending.parts))
}

/// Copies the files of `parts` from `snapshot` into the data and app state
/// folders. Only call it while nothing else uses them (see [`apply_pending`]).
pub fn restore(data: &DataPaths, app: &AppPaths, snapshot: &Path, parts: &[Part]) -> Result<Restored> {
    let info = snapshot::read_info(snapshot).with_context(|| format!("{} is not a backup", snapshot.display()))?;
    let parts = available(&info, parts);
    let (files, manifest) = sources(snapshot, &parts)?;
    verify(&files)?;
    let rewrites = rewrites(&manifest, data, app);
    let root = |side: Side| match side {
        Side::Data => &data.root,
        Side::Config => &app.config,
    };
    let mut done =
        Restored { snapshot: snapshot.to_path_buf(), created_at: info.created_at.clone(), parts: parts.clone(), ..Default::default() };
    let mut kept: HashSet<(Side, String)> = HashSet::new();
    for f in &files {
        let dest = root(f.side).join(&f.rel);
        let bytes = fs::read(&f.path).with_context(|| format!("reading {}", f.path.display()))?;
        let bytes = match rewrite_paths(&f.rel, &bytes, &rewrites) {
            Some(changed) => {
                done.rewritten += 1;
                changed
            }
            None => bytes,
        };
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        temps::write_atomic(&dest, &bytes, None).with_context(|| format!("writing {}", dest.display()))?;
        done.written += 1;
        kept.insert((f.side, f.rel.clone()));
    }
    // The rest of each restored part goes.
    for side in [Side::Data, Side::Config] {
        for found in catalog::walk(root(side), side, false) {
            let Class::In(part) = found.class else { continue };
            if parts.contains(&part) && !kept.contains(&(side, found.rel.clone())) && fs::remove_file(&found.path).is_ok() {
                done.removed += 1;
            }
        }
    }
    log::info!(
        "restored {:?} from {}: {} files written, {} removed, {} with rewritten paths",
        parts,
        snapshot.display(),
        done.written,
        done.removed,
        done.rewritten
    );
    Ok(done)
}

/// Old → new folder pairs, most specific first; empty when the backup came
/// from these very folders.
///
/// The new folders go into the files with `/` separators, which Windows
/// takes too: a backslash would have to be escaped in JSON, and the rest of a
/// path from a Linux backup has `/` anyway. A backup from Windows holds its
/// folders escaped (`C:\\Users\\...`) in its JSON files: that form is
/// replaced too.
fn rewrites(m: &Manifest, data: &DataPaths, app: &AppPaths) -> Vec<(String, String)> {
    let home = snapshot::dirs_home();
    let pairs = [
        (m.data_root.clone(), data.root.to_string_lossy().into_owned()),
        (m.config_root.clone(), app.config.to_string_lossy().into_owned()),
        (m.home.clone(), home),
    ];
    let mut out = Vec::new();
    for (old, new) in pairs.into_iter().filter(|(old, new)| !old.is_empty() && !new.is_empty() && old != new) {
        let new = new.replace('\\', "/");
        if old.contains('\\') {
            out.push((old.replace('\\', "\\\\"), new.clone()));
        }
        out.push((old, new));
    }
    out
}

/// The file with the backup's folders replaced, when it is text and any
/// matched.
fn rewrite_paths(rel: &str, bytes: &[u8], rewrites: &[(String, String)]) -> Option<Vec<u8>> {
    if rewrites.is_empty() || ![".json", ".ini", ".cfg", ".conf", ".txt"].iter().any(|e| rel.ends_with(e)) {
        return None;
    }
    let mut text = std::str::from_utf8(bytes).ok()?.to_owned();
    let mut changed = false;
    for (old, new) in rewrites {
        if let Some(next) = replace_path_prefix(&text, old, new) {
            text = next;
            changed = true;
        }
    }
    changed.then(|| text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::tests::fixture;

    #[test]
    fn restores_parts_after_checking_and_rewrites_paths() {
        let _s = crate::tests::serial();
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let folder = d.path().join("backups");
        fs::create_dir(&folder).unwrap();
        let root = data.root.to_string_lossy().into_owned();
        fs::create_dir_all(data.root.join("GamesConfig")).unwrap();
        fs::write(data.root.join("GamesConfig/g.json"), format!(r#"{{"winePrefix":"{root}/prefixes/g"}}"#)).unwrap();
        let made = snapshot::create(&data, &app, &folder, &Part::ALL, Reason::Manual, Local::now()).unwrap();

        // Changes after the backup.
        fs::write(data.root.join("local_library/library.json"), r#"{"games":{}}"#).unwrap();
        fs::write(data.root.join("GamesConfig/new.json"), "{}").unwrap();
        fs::write(app.config.join("ui-state.json"), r#"{"changed":true}"#).unwrap();
        fs::write(data.root.join("local_library/art/x/hero-1.png"), "new art").unwrap();
        fs::write(data.root.join("tools/proton/new"), "x").unwrap();

        let done = restore(&data, &app, &made.path, &[Part::Collection, Part::Settings]).unwrap();
        assert_eq!(done.parts, [Part::Collection, Part::Settings]);
        assert_eq!(fs::read_to_string(data.root.join("local_library/library.json")).unwrap(), r#"{"games":{"a":{},"b":{}}}"#);
        assert!(!data.root.join("GamesConfig/new.json").exists());
        assert_eq!(fs::read_to_string(app.config.join("ui-state.json")).unwrap(), "{}");
        // Art was not asked for; left-out files are never touched.
        assert_eq!(fs::read_to_string(data.root.join("local_library/art/x/hero-1.png")).unwrap(), "new art");
        assert!(data.root.join("tools/proton/new").exists());
        assert!(data.root.join("local_library/heroes/1.jpg").exists());
        assert_eq!(done.rewritten, 0);

        // Into another data folder: the paths follow it.
        let other = DataPaths::at(d.path().join("moved"));
        let other_app = AppPaths { config: d.path().join("moved-config"), cache: d.path().join("moved-cache") };
        let done = restore(&other, &other_app, &made.path, &Part::ALL).unwrap();
        assert!(done.rewritten >= 1);
        let moved = other.root.to_string_lossy().replace('\\', "/");
        assert_eq!(fs::read_to_string(other.root.join("GamesConfig/g.json")).unwrap(), format!(r#"{{"winePrefix":"{moved}/prefixes/g"}}"#));
        assert_eq!(fs::read_to_string(other.root.join("legendaryConfig/legendary/user.json")).unwrap(), "token");
    }

    #[test]
    fn a_backup_from_linux_points_at_this_machine_s_folders() {
        let _s = crate::tests::serial();
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let folder = d.path().join("backups");
        fs::create_dir(&folder).unwrap();
        fs::create_dir_all(data.root.join("GamesConfig")).unwrap();
        let config = r#"{"winePrefix":"/home/u/.local/share/heronite/prefixes/g","savesPath":"/home/u/Games/g","other":"/home/u2/x"}"#;
        fs::write(data.root.join("GamesConfig/g.json"), config).unwrap();
        let made = snapshot::create(&data, &app, &folder, &Part::ALL, Reason::Manual, Local::now()).unwrap();
        let mut manifest = Manifest::read(&made.path).unwrap();
        (manifest.data_root, manifest.config_root, manifest.home) =
            ("/home/u/.local/share/heronite".into(), "/home/u/.config/heronite".into(), "/home/u".into());
        manifest.write(&made.path).unwrap();

        let here = DataPaths::at(d.path().join("here"));
        let here_app = AppPaths { config: d.path().join("here-config"), cache: d.path().join("here-cache") };
        let done = restore(&here, &here_app, &made.path, &Part::ALL).unwrap();
        assert!(done.rewritten >= 1);
        let text = fs::read_to_string(here.root.join("GamesConfig/g.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
        let slashed = |p: &Path| p.to_string_lossy().replace('\\', "/");
        assert_eq!(value["winePrefix"], format!("{}/prefixes/g", slashed(&here.root)));
        assert_eq!(value["savesPath"], format!("{}/Games/g", snapshot::dirs_home().replace('\\', "/")));
        assert_eq!(value["other"], "/home/u2/x", "only whole folders are rewritten");
    }

    #[test]
    fn a_backup_from_windows_has_its_escaped_folders_rewritten() {
        let m = Manifest {
            format: FORMAT,
            app_version: String::new(),
            created_at: String::new(),
            reason: Reason::Manual,
            host: String::new(),
            home: r"C:\Users\a".into(),
            data_root: r"C:\Users\a\AppData\Roaming\heronite\data".into(),
            config_root: r"C:\Users\a\AppData\Roaming\heronite\config".into(),
            parts: Vec::new(),
            summary: Default::default(),
            files: Vec::new(),
        };
        let data = DataPaths::at("/new/data");
        let app = AppPaths { config: "/new/config".into(), cache: "/new/cache".into() };
        let text = r#"{"winePrefix":"C:\\Users\\a\\AppData\\Roaming\\heronite\\data\\prefixes\\g","x":"C:\\Users\\a\\Games"}"#;
        let out = String::from_utf8(rewrite_paths("GamesConfig/g.json", text.as_bytes(), &rewrites(&m, &data, &app)).unwrap()).unwrap();
        let home = snapshot::dirs_home().replace('\\', "/");
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["winePrefix"], r"/new/data\prefixes\g");
        assert_eq!(value["x"], format!(r"{home}\Games"));
    }

    #[test]
    fn a_damaged_backup_changes_nothing() {
        let _s = crate::tests::serial();
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let folder = d.path().join("backups");
        fs::create_dir(&folder).unwrap();
        let made = snapshot::create(&data, &app, &folder, &Part::ALL, Reason::Manual, Local::now()).unwrap();
        fs::write(made.path.join("data/config.json"), "{}!").unwrap();
        fs::write(data.root.join("local_library/library.json"), "now").unwrap();
        let err = restore(&data, &app, &made.path, &[Part::Collection]).unwrap_err();
        assert_eq!(err.downcast_ref::<Error>(), Some(&Error::Damaged("config.json".into())));
        assert_eq!(fs::read_to_string(data.root.join("local_library/library.json")).unwrap(), "now");
        assert!(prepare(&data, &app, &made.path, &[Part::Collection], &folder).is_err());
        assert!(!app.config.join(PENDING_FILE).exists());
    }

    #[test]
    fn prepare_backs_up_first_and_the_next_start_applies_it() {
        let _s = crate::tests::serial();
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let folder = d.path().join("backups");
        fs::create_dir(&folder).unwrap();
        let made = snapshot::create(&data, &app, &folder, &Part::ALL, Reason::Manual, Local::now()).unwrap();
        fs::write(data.root.join("local_library/library.json"), "now").unwrap();

        let safety = prepare(&data, &app, &made.path, &[Part::Collection], &folder).unwrap();
        assert_eq!(fs::read_to_string(safety.join("data/local_library/library.json")).unwrap(), "now");
        assert_eq!(snapshot::read_info(&safety).unwrap().reason, Reason::BeforeRestore);
        // Nothing changes until the next start.
        assert_eq!(fs::read_to_string(data.root.join("local_library/library.json")).unwrap(), "now");

        let done = apply_pending(&data, &app).unwrap().unwrap();
        assert_eq!(done.parts, [Part::Collection]);
        assert_eq!(fs::read_to_string(data.root.join("local_library/library.json")).unwrap(), r#"{"games":{"a":{},"b":{}}}"#);
        assert!(apply_pending(&data, &app).is_none());

        // A stale note is dropped.
        let old = Pending { snapshot: made.path.clone(), parts: vec![Part::Collection], requested_at: "2020-01-01T00:00:00Z".into() };
        fs::write(app.config.join(PENDING_FILE), serde_json::to_string(&old).unwrap()).unwrap();
        assert!(apply_pending(&data, &app).is_none());
        assert!(!app.config.join(PENDING_FILE).exists());
    }

    #[test]
    fn a_folder_without_a_manifest_is_refused() {
        let _s = crate::tests::serial();
        let d = tempfile::tempdir().unwrap();
        let (data, app) = fixture(d.path());
        let old = d.path().join("backups/heronite-2026-10-13");
        fs::create_dir_all(old.join("local_library")).unwrap();
        fs::write(old.join("local_library/library.json"), r#"{"games":{}}"#).unwrap();
        assert!(restore(&data, &app, &old, &Part::ALL).is_err());
        assert!(prepare(&data, &app, &old, &Part::ALL, &d.path().join("backups")).is_err());
        assert_eq!(fs::read_to_string(data.root.join("local_library/library.json")).unwrap(), r#"{"games":{"a":{},"b":{}}}"#);
    }
}
