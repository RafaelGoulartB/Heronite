//! A portable copy on Windows: a folder holding `heronite-rs.exe` and the
//! marker `portable.txt` ([`heronite_platform::dirs::PORTABLE_MARKER`]),
//! with the user's `data`, `config` and `cache` folders beside them.
//!
//! The release's `Heronite-Portable-<arch>.zip` has one top folder,
//! `Heronite/`, with the program files. An update downloads it into
//! `<copy>\cache\update` (same volume as the copy, so moves are renames),
//! checks its signature, unpacks it there while refusing any entry that
//! could land elsewhere (absolute paths, `..`, links, anything outside
//! `Heronite/` or inside the user's folders), then swaps the files in: each
//! file replaced is first renamed to `<name>.old` (Windows lets a running
//! program or a loaded library be renamed, not deleted or overwritten), the
//! new one is moved in, and a failure puts every old file back. The running
//! app keeps its renamed program; the next start runs the new one and
//! deletes the `.old` files ([`Portable::sweep`]).

use heronite_i18n::tr;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::install::{download_signed, InstallError, Progress, Unsupported};
use crate::release::{Release, WindowsAsset};

/// The program, in the zip and in a copy.
pub const PROGRAM: &str = "heronite-rs.exe";
/// The zip's one top folder.
pub const TOP: &str = "Heronite";
/// A copy's folders that belong to the user: never written by an update.
const USER_DIRS: [&str; 3] = ["data", "config", "cache"];
/// Limits of what an update unpacks (a few files of some MB in practice).
const MAX_FILES: usize = 10_000;
const MAX_BYTES: u64 = 4 << 30;

/// Held while files of a copy are swapped or swept.
static SWAP: Mutex<()> = Mutex::new(());

/// A portable copy, writable in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Portable {
    /// The copy's folder ([`heronite_platform::dirs::portable_root`]).
    pub root: PathBuf,
    /// This program, read at start (normally `<root>\heronite-rs.exe`).
    pub exe: PathBuf,
}

impl Portable {
    /// The copy in `root`, run from `exe`. Refused when the folder cannot
    /// be written (a read-only medium, a protected folder).
    pub fn detect(root: PathBuf, exe: PathBuf) -> Result<Self, Unsupported> {
        if !heronite_platform::fs::writable_dir(&root) {
            return Err(Unsupported::ReadOnly(root));
        }
        Ok(Self { root, exe })
    }

    /// Where an update is downloaded and unpacked.
    pub fn staging(&self) -> PathBuf {
        self.root.join("cache").join("update")
    }

    /// The `.old` files the last update left, relative to the copy.
    fn old_list(&self) -> PathBuf {
        self.root.join("cache").join("update-old.txt")
    }

