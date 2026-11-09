//! The account (or folder) everything goes to, with one sign-in, and what
//! goes there besides the library sync:
//!
//! ```text
//! <root>/Sync/devices/<id>/state.json        the library sync (transport)
//! <root>/Backups/<PC>/heronite-<date>.zip     each backup, without sign-ins
//! <root>/Backups/<PC>/art, metadata-art       the artwork, sent once
//! <root>/Saves/<PC>/<game>/…                  Ludusavi's backups, mirrored
//! <root>/Screenshots/<game>/…                 the captures, one file each
//! ```
//!
//! A backup goes up as one zip (hundreds of small files would take far
//! longer); the artwork, which barely changes and does not compress, goes
//! next to it as files, so only new pictures are sent. Ludusavi's backups
//! already are a zip per game. The screenshots are WebP already: a zip would
//! save nothing and send a month again for each new capture, so they go one
//! by one, several at a time, and only the ones not there yet.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use heronite_backup::{Manifest, Part};
use heronite_data::proc::Cancel;
use heronite_data::{AppPaths, DataPaths};

use crate::prefs::{self, LastUpload, Mode, Provider, SyncPrefs};
use crate::rclone::{self, remote_path, Rclone, RcloneError};
use crate::SyncError;
use crate::{sent, usage};

pub const SYNC_DIR: &str = "Sync";
pub const BACKUPS_DIR: &str = "Backups";
pub const SAVES_DIR: &str = "Saves";
pub const CAPTURES_DIR: &str = "Screenshots";

/// Ludusavi backups the mirror may delete in one go (its retention removes
/// old ones); more means the local folder is not what it was, and nothing
/// is deleted.
const MAX_SAVES_DELETED: &str = "50";

/// What goes up besides the library sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Upload {
    Backups,
    Saves,
    Captures,
}

impl Upload {
    pub const ALL: [Upload; 3] = [Upload::Backups, Upload::Saves, Upload::Captures];

    /// The key in `sync.json` `uploads`.
    pub fn key(self) -> &'static str {
        match self {
            Upload::Backups => "backups",
            Upload::Saves => "saves",
            Upload::Captures => "captures",
        }
    }

    pub fn mode(self, p: &SyncPrefs) -> Mode {
        match self {
            Upload::Backups => p.backups,
            Upload::Saves => p.saves,
            Upload::Captures => p.captures,
        }
    }
}

/// rclone and the root everything goes under. A folder goes through rclone
/// too (a local path to it).
pub fn root(data: &DataPaths, app: &AppPaths, p: &SyncPrefs) -> Result<(Rclone, String), SyncError> {
    if p.provider == Provider::Folder {
        let folder = p.folder.trim();
        if !Path::new(folder).is_dir() {
            return Err(SyncError::FolderMissing(folder.to_owned()));
        }
    }
    let bin = rclone::ensure(data, &|| false).map_err(SyncError::NoRclone)?;
    Ok(match p.provider {
        Provider::Folder => (Rclone { bin, config: None }, p.folder.trim().to_owned()),
        Provider::Custom => (Rclone { bin, config: None }, p.custom_remote.trim().to_owned()),
        _ => (Rclone { bin, config: Some(app.rclone_conf()) }, remote_path(&format!("{}:", rclone::REMOTE), &p.remote_folder)),
    })
}

/// The settings, when `upload` may run now (asked by the user: `manual`)
/// and has somewhere to go.
fn prefs_for(app: &AppPaths, upload: Upload, manual: bool) -> Result<SyncPrefs, SyncError> {
    let p = prefs::load(app).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    if !upload.mode(&p).allows(manual) || !p.ready() {
        return Err(SyncError::Off);
    }
    Ok(p)
}

