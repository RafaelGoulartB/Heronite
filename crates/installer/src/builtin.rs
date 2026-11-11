//! Archives opened in process, without a program on the PATH: zip (stored,
//! deflate), 7z (LZMA, LZMA2, BZip2, PPMd, BCJ filters, no password) and a
//! lone `.gz`. A stock Windows has no 7-Zip, `unzip` or `bsdtar`; its own
//! `tar.exe` (libarchive) opens tar and RAR, see [`windows_tar`].
//!
//! Names are checked before anything is written: an absolute path, a drive,
//! a `..` or a stream name (`a:b`) is refused, and links are skipped, so an
//! archive can never write outside the folder it is extracted into.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use heronite_i18n::tr;

use crate::engine::CANCELLED;
use crate::exec::Io;

/// Why an in-process unpack stopped.
#[derive(Debug)]
pub enum Failure {
    /// The user cancelled: nothing else is tried.
    Cancelled,
    /// The archive uses something this code does not read (a method, a
    /// password, a damaged header). An archiver on the PATH may still open it.
    Unreadable(String),
}

impl Failure {
    pub fn into_message(self) -> String {
        match self {
            Failure::Cancelled => CANCELLED.into(),
            Failure::Unreadable(m) => m,
        }
    }
}

/// `name` from an archive as a path below the extraction folder, or `None`
/// when it would land outside it (or names nothing).
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    if name.starts_with(['/', '\\']) {
        return None;
    }
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            // A drive (`C:`) or an NTFS stream (`file:stream`).
            p if p.contains(':') => return None,
            p => out.push(p),
        }
    }
    let plain = out.components().all(|c| matches!(c, Component::Normal(_)));
    (plain && !out.as_os_str().is_empty()).then_some(out)
}

/// Copies `from` into a new file at `to`, checking for a cancel every MiB.
fn write_file(from: &mut dyn Read, to: &Path, io: &Io) -> Result<(), Failure> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| Failure::Unreadable(format!("{}: {e}", parent.display())))?;
    }
    let mut out = fs::File::create(to).map_err(|e| Failure::Unreadable(format!("{}: {e}", to.display())))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if (io.cancel)() {
            return Err(Failure::Cancelled);
        }
        let n = match from.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Failure::Unreadable(format!("{}: {e}", to.display()))),
        };
        out.write_all(&buf[..n]).map_err(|e| Failure::Unreadable(format!("{}: {e}", to.display())))?;
    }
}

/// Unpacks a zip into `out`.
pub fn unzip(archive: &Path, out: &Path, io: &Io) -> Result<(), Failure> {
    let unreadable = |e: &dyn std::fmt::Display| Failure::Unreadable(format!("{}: {e}", archive.display()));
    let file = fs::File::open(archive).map_err(|e| unreadable(&e))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file)).map_err(|e| unreadable(&e))?;
    for i in 0..zip.len() {
        if (io.cancel)() {
            return Err(Failure::Cancelled);
        }
        let mut entry = zip.by_index(i).map_err(|e| unreadable(&e))?;
        let name = entry.name().map_err(|e| unreadable(&e))?.into_owned();
        if entry.is_symlink() {
            (io.log)(&format!("skipped the link {name}"));
            continue;
        }
        let Some(rel) = safe_rel(&name) else {
            (io.log)(&format!("skipped {name}: it points outside the folder"));
            continue;
        };
        let target = out.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| Failure::Unreadable(format!("{}: {e}", target.display())))?;
        } else {
            write_file(&mut entry, &target, io)?;
        }
    }
    Ok(())
}

/// Unpacks a 7z into `out`.
pub fn un7z(archive: &Path, out: &Path, io: &Io) -> Result<(), Failure> {
    let unreadable = |e: &dyn std::fmt::Display| Failure::Unreadable(format!("{}: {e}", archive.display()));
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty()).map_err(|e| unreadable(&e))?;
    let mut stopped: Option<Failure> = None;
    let result = reader.for_each_entries(|entry, data| {
        let name = entry.name().to_string();
        let Some(rel) = safe_rel(&name) else {
            (io.log)(&format!("skipped {name}: it points outside the folder"));
            return Ok(true);
        };
        if entry.is_anti_item() {
            return Ok(true);
        }
        let target = out.join(rel);
        let written = if entry.is_directory() {
            fs::create_dir_all(&target).map_err(|e| Failure::Unreadable(format!("{}: {e}", target.display())))
        } else {
            write_file(data, &target, io)
        };
        match written {
            Ok(()) => Ok(true),
            Err(f) => {
                stopped = Some(f);
                Err(sevenz_rust2::Error::Other("stopped".into()))
            }
        }
    });
    if let Some(f) = stopped {
        return Err(f);
    }
    result.map_err(|e| unreadable(&e))
}

/// Unpacks a lone `.gz` (not a `.tar.gz`) into `out/<name without .gz>`.
pub fn gunzip(archive: &Path, out: &Path, io: &Io) -> Result<(), Failure> {
    let file = fs::File::open(archive).map_err(|e| Failure::Unreadable(format!("{}: {e}", archive.display())))?;
    let name = archive.file_stem().unwrap_or_default();
    let mut decoder = flate2::read::GzDecoder::new(io::BufReader::new(file));
    write_file(&mut decoder, &out.join(name), io)
}

