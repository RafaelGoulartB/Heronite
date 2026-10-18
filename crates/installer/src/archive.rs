//! The `extract` step, through the system's archivers: `tar` (any
//! compression it knows), `unzip`, `7z` / `7za` / `7zz`, `unrar`,
//! `innoextract` (Inno Setup and GOG installers) and `bsdtar` as the catch
//! all. The archive goes to a temporary folder inside the destination first;
//! a single top folder is then merged into the destination (unless
//! `nomerge`), and an archive that held one more archive is opened too.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use heronite_i18n::tr;

use crate::exec::{run, run_ok, which_any, Io};
use crate::fsops::move_contents;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Tar,
    Zip,
    SevenZip,
    Rar,
    Gzip,
    /// Inno Setup installer (GOG's offline installers).
    Inno,
    /// A Windows program that may be an Inno Setup or self-extracting archive.
    Exe,
    Deb,
    /// Not an archive: copied as it is.
    AppImage,
}

const SEVEN_ZIP: &[&str] = &["7z", "7za", "7zz"];

impl Format {
    /// The programs that can open it: one of each group is needed.
    pub fn tools(self) -> &'static [&'static [&'static str]] {
        match self {
            Format::Tar => &[&["tar", "bsdtar"]],
            Format::Zip => &[&["unzip", "7z", "7za", "7zz", "bsdtar"]],
            Format::SevenZip => &[&["7z", "7za", "7zz", "bsdtar"]],
            Format::Rar => &[&["unrar", "7z", "7za", "7zz", "bsdtar"]],
            Format::Gzip => &[&["gzip", "7z", "7za", "7zz"]],
            Format::Inno => &[&["innoextract"]],
            Format::Exe => &[&["innoextract", "7z", "7za", "7zz"]],
            Format::Deb => &[&["bsdtar", "ar"], &["tar", "bsdtar"]],
            Format::AppImage => &[],
        }
    }
}

/// The format a script asked for (`format:`), in the names scripts use.
pub fn named(format: &str) -> Option<Format> {
    Some(match format.trim().to_ascii_lowercase().as_str() {
        "tar" | "tgz" | "txz" | "tbz2" | "bz2" | "tzst" | "tar.gz" | "tar.xz" | "tar.bz2" => Format::Tar,
        "zip" => Format::Zip,
        "7z" | "7zip" => Format::SevenZip,
        "rar" => Format::Rar,
        "gzip" | "gz" => Format::Gzip,
        "gog" | "innoextract" | "inno" => Format::Inno,
        "exe" => Format::Exe,
        "deb" => Format::Deb,
        "appimage" => Format::AppImage,
        _ => return None,
    })
}

/// The format from the file name, else from its first bytes.
pub fn guess(path: &Path) -> Format {
    if let Some(f) = by_name(&path.to_string_lossy()) {
        return f;
    }
    let mut head = [0u8; 264];
    let n = fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"PK\x03\x04") {
        Format::Zip
    } else if head.starts_with(b"Rar!") {
        Format::Rar
    } else if head.starts_with(b"MZ") {
        Format::Exe
    } else if head.starts_with(b"7z\xBC\xAF") {
        Format::SevenZip
    } else if head.starts_with(b"!<arch>") {
        Format::Deb
    } else if head.len() > 262 && &head[257..262] == b"ustar"
        || head.starts_with(&[0x1f, 0x8b])
        || head.starts_with(b"BZh")
        || head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z'])
        || head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
    {
        Format::Tar
    } else {
        Format::SevenZip
    }
}

/// The format from the file name alone.
pub fn by_name(name: &str) -> Option<Format> {
    let n = name.to_ascii_lowercase();
    let ends = |exts: &[&str]| exts.iter().any(|e| n.ends_with(e));
    Some(if ends(&[".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.lzma", ".tar.bz2", ".tbz2", ".tbz", ".tar.zst", ".tzst"]) {
        Format::Tar
    } else if ends(&[".zip"]) {
        Format::Zip
    } else if ends(&[".7z"]) {
        Format::SevenZip
    } else if ends(&[".rar"]) {
        Format::Rar
    } else if ends(&[".gz"]) {
        Format::Gzip
    } else if ends(&[".exe"]) {
        Format::Exe
    } else if ends(&[".deb"]) {
        Format::Deb
    } else if ends(&[".appimage"]) {
        Format::AppImage
    } else {
        return None;
    })
}