/// This PC's folder name in the account, fixed the first time it is asked.
fn pc_folder(app: &AppPaths, p: &SyncPrefs) -> String {
    if !p.device_folder.is_empty() {
        return p.device_folder.clone();
    }
    let name: String = p
        .shown_name()
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    let name = name.trim().trim_matches('.').to_owned();
    let name = if name.is_empty() { p.device_id.clone() } else { name };
    if let Err(e) = prefs::update(app, |p| p.device_folder.clone_from(&name)) {
        log::warn!("cloud: saving this PC's folder name: {e:#}");
    }
    name
}

fn cloud_error(e: RcloneError) -> SyncError {
    match e {
        RcloneError::Cancelled => SyncError::Cancelled,
        other => SyncError::from_cloud(other.to_string()),
    }
}

/// Keeps how `upload` went in `sync.json` (not when it was off or stopped).
fn record(app: &AppPaths, upload: Upload, result: &Result<(), SyncError>) {
    if matches!(result, Err(SyncError::Off | SyncError::Cancelled)) {
        return;
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let saved = prefs::update(app, |p| {
        let last = p.uploads.entry(upload.key().to_owned()).or_insert_with(LastUpload::default);
        match result {
            Ok(()) => {
                last.at = Some(now);
                last.error = None;
            }
            Err(e) => last.error = Some(e.localized()),
        }
    });
    if let Err(e) = saved {
        log::warn!("cloud: saving how the upload went: {e:#}");
    }
}

/// Sends the backup in `backup` (a backup folder of `heronite_backup`) as
/// one zip, the artwork next to it, and keeps the newest `keep` zips of this
/// PC (0: all).
pub fn upload_backup(data: &DataPaths, app: &AppPaths, backup: &Path, keep: u32, manual: bool, cancel: &Cancel) -> Result<(), SyncError> {
    let result = (|| {
        let p = prefs_for(app, Upload::Backups, manual)?;
        let (rc, root) = root(data, app, &p)?;
        let dest = remote_path(&root, &format!("{BACKUPS_DIR}/{}", pc_folder(app, &p)));
        let name = backup.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or(SyncError::Off)?;
        // Sent already (the app restarted after it, say): only on request.
        if !manual && sent::load(app, &p.destination()).backups.contains(&name) {
            return Ok(());
        }
        let outbox = app.sync_mirror_dir().join("outbox");
        fs::create_dir_all(&outbox).map_err(|e| SyncError::Data(e.to_string()))?;
        clear_stale_zips(&outbox);
        let zip = outbox.join(format!("{name}.zip"));
        let made = backup_zip(backup, &zip).map_err(|e| SyncError::Data(format!("{e:#}")));
        let sent = made.and_then(|()| {
            rc.copy_file(&zip.to_string_lossy(), &remote_path(&dest, &format!("{name}.zip")), Some(cancel)).map_err(cloud_error)
        });
        let _ = fs::remove_file(&zip);
        sent?;
        let art = art_files(data);
        for dir in ["art", "metadata-art"] {
            let local = data.root.join("local_library").join(dir);
            if local.is_dir() {
                let (src, to) = (local.to_string_lossy().into_owned(), remote_path(&dest, dir));
                rc.transfer(&["copy", &src, &to], cancel, &mut |_| {}).map_err(cloud_error)?;
            }
        }
        prune_backups(&rc, &dest, keep)?;
        keep_sent(app, &p, |s| {
            s.backups.insert(name.clone());
            s.art = art;
        });
        Ok(())
    })();
    record(app, Upload::Backups, &result);
    result
}

/// The artwork as the backups send it: `art/…`, `metadata-art/…` → bytes.
fn art_files(data: &DataPaths) -> BTreeMap<String, u64> {
    let mut files = BTreeMap::new();
    for dir in ["art", "metadata-art"] {
        for f in usage::walk(&data.root.join("local_library").join(dir)) {
            files.insert(format!("{dir}/{}", f.rel), f.size);
        }
    }
    files
}

/// Records what an upload sent; a failure only costs the green dot.
fn keep_sent(app: &AppPaths, p: &SyncPrefs, f: impl FnOnce(&mut sent::Sent)) {
    if let Err(e) = sent::update(app, &p.destination(), f) {
        log::warn!("cloud: saving what was sent: {e:#}");
    }
}

/// Zips a stopped upload left behind (the app closed while sending).
fn clear_stale_zips(outbox: &Path) {
    for e in fs::read_dir(outbox).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".zip") || name.ends_with(".zip.partial") {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Keeps the newest `keep` backup zips in `dest` (0: all).
fn prune_backups(rc: &Rclone, dest: &str, keep: u32) -> Result<(), SyncError> {
    if keep == 0 {
        return Ok(());
    }
    let mut zips: Vec<String> = rc
        .list(dest, false)
        .map_err(cloud_error)?
        .into_iter()
        .filter(|n| n.starts_with(heronite_backup::snapshot::PREFIX) && n.ends_with(".zip"))
        .collect();
    // `heronite-YYYY-MM-DD_HHMMSS`: the name sorts by date.
    zips.sort();
    let extra = zips.len().saturating_sub(keep as usize);
    for old in &zips[..extra] {
        rc.delete_file(&remote_path(dest, old)).map_err(cloud_error)?;
    }
    Ok(())
}

/// The backup's Collection and app settings in one zip, with a manifest of
/// only them: a restore of it checks what is there and leaves the artwork
/// and the sign-ins of the PC alone.
fn backup_zip(backup: &Path, out: &Path) -> anyhow::Result<()> {
    let mut manifest = Manifest::read(backup)?;
    let kept = |part: Part| matches!(part, Part::Collection | Part::Settings);
    manifest.files.retain(|f| kept(f.part));
    manifest.parts.retain(|p| kept(*p));
    manifest.summary.files = manifest.files.len();
    manifest.summary.bytes = manifest.files.iter().map(|f| f.size).sum();
    manifest.summary.new_bytes = manifest.summary.bytes;
    let partial = out.with_extension("zip.partial");
    let file = fs::File::create(&partial)?;
    let mut zip = zip::ZipWriter::new(io::BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for f in &manifest.files {
        let rel = format!("{}/{}", f.side.dir_name(), f.path);
        let mut src = fs::File::open(backup.join(&rel))?;
        zip.start_file(rel, options)?;
        io::copy(&mut src, &mut zip)?;
    }
    zip.start_file(heronite_backup::manifest::MANIFEST_FILE, options)?;
    io::Write::write_all(&mut zip, serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    zip.finish()?;
    fs::rename(&partial, out)?;
    Ok(())
}

/// Mirrors Ludusavi's backup folder (`saves`) into `Saves/<PC>`.
pub fn upload_saves(
    data: &DataPaths,
    app: &AppPaths,
    saves: &Path,
    manual: bool,
    cancel: &Cancel,
    on_percent: &mut dyn FnMut(u8),
) -> Result<(), SyncError> {
    let result = (|| {
        let p = prefs_for(app, Upload::Saves, manual)?;
        // An empty or missing folder (a drive not mounted) must not empty
        // the copy in the account.
        if fs::read_dir(saves).map_or(true, |mut d| d.next().is_none()) {
            return Err(SyncError::FolderMissing(saves.display().to_string()));
        }
        let (rc, root) = root(data, app, &p)?;
        let dest = remote_path(&root, &format!("{SAVES_DIR}/{}", pc_folder(app, &p)));
        let src = saves.to_string_lossy().into_owned();
        let files = usage::walk(saves).into_iter().filter(|f| !f.rel.ends_with(".tmp")).map(|f| (f.rel, f.size)).collect();
        rc.transfer(&["sync", &src, &dest, "--max-delete", MAX_SAVES_DELETED, "--exclude", "*.tmp"], cancel, on_percent)
            .map_err(cloud_error)?;
        keep_sent(app, &p, |s| s.saves = files);
        Ok(())
    })();
    record(app, Upload::Saves, &result);
    result
}

/// Sends the screenshots of `captures` not in the account yet. `pending_png`:
/// PNGs still wait for their compression and are left for a later run.
/// Nothing is deleted there when a capture is deleted here.
pub fn upload_captures(
    data: &DataPaths,
    app: &AppPaths,
    captures: &Path,
    pending_png: bool,
    manual: bool,
    cancel: &Cancel,
    on_percent: &mut dyn FnMut(u8),
) -> Result<(), SyncError> {
    let result = (|| {
        let p = prefs_for(app, Upload::Captures, manual)?;
        if !captures.is_dir() {
            return Err(SyncError::FolderMissing(captures.display().to_string()));
        }
        let (rc, root) = root(data, app, &p)?;
        let dest = remote_path(&root, CAPTURES_DIR);
        let src = captures.to_string_lossy().into_owned();
        let mut args = vec!["copy", src.as_str(), dest.as_str(), "--ignore-case"];
        // The replay buffer's own folder, and files still being written.
        args.extend(["--exclude", ".heronite-replay/**", "--exclude", "*.{tmp,partial,part}"]);
        if pending_png {
            args.extend(["--exclude", "*.png"]);
        }
        if !p.replays {
            args.extend(["--exclude", "*.{mp4,mkv,webm,mov}", "--exclude", "**/replays/**"]);
        }
        // What this run sets out to send; a capture taken meanwhile waits
        // for the next one.
        let files: BTreeMap<String, u64> = usage::walk(captures)
            .into_iter()
            .filter(|f| usage::capture_goes(&f.rel, p.replays, pending_png))
            .map(|f| (f.rel, f.size))
            .collect();
        rc.transfer(&args, cancel, on_percent).map_err(cloud_error)?;
        // Deleting one here keeps it in the cloud: still sent.
        keep_sent(app, &p, |s| s.captures.extend(files));
        Ok(())
    })();
    record(app, Upload::Captures, &result);
    result
}

/// A backup in the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudBackup {
    /// The PC's folder (`Backups/<pc>`).
    pub pc: String,
    /// `heronite-YYYY-MM-DD_HHMMSS`.
    pub name: String,
    pub bytes: u64,
}

impl CloudBackup {
    /// When it was made, from its name (`YYYY-MM-DD HH:MM:SS`, local time
    /// of the PC that made it).
    pub fn made(&self) -> Option<chrono::NaiveDateTime> {
        let stamp = self.name.strip_prefix(heronite_backup::snapshot::PREFIX)?;
        chrono::NaiveDateTime::parse_from_str(stamp.get(..17)?, "%Y-%m-%d_%H%M%S").ok()
    }
}

/// The backups of every PC in the account, newest first (one listing, when
/// the user asks).
pub fn list_backups(data: &DataPaths, app: &AppPaths) -> Result<Vec<CloudBackup>, SyncError> {
    let p = prefs::load(app).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    if !p.ready() {
        return Err(SyncError::Off);
    }
    let (rc, root) = root(data, app, &p)?;
    let files = rc.list_files(&remote_path(&root, BACKUPS_DIR), "/*/heronite-*.zip").map_err(cloud_error)?;
    let mut list: Vec<CloudBackup> = files
        .into_iter()
        .filter_map(|(path, bytes)| {
            let (pc, file) = path.split_once('/')?;
            Some(CloudBackup { pc: pc.to_owned(), name: file.strip_suffix(".zip")?.to_owned(), bytes })
        })
        .collect();
    list.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(list)
}

/// Brings `backup` into `into` (the backup folder), where Settings → Backups
/// lists and restores it, with the artwork of its PC missing here. Its
/// folder there (already there: nothing is downloaded).
pub fn download_backup(data: &DataPaths, app: &AppPaths, backup: &CloudBackup, into: &Path, cancel: &Cancel) -> Result<PathBuf, SyncError> {
    let target = into.join(&backup.name);
    if target.join(heronite_backup::manifest::MANIFEST_FILE).is_file() {
        return Ok(target);
    }
    let p = prefs::load(app).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    if !p.ready() {
        return Err(SyncError::Off);
    }
    let (rc, root) = root(data, app, &p)?;
    let dir = remote_path(&root, &format!("{BACKUPS_DIR}/{}", backup.pc));
    let inbox = app.sync_mirror_dir().join("inbox");
    fs::create_dir_all(&inbox).map_err(|e| SyncError::Data(e.to_string()))?;
    fs::create_dir_all(into).map_err(|e| SyncError::Data(e.to_string()))?;
    let zip = inbox.join(format!("{}.zip", backup.name));
    rc.copy_file(&remote_path(&dir, &format!("{}.zip", backup.name)), &zip.to_string_lossy(), Some(cancel)).map_err(cloud_error)?;
    let unpacked = unpack_backup(&zip, into, &backup.name).map_err(|e| SyncError::Data(format!("{e:#}")));
    let _ = fs::remove_file(&zip);
    unpacked?;
    for art in ["art", "metadata-art"] {
        let local = data.root.join("local_library").join(art).to_string_lossy().into_owned();
        match rc.transfer(&["copy", &remote_path(&dir, art), &local, "--ignore-existing"], cancel, &mut |_| {}) {
            Ok(()) | Err(RcloneError::NotFound) => {}
            Err(e) => return Err(cloud_error(e)),
        }
    }
    Ok(target)
}

/// What the account holds to bring back to a PC (a new one, or one
/// formatted): the backups, the PCs with game saves, whether screenshots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InCloud {
    /// Newest first.
    pub backups: Vec<CloudBackup>,
    /// The PCs whose game saves are there (`Saves/<pc>`).
    pub saves: Vec<String>,
    pub captures: bool,
}

impl InCloud {
    /// Every PC with something there, the one of the newest backup first.
    pub fn pcs(&self) -> Vec<String> {
        let mut pcs: Vec<String> = Vec::new();
        for pc in self.backups.iter().map(|b| &b.pc).chain(self.saves.iter()) {
            if !pcs.contains(pc) {
                pcs.push(pc.clone());
            }
        }
        pcs
    }
}

/// Lists what can be brought back (a few listings, when the user asks).
pub fn in_cloud(data: &DataPaths, app: &AppPaths) -> Result<InCloud, SyncError> {
    let backups = list_backups(data, app)?;
    let p = prefs::load(app).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    let (rc, root) = root(data, app, &p)?;
    let saves = rc.list(&remote_path(&root, SAVES_DIR), true).map_err(cloud_error)?;
    let captures = !rc.list(&remote_path(&root, CAPTURES_DIR), true).map_err(cloud_error)?.is_empty();
    Ok(InCloud { backups, saves, captures })
}

/// Brings the game saves `pc` sent into `into`, Ludusavi's backup folder:
/// only its backups come back, a game's own save folder is never touched
/// (restoring one is Ludusavi's, from the game's Saves tab). What is newer
/// here stays; nothing here is deleted.
pub fn download_saves(
    data: &DataPaths,
    app: &AppPaths,
    pc: &str,
    into: &Path,
    cancel: &Cancel,
    on_percent: &mut dyn FnMut(u8),
) -> Result<(), SyncError> {
    let p = prefs::load(app).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    if !p.ready() {
        return Err(SyncError::Off);
    }
    let (rc, root) = root(data, app, &p)?;
    fs::create_dir_all(into).map_err(|e| SyncError::Data(e.to_string()))?;
    let from = remote_path(&root, &format!("{SAVES_DIR}/{pc}"));
    let to = into.to_string_lossy().into_owned();
    rc.transfer(&["copy", &from, &to, "--update"], cancel, on_percent).map_err(cloud_error)
}

/// Brings the screenshots of the account into `into` (the captures folder);
/// the ones already here are left as they are.
pub fn download_captures(
    data: &DataPaths,
    app: &AppPaths,
    into: &Path,
    cancel: &Cancel,
    on_percent: &mut dyn FnMut(u8),
) -> Result<(), SyncError> {
    let p = prefs::load(app).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    if !p.ready() {
        return Err(SyncError::Off);
    }
    let (rc, root) = root(data, app, &p)?;
    fs::create_dir_all(into).map_err(|e| SyncError::Data(e.to_string()))?;
    let from = remote_path(&root, CAPTURES_DIR);
    let to = into.to_string_lossy().into_owned();
    rc.transfer(&["copy", &from, &to, "--ignore-existing"], cancel, on_percent).map_err(cloud_error)?;
    // What came down is in the cloud already: it does not show as not sent.
    let files: BTreeMap<String, u64> =
        usage::walk(into).into_iter().filter(|f| usage::capture_goes(&f.rel, true, false)).map(|f| (f.rel, f.size)).collect();
    let listed = rc.list_files(&from, "**").map_err(cloud_error)?;
    let there: std::collections::HashMap<String, u64> = listed.into_iter().collect();
    keep_sent(app, &p, |s| s.captures.extend(files.into_iter().filter(|(rel, size)| there.get(rel) == Some(size))));
    Ok(())
}

/// Unpacks a backup zip as `into/<name>` (through `<name>.partial`, which
/// the backup list skips).
fn unpack_backup(zip: &Path, into: &Path, name: &str) -> anyhow::Result<()> {
    let partial: PathBuf = into.join(format!("{name}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(zip)?)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // Never outside the folder.
        let Some(rel) = entry.enclosed_name() else { continue };
        let target = partial.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut entry, &mut fs::File::create(&target)?)?;
    }
    Manifest::read(&partial)?;
    fs::rename(&partial, into.join(name))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_backup::manifest::{FileEntry, Reason, Summary};
    use heronite_backup::Side;

    fn entry(side: Side, path: &str, part: Part) -> FileEntry {
        FileEntry { side, path: path.into(), size: 2, mtime_ms: 0, sha256: String::new(), part }
    }

    #[test]
    fn a_backup_zip_keeps_the_collection_and_settings_and_unpacks_as_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("heronite-2026-11-01_100000");
        let files = [
            entry(Side::Data, "local_library/sessions.json", Part::Collection),
            entry(Side::Config, "ui-state.json", Part::Settings),
            entry(Side::Data, "local_library/art/a.png", Part::Art),
            entry(Side::Data, "gog_store/auth.json", Part::Logins),
        ];
        for f in &files {
            let p = backup.join(f.side.dir_name()).join(&f.path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"{}").unwrap();
        }
        let manifest = Manifest {
            format: heronite_backup::manifest::FORMAT,
            app_version: "1".into(),
            created_at: "2026-11-01T10:00:00Z".into(),
            reason: Reason::Scheduled,
            host: String::new(),
            home: String::new(),
            data_root: String::new(),
            config_root: String::new(),
            parts: Part::ALL.to_vec(),
            summary: Summary::default(),
            files: files.to_vec(),
        };
        manifest.write(&backup).unwrap();
        let zip = dir.path().join("b.zip");
        backup_zip(&backup, &zip).unwrap();

        let into = dir.path().join("restored");
        fs::create_dir_all(&into).unwrap();
        unpack_backup(&zip, &into, "heronite-2026-11-01_100000").unwrap();
        let got = Manifest::read(&into.join("heronite-2026-11-01_100000")).unwrap();
        assert_eq!(got.parts, [Part::Collection, Part::Settings]);
        assert_eq!(got.files.len(), 2);
        assert!(into.join("heronite-2026-11-01_100000/data/local_library/sessions.json").is_file());
        assert!(!into.join("heronite-2026-11-01_100000/data/gog_store/auth.json").exists());
        assert!(!into.join("heronite-2026-11-01_100000.partial").exists());
    }
}