/// Windows' own `tar.exe` (libarchive's bsdtar, in every Windows 10 and
/// later). It opens tar (gzip, bzip2; xz and zstd on newer builds), zip,
/// 7z and RAR (4 and 5). Named by its full path: a `tar` on the PATH may be
/// Git's GNU tar, which reads `C:` as a host name.
pub fn windows_tar() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let tar = root.join("System32").join("tar.exe");
    tar.is_file().then_some(tar)
}

/// The message when nothing could open an archive.
pub fn no_tool(format: &str) -> String {
    tr!("This {format} archive could not be opened. Install 7-Zip, or unpack the archive yourself and add the folder instead.", format)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io() -> (Box<dyn Fn() -> bool>, Box<dyn Fn(&str)>) {
        (Box::new(|| false), Box::new(|_| {}))
    }

    #[test]
    fn names_outside_the_folder_are_refused() {
        assert_eq!(safe_rel("Data/a.esp"), Some(PathBuf::from("Data").join("a.esp")));
        assert_eq!(safe_rel(r"Data\Textures\b.dds"), Some(PathBuf::from("Data").join("Textures").join("b.dds")));
        assert_eq!(safe_rel("./x/./y"), Some(PathBuf::from("x").join("y")));
        assert_eq!(safe_rel("../evil.dll"), None);
        assert_eq!(safe_rel("a/../../evil.dll"), None);
        assert_eq!(safe_rel("/etc/passwd"), None);
        assert_eq!(safe_rel(r"\\server\share\x"), None);
        assert_eq!(safe_rel(r"C:\Windows\x.dll"), None);
        assert_eq!(safe_rel("file.txt:stream"), None);
        assert_eq!(safe_rel(""), None);
        assert_eq!(safe_rel("/"), None);
    }

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        use zip::write::SimpleFileOptions;
        let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, data) in files {
            if name.ends_with('/') {
                w.add_directory(*name, SimpleFileOptions::default()).unwrap();
            } else {
                w.start_file(*name, SimpleFileOptions::default()).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
    }

    #[test]
    fn a_zip_opens_without_any_program() {
        let t = tempfile::tempdir().unwrap();
        let zip = t.path().join("mod.zip");
        let big = vec![7u8; 3 << 20];
        make_zip(&zip, &[("Data/", b""), ("Data/a.esp", b"plugin"), ("Data/Textures/big.dds", &big), ("../evil.txt", b"no")]);
        let out = t.path().join("out");
        fs::create_dir_all(&out).unwrap();
        let (cancel, log) = io();
        unzip(&zip, &out, &Io { cancel: &*cancel, log: &*log }).unwrap();
        assert_eq!(fs::read(out.join("Data/a.esp")).unwrap(), b"plugin");
        assert_eq!(fs::read(out.join("Data/Textures/big.dds")).unwrap(), big);
        assert!(!t.path().join("evil.txt").exists(), "nothing lands outside the folder");
    }

    #[test]
    fn a_cancel_stops_an_unpack() {
        let t = tempfile::tempdir().unwrap();
        let zip = t.path().join("mod.zip");
        make_zip(&zip, &[("a.txt", b"a")]);
        let cancel = || true;
        let log = |_: &str| {};
        assert!(matches!(unzip(&zip, t.path(), &Io { cancel: &cancel, log: &log }), Err(Failure::Cancelled)));
    }

    #[test]
    fn a_7z_opens_without_any_program() {
        let t = tempfile::tempdir().unwrap();
        let archive = t.path().join("mod.7z");
        let mut w = sevenz_rust2::ArchiveWriter::new(fs::File::create(&archive).unwrap()).unwrap();
        let text = "x".repeat(100_000);
        w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_directory("Data"), None::<&[u8]>).unwrap();
        w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file("Data/a.esp"), Some(&b"plugin"[..])).unwrap();
        w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file("Data/readme.txt"), Some(text.as_bytes())).unwrap();
        w.finish().unwrap();
        let out = t.path().join("out");
        let (cancel, log) = io();
        un7z(&archive, &out, &Io { cancel: &*cancel, log: &*log }).unwrap();
        assert_eq!(fs::read(out.join("Data/a.esp")).unwrap(), b"plugin");
        assert_eq!(fs::read_to_string(out.join("Data/readme.txt")).unwrap(), text);
    }

    #[test]
    fn a_damaged_archive_is_unreadable_not_a_panic() {
        let t = tempfile::tempdir().unwrap();
        let bad = t.path().join("bad.7z");
        fs::write(&bad, b"7z\xBC\xAF\x27\x1C broken").unwrap();
        let (cancel, log) = io();
        let io = Io { cancel: &*cancel, log: &*log };
        assert!(matches!(un7z(&bad, t.path(), &io), Err(Failure::Unreadable(_))));
        fs::write(t.path().join("bad.zip"), b"PK\x03\x04 broken").unwrap();
        assert!(matches!(unzip(&t.path().join("bad.zip"), t.path(), &io), Err(Failure::Unreadable(_))));
    }

    #[test]
    fn a_lone_gz_opens_without_any_program() {
        let t = tempfile::tempdir().unwrap();
        let gz = t.path().join("game.bin.gz");
        let mut enc = flate2::write::GzEncoder::new(fs::File::create(&gz).unwrap(), flate2::Compression::default());
        enc.write_all(b"payload").unwrap();
        enc.finish().unwrap();
        let (cancel, log) = io();
        gunzip(&gz, t.path(), &Io { cancel: &*cancel, log: &*log }).unwrap();
        assert_eq!(fs::read(t.path().join("game.bin")).unwrap(), b"payload");
    }
}