/// Extracts `archive` into `dest`.
pub fn extract(archive: &Path, dest: &Path, format: Option<Format>, merge_single: bool, io: &Io) -> Result<(), String> {
    let format = format.unwrap_or_else(|| guess(archive));
    if !archive.is_file() {
        return Err(tr!("{path} does not exist", path = archive.display()));
    }
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let tmp = dest.join(format!(".heronite-extract-{}", std::process::id() ^ nanos()));
    fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let result = (|| {
        unpack(archive, &tmp, format, io)?;
        place(&tmp, dest, merge_single, io)
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn nanos() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0)
}

/// Moves what was extracted into `tmp` to `dest`.
fn place(tmp: &Path, dest: &Path, merge_single: bool, io: &Io) -> Result<(), String> {
    let entries: Vec<PathBuf> = fs::read_dir(tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    let root = match entries.as_slice() {
        [only] if merge_single => only.clone(),
        _ => tmp.to_path_buf(),
    };
    if root.is_file() {
        // An archive inside the archive (a zip in a tar.gz): open it as well.
        let inner = by_name(&root.to_string_lossy()).or_else(|| match guess(&root) {
            Format::SevenZip | Format::Exe => None,
            f => Some(f),
        });
        if let Some(f) = inner.filter(|f| !matches!(f, Format::Exe | Format::AppImage)) {
            return extract(&root, dest, Some(f), merge_single, io);
        }
        let target = dest.join(root.file_name().unwrap_or_default());
        if target.is_dir() {
            fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        return fs::rename(&root, &target).map_err(|e| format!("{}: {e}", target.display()));
    }
    move_contents(&root, dest)
}

fn tool(names: &[&str]) -> Result<String, String> {
    which_any(names).ok_or_else(|| tr!("{tool} is needed to extract this archive; install it with your package manager", tool = names[0]))
}

fn unpack(archive: &Path, out: &Path, format: Format, io: &Io) -> Result<(), String> {
    match format {
        Format::AppImage => {
            let target = out.join(archive.file_name().unwrap_or_default());
            fs::copy(archive, &target).map_err(|e| format!("{}: {e}", target.display()))?;
            crate::fsops::make_executable(&target)
        }
        Format::Tar => {
            let bin = tool(&["tar", "bsdtar"])?;
            run_ok(Command::new(bin).arg("-xf").arg(archive).arg("-C").arg(out), io, None)
        }
        Format::Zip => {
            if let Some(unzip) = which_any(&["unzip"]) {
                // 1: warnings only (odd timestamps, names).
                return run_ok(Command::new(unzip).args(["-o", "-qq"]).arg(archive).arg("-d").arg(out), io, Some(1));
            }
            seven_zip_or_bsdtar(archive, out, io)
        }
        Format::SevenZip | Format::Gzip => {
            if format == Format::Gzip && which_any(SEVEN_ZIP).is_none() {
                let gzip = tool(&["gzip"])?;
                let name = archive.file_stem().unwrap_or_default();
                let target = out.join(name);
                let mut cmd = Command::new("sh");
                cmd.args(["-c", "exec \"$0\" -dc \"$1\" > \"$2\"", &gzip]).arg(archive).arg(&target);
                return run_ok(&mut cmd, io, None);
            }
            seven_zip_or_bsdtar(archive, out, io)
        }
        Format::Rar => {
            if let Some(unrar) = which_any(&["unrar"]) {
                let mut dir = out.as_os_str().to_os_string();
                dir.push("/");
                return run_ok(Command::new(unrar).args(["x", "-o+", "-y", "-idq"]).arg(archive).arg(dir), io, None);
            }
            seven_zip_or_bsdtar(archive, out, io)
        }
        Format::Inno => innoextract(archive, out, io),
        Format::Exe => {
            if let Some(inno) = which_any(&["innoextract"]) {
                let code = run(Command::new(inno).arg("-i").arg(archive), &Io { cancel: io.cancel, log: &|_| {} })?;
                if code == 0 {
                    return innoextract(archive, out, io);
                }
            }
            let bin = tool(SEVEN_ZIP).map_err(|_| tr!("This program is not an archive or an installer that can be extracted"))?;
            run_ok(Command::new(bin).args(["x", "-y", "-aoa"]).arg(format!("-o{}", out.display())).arg(archive), io, None)
        }
        Format::Deb => {
            let parts = out.join(".deb-parts");
            fs::create_dir_all(&parts).map_err(|e| e.to_string())?;
            if let Some(bsdtar) = which_any(&["bsdtar"]) {
                run_ok(Command::new(bsdtar).arg("-xf").arg(archive).arg("-C").arg(&parts), io, None)?;
            } else {
                let ar = tool(&["ar"])?;
                run_ok(Command::new(ar).arg("x").arg(archive).current_dir(&parts), io, None)?;
            }
            let data = fs::read_dir(&parts)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("data.tar")))
                .ok_or_else(|| tr!("This package has no data to extract"))?;
            let bin = tool(&["tar", "bsdtar"])?;
            run_ok(Command::new(bin).arg("-xf").arg(&data).arg("-C").arg(out), io, None)?;
            fs::remove_dir_all(&parts).map_err(|e| e.to_string())
        }
    }
}

fn seven_zip_or_bsdtar(archive: &Path, out: &Path, io: &Io) -> Result<(), String> {
    if let Some(bin) = which_any(SEVEN_ZIP) {
        return run_ok(Command::new(bin).args(["x", "-y", "-aoa"]).arg(format!("-o{}", out.display())).arg(archive), io, None);
    }
    let bin = tool(&["bsdtar", "7z"])?;
    run_ok(Command::new(bin).arg("-xf").arg(archive).arg("-C").arg(out), io, None)
}

/// GOG installers keep the game in `app/` (and some files in
/// `__support/app`); those become the top of the extracted folder.
fn innoextract(archive: &Path, out: &Path, io: &Io) -> Result<(), String> {
    let bin = tool(&["innoextract"])?;
    run_ok(Command::new(bin).args(["-e", "-s", "-g", "-d"]).arg(out).arg(archive), io, None)?;
    let app = out.join("app");
    if app.is_dir() {
        let support = out.join("__support/app");
        if support.is_dir() {
            move_contents(&support, &app)?;
        }
        for entry in fs::read_dir(out).map_err(|e| e.to_string())?.flatten() {
            let p = entry.path();
            if p != app {
                let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
            }
        }
        move_contents(&app, out)?;
        let _ = fs::remove_dir_all(&app);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io() -> (Box<dyn Fn() -> bool>, Box<dyn Fn(&str)>) {
        (Box::new(|| false), Box::new(|_| {}))
    }

    #[test]
    fn formats_by_name_and_content() {
        assert_eq!(by_name("a.TAR.GZ"), Some(Format::Tar));
        assert_eq!(by_name("setup_game.exe"), Some(Format::Exe));
        assert_eq!(by_name("x.AppImage"), Some(Format::AppImage));
        assert_eq!(by_name("x.bin"), None);
        assert_eq!(named("tgz"), Some(Format::Tar));
        assert_eq!(named("gog"), Some(Format::Inno));
        assert_eq!(named("auto"), None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("data.bin");
        fs::write(&f, b"PK\x03\x04rest").unwrap();
        assert_eq!(guess(&f), Format::Zip);
        fs::write(&f, b"\x1f\x8bxx").unwrap();
        assert_eq!(guess(&f), Format::Tar);
    }

    #[test]
    fn a_single_top_folder_is_merged() {
        if which_any(&["tar"]).is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir_all(d.join("src/Game-1.0/data")).unwrap();
        fs::write(d.join("src/Game-1.0/start.sh"), "#!/bin/sh").unwrap();
        fs::write(d.join("src/Game-1.0/data/a"), "a").unwrap();
        let archive = d.join("game.tar.gz");
        assert!(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(d.join("src")).arg("Game-1.0").status().unwrap().success());
        let (cancel, log) = io();
        let io = Io { cancel: &*cancel, log: &*log };
        extract(&archive, &d.join("out"), None, true, &io).unwrap();
        assert!(d.join("out/start.sh").exists() && d.join("out/data/a").exists());
        extract(&archive, &d.join("kept"), None, false, &io).unwrap();
        assert!(d.join("kept/Game-1.0/start.sh").exists());
        let leftovers: Vec<_> =
            fs::read_dir(d.join("out")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with('.')).collect();
        assert!(leftovers.is_empty(), "the temporary folder is removed");
    }

    #[test]
    fn a_nested_archive_is_opened() {
        if which_any(&["tar"]).is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir_all(d.join("inner")).unwrap();
        fs::write(d.join("inner/game.x86_64"), "elf").unwrap();
        assert!(Command::new("tar")
            .arg("-cf")
            .arg(d.join("inner.tar"))
            .arg("-C")
            .arg(d.join("inner"))
            .arg(".")
            .status()
            .unwrap()
            .success());
        assert!(Command::new("tar").arg("-czf").arg(d.join("outer.tgz")).arg("-C").arg(d).arg("inner.tar").status().unwrap().success());
        let (cancel, log) = io();
        extract(&d.join("outer.tgz"), &d.join("out"), None, true, &Io { cancel: &*cancel, log: &*log }).unwrap();
        assert!(d.join("out/game.x86_64").exists());
    }
}