    /// Removes what the last update left: an unfinished download and the
    /// files it replaced. Blocks for up to a few seconds while the previous
    /// process (a restart) still runs its old program.
    pub fn sweep(&self) {
        let _lock = SWAP.lock().unwrap_or_else(|e| e.into_inner());
        if fs::remove_dir_all(self.staging()).is_ok() {
            log::info!("removed the unfinished update {}", self.staging().display());
        }
        let list = self.old_list();
        let Ok(text) = fs::read_to_string(&list) else { return };
        let mut olds: Vec<PathBuf> = text.lines().filter_map(|l| safe_relative(l.trim())).map(|rel| self.root.join(rel)).collect();
        for _ in 0..40 {
            olds.retain(|p| match fs::remove_file(p) {
                Ok(()) => {
                    log::info!("removed {} (replaced by the last update)", p.display());
                    false
                }
                Err(e) => e.kind() != io::ErrorKind::NotFound,
            });
            if olds.is_empty() {
                let _ = fs::remove_file(&list);
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        log::warn!("could not remove {olds:?}; trying again at the next start");
    }
}

/// `line` of the `.old` list as a path inside the copy (only `….old` files,
/// never in the user's folders).
fn safe_relative(line: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = line.split('\\').collect();
    let ok = line.ends_with(".old") && parts.iter().all(|p| valid_component(p)) && !is_user_dir(parts[0]);
    ok.then(|| parts.iter().collect())
}

fn failed(m: impl std::fmt::Display) -> InstallError {
    InstallError::Failed(m.to_string())
}

/// Downloads the portable zip of `release`, checks it against `public_key`
/// ([`crate::PUBLIC_KEY`]) and swaps its files into `copy`. Nothing in the
/// copy changes unless every file arrived and checked out.
pub fn install(
    release: &Release,
    copy: &Portable,
    public_key: &str,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), InstallError> {
    let kind = WindowsAsset::Portable;
    let asset = release.windows_asset(kind).ok_or_else(|| failed(tr!("This release has no portable zip")))?;
    let signature = release.windows_signature(kind).ok_or_else(|| failed(tr!("This release is not signed")))?;
    let staging = copy.staging();
    let _ = fs::remove_dir_all(&staging);
    let result = (|| {
        let zip = download_signed(asset, signature, &release.version, &staging, public_key, cancel, on_progress)?;
        let unpacked = staging.join("new");
        let files = unpack(&zip, &unpacked, cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        let _lock = SWAP.lock().unwrap_or_else(|e| e.into_inner());
        let olds = swap_in(&unpacked, &files, &copy.root, &exe_name(&copy.exe))?;
        remember_olds(&copy.old_list(), &olds);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Adds `olds` (relative to the copy) to the list [`Portable::sweep`] reads.
fn remember_olds(list: &Path, olds: &[PathBuf]) {
    let mut text = fs::read_to_string(list).unwrap_or_default();
    for old in olds {
        let line = old.to_string_lossy();
        if !text.lines().any(|l| l == line) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    if let Err(e) = fs::write(list, text) {
        log::warn!("could not write {}: {e}", list.display());
    }
}

/// A file or folder name Windows takes as it is: not empty, not `.` or
/// `..`, no separator, drive, stream (`:`) or wildcard, no control
/// character, no trailing dot or space (Windows drops them), not a device
/// (`CON`, `NUL`, `COM1`…).
fn valid_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.ends_with(['.', ' ']) {
        return false;
    }
    if name.chars().any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    !device
}

fn is_user_dir(first: &str) -> bool {
    USER_DIRS.iter().any(|d| d.eq_ignore_ascii_case(first))
}

/// The path inside the copy of the zip entry `name`: `None` for the top
/// folder itself; an error for anything outside `Heronite/`, absolute or
/// climbing, with a bad name, or inside the user's folders.
fn entry_path(name: &str) -> Result<Option<PathBuf>, String> {
    let bad = || tr!("The update has a file outside its folder: {name}", name = format!("{name:?}"));
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    let mut parts = trimmed.split('/');
    if parts.next() != Some(TOP) {
        return Err(bad());
    }
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() {
        return Ok(None);
    }
    if !rest.iter().all(|p| valid_component(p)) || is_user_dir(rest[0]) {
        return Err(bad());
    }
    let path: PathBuf = rest.iter().collect();
    // Belt and braces: only plain names remain.
    if !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(bad());
    }
    Ok(Some(path))
}

/// Unpacks the program files of the portable `zip` into `dest` (created,
/// must not exist). Returns the files, relative to `dest`. Refuses unsafe
/// entries, links, encrypted entries, a zip without [`PROGRAM`] and
/// oversized ones.
fn unpack(zip: &Path, dest: &Path, cancel: &AtomicBool) -> Result<Vec<PathBuf>, InstallError> {
    let unreadable = |e: &dyn std::fmt::Display| failed(tr!("The update could not be unpacked: {error}", error = e));
    let file = File::open(zip).map_err(|e| unreadable(&e))?;
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file)).map_err(|e| unreadable(&e))?;
    if archive.len() > MAX_FILES {
        return Err(unreadable(&format!("{} entries", archive.len())));
    }
    fs::create_dir(dest).map_err(|e| failed(tr!("Could not create {path}: {error}", path = dest.display(), error = e)))?;
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        let mut entry = archive.by_index(i).map_err(|e| unreadable(&e))?;
        let name = std::str::from_utf8(entry.name_raw()).map_err(|_| unreadable(&"a file name is not UTF-8"))?.to_owned();
        let Some(rel) = entry_path(&name).map_err(failed)? else { continue };
        // FILE_ATTRIBUTE_REPARSE_POINT, for zips made on Windows.
        let reparse = entry.external_attributes() & 0x400 != 0;
        if entry.is_symlink() || reparse || entry.encrypted() {
            return Err(failed(tr!("The update has a link or an encrypted file: {name}", name = format!("{name:?}"))));
        }
        if !seen.insert(rel.to_string_lossy().to_lowercase()) {
            return Err(unreadable(&format!("{name:?} twice")));
        }
        let path = dest.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&path).map_err(|e| failed(tr!("Could not create {path}: {error}", path = path.display(), error = e)))?;
            continue;
        }
        total += entry.size();
        if total > MAX_BYTES {
            return Err(unreadable(&"too large"));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| failed(tr!("Could not create {path}: {error}", path = parent.display(), error = e)))?;
        }
        let write_err = |e: io::Error| failed(tr!("Could not write {path}: {error}", path = path.display(), error = e));
        let mut out = File::create_new(&path).map_err(write_err)?;
        let size = entry.size();
        // Reading to the end checks the entry's CRC.
        let copied = io::copy(&mut (&mut entry).take(size + 1), &mut out).map_err(|e| unreadable(&e))?;
        if copied != size {
            return Err(unreadable(&format!("{name:?} is {copied} bytes, not {size}")));
        }
        out.flush().and_then(|()| out.sync_all()).map_err(write_err)?;
        files.push(rel);
    }
    let program = files.iter().find(|f| f.as_os_str().eq_ignore_ascii_case(PROGRAM));
    let is_program = program.is_some_and(|p| {
        let mut head = [0u8; 2];
        File::open(dest.join(p)).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == b"MZ"
    });
    if !is_program {
        return Err(failed(tr!("The update has no {program}", program = PROGRAM)));
    }
    Ok(files)
}

/// `name` + `.old`.
fn old_name(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// One file moved into the copy, for the way back.
struct Moved {
    dest: PathBuf,
    /// Where the file it replaced went.
    old: Option<PathBuf>,
}

/// Moves `files` (relative) from `from` into `root`, the program under the
/// running program's name `exe_name`. Each existing file is renamed to
/// `<name>.old` first. On failure everything done is undone. Returns the
/// `.old` files, relative to `root`.
fn swap_in(from: &Path, files: &[PathBuf], root: &Path, exe_name: &OsStr) -> Result<Vec<PathBuf>, InstallError> {
    let mut moved: Vec<Moved> = Vec::new();
    let mut created: Vec<PathBuf> = Vec::new();
    let mut olds = Vec::new();
    for rel in files {
        let target: PathBuf = if rel.as_os_str().eq_ignore_ascii_case(PROGRAM) { PathBuf::from(exe_name) } else { rel.clone() };
        match move_one(&from.join(rel), root, &target, &mut created) {
            Ok(m) => {
                if m.old.is_some() {
                    olds.push(old_name(&target));
                }
                moved.push(m);
            }
            Err(e) => {
                log::error!("update of {} failed, putting the old files back: {e}", root.display());
                roll_back(moved, created);
                return Err(e);
            }
        }
    }
    Ok(olds)
}

fn move_one(src: &Path, root: &Path, target: &Path, created: &mut Vec<PathBuf>) -> Result<Moved, InstallError> {
    let dest = root.join(target);
    let cant = |e: io::Error| failed(tr!("Could not replace {path}: {error}", path = dest.display(), error = e));
    // Missing folders, outermost first, so the way back removes them.
    let mut missing: Vec<PathBuf> =
        dest.parent().into_iter().flat_map(Path::ancestors).take_while(|d| !d.exists()).map(Path::to_path_buf).collect();
    missing.reverse();
    for dir in missing {
        fs::create_dir(&dir).map_err(cant)?;
        created.push(dir);
    }
    let old = match fs::symlink_metadata(&dest) {
        Ok(m) if m.is_dir() => return Err(cant(io::Error::other("a folder is in the way"))),
        Ok(_) => {
            let old = old_name(&dest);
            // An earlier update's, not swept yet.
            match fs::remove_file(&old) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(cant(e)),
            }
            heronite_platform::fs::rename_replacing(&dest, &old).map_err(cant)?;
            Some(old)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(cant(e)),
    };
    if let Err(e) = heronite_platform::fs::rename_replacing(src, &dest) {
        if let Some(old) = &old {
            let _ = fs::rename(old, &dest);
        }
        return Err(cant(e));
    }
    Ok(Moved { dest, old })
}

/// Puts the old files back, newest first, and removes the folders made.
fn roll_back(moved: Vec<Moved>, created: Vec<PathBuf>) {
    for m in moved.into_iter().rev() {
        if let Err(e) = fs::remove_file(&m.dest) {
            log::warn!("could not remove the new {}: {e}", m.dest.display());
        }
        if let Some(old) = m.old {
            if let Err(e) = fs::rename(&old, &m.dest) {
                log::error!("could not put {} back: {e}", m.dest.display());
            }
        }
    }
    for dir in created.into_iter().rev() {
        let _ = fs::remove_dir(dir);
    }
}

/// The running program's file name: the name the new program gets.
fn exe_name(exe: &Path) -> OsString {
    exe.file_name().map(OsStr::to_owned).unwrap_or_else(|| PROGRAM.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::tests::serve;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    /// A zip of `entries` (name, bytes; a name ending in `/` is a folder).
    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in entries {
            if let Some(dir) = name.strip_suffix('/') {
                w.add_directory(dir, opts).unwrap();
            } else {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
    }

    const EXE: &[u8] = b"MZ new program";

    #[test]
    fn entry_names_stay_inside_the_program_folder() {
        assert_eq!(entry_path("Heronite/"), Ok(None));
        assert_eq!(entry_path("Heronite/heronite-rs.exe"), Ok(Some(PathBuf::from("heronite-rs.exe"))));
        assert_eq!(entry_path("Heronite/res/a b.pak"), Ok(Some(PathBuf::from(r"res\a b.pak"))));
        for bad in [
            "heronite-rs.exe",
            "Other/heronite-rs.exe",
            "heronite/x",
            "/Heronite/x",
            "Heronite/../x",
            "Heronite/a/../../x",
            "Heronite/./x",
            "Heronite//x",
            r"Heronite/a\..\..\x",
            "Heronite/C:x",
            "Heronite/x.exe:stream",
            "Heronite/data/collection.json",
            "Heronite/Config/x",
            "Heronite/CACHE/",
            "Heronite/x.",
            "Heronite/x ",
            "Heronite/NUL",
            "Heronite/com1.txt",
            "Heronite/a\u{1}b",
        ] {
            assert!(entry_path(bad).is_err(), "{bad:?} accepted");
        }
        assert!(valid_component("COM10") && valid_component("console.txt") && valid_component("datafile"));
        assert_eq!(safe_relative(r"heronite-rs.exe.old"), Some(PathBuf::from("heronite-rs.exe.old")));
        assert_eq!(safe_relative(r"res\x.pak.old"), Some(PathBuf::from(r"res\x.pak.old")));
        for bad in [r"..\x.old", r"C:\x.old", "x.exe", r"data\x.old", r"a\\x.old", ""] {
            assert_eq!(safe_relative(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn unpacks_only_safe_zips_with_the_program() {
        let tmp = tempfile::tempdir().unwrap();
        let no = AtomicBool::new(false);
        let zip = tmp.path().join("p.zip");
        make_zip(
            &zip,
            &[("Heronite/", b""), ("Heronite/heronite-rs.exe", EXE), ("Heronite/LICENSE", b"MIT"), ("Heronite/res/a.pak", b"a")],
        );
        let files = unpack(&zip, &tmp.path().join("ok"), &no).unwrap();
        assert_eq!(files, [PathBuf::from("heronite-rs.exe"), "LICENSE".into(), r"res\a.pak".into()]);
        assert_eq!(fs::read(tmp.path().join(r"ok\res\a.pak")).unwrap(), b"a");

        let refused = |entries: &[(&str, &[u8])], why: &str| {
            let dest = tmp.path().join(format!("d{}", rand_suffix()));
            make_zip(&zip, entries);
            let err = unpack(&zip, &dest, &no).unwrap_err().to_string();
            assert!(err.contains(why), "{err}");
        };
        refused(&[("Heronite/heronite-rs.exe", EXE), ("Heronite/../evil.dll", b"x")], "outside");
        refused(&[("Heronite/heronite-rs.exe", EXE), ("Heronite/data/x.json", b"x")], "outside");
        refused(&[("Heronite/LICENSE", b"MIT")], "has no heronite-rs.exe");
        refused(&[("Heronite/heronite-rs.exe", b"#!/bin/sh")], "has no heronite-rs.exe");
        refused(&[("Heronite/heronite-rs.exe", EXE), ("Heronite/LICENSE", b"a"), ("Heronite/license", b"b")], "twice");

        // A symbolic link entry (Unix mode S_IFLNK).
        let link = tmp.path().join("link.zip");
        let mut w = zip::ZipWriter::new(File::create(&link).unwrap());
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        w.start_file("Heronite/heronite-rs.exe", opts).unwrap();
        w.write_all(EXE).unwrap();
        w.add_symlink("Heronite/escape", r"C:\Windows", opts).unwrap();
        w.finish().unwrap();
        let err = unpack(&link, &tmp.path().join("l"), &no).unwrap_err().to_string();
        assert!(err.contains("link"), "{err}");

        // Not a zip at all.
        fs::write(&zip, b"MZ not a zip").unwrap();
        assert!(unpack(&zip, &tmp.path().join("n"), &no).unwrap_err().to_string().contains("could not be unpacked"));
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::AtomicU64;
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    /// A copy with an old program, a marker and the user's folders.
    fn copy_in(dir: &Path) -> Portable {
        let root = dir.join("Heronite");
        for d in ["data", "config", "cache"] {
            fs::create_dir_all(root.join(d)).unwrap();
            fs::write(root.join(d).join("mine.json"), d).unwrap();
        }
        fs::write(root.join(PROGRAM), b"MZ old program").unwrap();
        fs::write(root.join("portable.txt"), b"").unwrap();
        fs::write(root.join("notes.txt"), b"the user's").unwrap();
        Portable::detect(root.clone(), root.join(PROGRAM)).unwrap()
    }

    fn new_files(dir: &Path) -> (PathBuf, Vec<PathBuf>) {
        let from = dir.join("new");
        fs::create_dir_all(from.join("res")).unwrap();
        fs::write(from.join(PROGRAM), EXE).unwrap();
        fs::write(from.join("portable.txt"), b"").unwrap();
        fs::write(from.join(r"res\a.pak"), b"a").unwrap();
        (from, vec![PROGRAM.into(), "portable.txt".into(), r"res\a.pak".into()])
    }

    #[test]
    fn swaps_the_files_and_keeps_the_old_ones_until_the_sweep() {
        let tmp = tempfile::tempdir().unwrap();
        let copy = copy_in(tmp.path());
        let (from, files) = new_files(tmp.path());
        let olds = swap_in(&from, &files, &copy.root, &exe_name(&copy.exe)).unwrap();
        assert_eq!(olds, [PathBuf::from("heronite-rs.exe.old"), "portable.txt.old".into()]);
        assert_eq!(fs::read(copy.root.join(PROGRAM)).unwrap(), EXE);
        assert_eq!(fs::read(copy.root.join("heronite-rs.exe.old")).unwrap(), b"MZ old program");
        assert_eq!(fs::read(copy.root.join(r"res\a.pak")).unwrap(), b"a");
        for d in ["data", "config", "cache"] {
            assert_eq!(fs::read_to_string(copy.root.join(d).join("mine.json")).unwrap(), d);
        }
        assert_eq!(fs::read(copy.root.join("notes.txt")).unwrap(), b"the user's");
        remember_olds(&copy.old_list(), &olds);
        fs::create_dir_all(copy.staging()).unwrap();
        copy.sweep();
        assert!(!copy.root.join("heronite-rs.exe.old").exists() && !copy.root.join("portable.txt.old").exists());
        assert!(!copy.old_list().exists() && !copy.staging().exists());
        assert!(copy.root.join("cache").join("mine.json").exists());
    }

    #[test]
    fn a_renamed_program_is_replaced_under_its_name() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("copy");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Heronite.exe"), b"MZ old").unwrap();
        let copy = Portable::detect(root.clone(), root.join("Heronite.exe")).unwrap();
        let (from, files) = new_files(tmp.path());
        swap_in(&from, &files, &copy.root, &exe_name(&copy.exe)).unwrap();
        assert_eq!(fs::read(root.join("Heronite.exe")).unwrap(), EXE);
        assert!(!root.join(PROGRAM).exists());
    }

    /// A file that cannot be moved (held open without delete sharing) makes
    /// the swap put back what it already replaced.
    #[test]
    fn a_failed_swap_puts_the_old_files_back() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let copy = copy_in(tmp.path());
        let (from, files) = new_files(tmp.path());
        fs::create_dir_all(copy.root.join("res")).unwrap();
        fs::write(copy.root.join(r"res\a.pak"), b"old a").unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(1).open(copy.root.join(r"res\a.pak")).unwrap();
        let err = swap_in(&from, &files, &copy.root, &exe_name(&copy.exe)).unwrap_err();
        drop(held);
        assert!(err.to_string().contains("Could not replace"), "{err}");
        assert_eq!(fs::read(copy.root.join(PROGRAM)).unwrap(), b"MZ old program");
        assert_eq!(fs::read(copy.root.join(r"res\a.pak")).unwrap(), b"old a");
        assert!(!copy.root.join("heronite-rs.exe.old").exists() && !copy.root.join("portable.txt.old").exists());
        let mut names: Vec<_> = fs::read_dir(&copy.root).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["cache", "config", "data", "heronite-rs.exe", "notes.txt", "portable.txt", "res"]);

        // Folders the swap made are removed again.
        fs::remove_dir_all(copy.root.join("res")).unwrap();
        let (from, _) = new_files(&tmp.path().join("second"));
        fs::create_dir_all(from.join(r"res\deep")).unwrap();
        fs::write(from.join(r"res\deep\b.pak"), b"b").unwrap();
        fs::write(copy.root.join("portable.txt.old"), b"").unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(1).open(copy.root.join("portable.txt.old")).unwrap();
        let files = vec![PathBuf::from(r"res\deep\b.pak"), PROGRAM.into(), "portable.txt".into()];
        assert!(swap_in(&from, &files, &copy.root, &exe_name(&copy.exe)).is_err());
        drop(held);
        assert!(!copy.root.join("res").exists());
        assert_eq!(fs::read(copy.root.join(PROGRAM)).unwrap(), b"MZ old program");
    }

    /// The point of the `.old` dance: a program that runs can be replaced.
    #[test]
    fn a_running_program_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let copy = copy_in(tmp.path());
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
        fs::copy(system.join(r"System32\PING.EXE"), copy.root.join(PROGRAM)).unwrap();
        let mut running = std::process::Command::new(copy.root.join(PROGRAM))
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        // Overwriting or deleting it fails while it runs…
        assert!(fs::write(copy.root.join(PROGRAM), b"x").is_err());
        let (from, files) = new_files(tmp.path());
        let result = swap_in(&from, &files, &copy.root, &exe_name(&copy.exe));
        // …the sweep cannot remove it yet either, and keeps the list.
        remember_olds(&copy.old_list(), &[PathBuf::from("heronite-rs.exe.old")]);
        let _ = running.kill();
        let _ = running.wait();
        result.unwrap();
        assert_eq!(fs::read(copy.root.join(PROGRAM)).unwrap(), EXE);
        copy.sweep();
        assert!(!copy.root.join("heronite-rs.exe.old").exists());
    }

    /// Second test key, for the Windows fixtures; not the release key. Made
    /// with minisign 0.12 (`minisign -G -W`; the secret key was not kept):
    /// `portable-0.1.4.zip` holds `Heronite/` with a stand-in
    /// `heronite-rs.exe` (`MZ …`), `portable.txt` and `LICENSE`, stored, and
    /// was signed with `minisign -S -t "version:0.1.4
    /// file:Heronite-Portable-x86_64.zip"`.
    const TEST_KEY_2: &str = "RWRqLltp08OtbomHRa3MbmFMkaunSZk7qXsx7iv+DIdZVgNMMDkcnmkA";
    const ZIP_014: &[u8] = include_bytes!("../tests/fixtures/portable-0.1.4.zip");
    const ZIP_014_SIG: &str = include_str!("../tests/fixtures/portable-0.1.4.zip.minisig");

    fn release(base: &str, version: &str) -> Release {
        let zip = WindowsAsset::Portable.file_name();
        Release {
            version: version.into(),
            name: String::new(),
            notes: String::new(),
            page_url: String::new(),
            published_at: String::new(),
            appimage: None,
            signature: None,
            packages: Vec::new(),
            windows: [zip.clone(), format!("{zip}.minisig")]
                .into_iter()
                .map(|name| crate::release::Asset { url: format!("{base}/{name}"), name, size: 0 })
                .collect(),
        }
    }

    #[test]
    fn installs_a_signed_zip_and_refuses_a_tampered_one() {
        let tmp = tempfile::tempdir().unwrap();
        let copy = copy_in(tmp.path());
        let zip = WindowsAsset::Portable.file_name();
        let mut tampered = ZIP_014.to_vec();
        let at = tampered.windows(2).position(|w| w == b"MZ").unwrap() + 3;
        tampered[at] ^= 1;
        let base = serve(
            vec![
                (format!("/{zip}"), ZIP_014.to_vec()),
                (format!("/{zip}.minisig"), ZIP_014_SIG.into()),
                (format!("/bad/{zip}"), tampered),
                (format!("/bad/{zip}.minisig"), ZIP_014_SIG.into()),
            ],
            0,
        );
        let no = AtomicBool::new(false);
        let err = install(&release(&format!("{base}/bad"), "0.1.4"), &copy, TEST_KEY_2, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("does not match its signature"), "{err}");
        let err = install(&release(&base, "0.1.5"), &copy, TEST_KEY_2, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("another version"), "{err}");
        let err = install(&release(&base, "0.1.4"), &copy, crate::install::tests::TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("update key"), "{err}");
        assert_eq!(fs::read(copy.root.join(PROGRAM)).unwrap(), b"MZ old program");
        assert!(!copy.staging().exists());

        let mut last = Progress::default();
        install(&release(&base, "0.1.4"), &copy, TEST_KEY_2, &no, &mut |p| last = p).unwrap();
        assert_eq!(last.downloaded, ZIP_014.len() as u64);
        assert_eq!(fs::read(copy.root.join(PROGRAM)).unwrap(), b"MZ heronite 0.1.4 test program\n");
        assert_eq!(fs::read(copy.root.join("LICENSE")).unwrap(), b"test license\n");
        assert_eq!(fs::read(copy.root.join("heronite-rs.exe.old")).unwrap(), b"MZ old program");
        assert_eq!(fs::read_to_string(copy.root.join(r"data\mine.json")).unwrap(), "data");
        assert!(!copy.staging().exists());
        copy.sweep();
        assert!(!copy.root.join("heronite-rs.exe.old").exists() && !copy.root.join("portable.txt.old").exists());

        let mut unsigned = release(&base, "0.1.4");
        unsigned.windows.pop();
        assert!(install(&unsigned, &copy, TEST_KEY_2, &no, &mut |_| {}).unwrap_err().to_string().contains("not signed"));
    }

    #[test]
    fn a_folder_that_cannot_be_written_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("gone");
        assert_eq!(Portable::detect(missing.clone(), missing.join(PROGRAM)), Err(Unsupported::ReadOnly(missing)));
    }
}
