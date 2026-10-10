//! Lossless compression of the captures: PNG -> WebP with every pixel kept.
//!
//! * A converted file carries an XMP marker ([`marker`]) with the original
//!   name and size. That marker is the only record: nothing else is stored,
//!   a `.webp` is never re-encoded, and a `.png` still in the folder is
//!   simply work left to do. A PNG that would not get smaller gets a small
//!   `tEXt` marker instead ([`png::mark_kept`]) so it is not tried again.
//! * Each conversion is checked before the original goes: the new file is
//!   decoded and compared with the PNG's pixels, written to a temp name,
//!   fsynced, renamed, and only then is the PNG removed. Times (and so the
//!   order of the gallery and the timeline) are kept.
//! * A crash at any point leaves either the PNG alone (plus a temp file that
//!   the next scan sweeps) or both files; the next run sees the marked WebP
//!   next to its PNG, checks the pixels again and finishes the job.
//!
//! All functions block; run them on a background thread.

pub mod marker;
mod png;
mod refs;
mod scan;
mod webp;

pub use marker::Marker;
pub use refs::rewrite_art_refs;
pub use scan::{pending, usage, FolderUsage};

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

/// Files changed more recently than this are left for a later run: another
/// program (Steam, Game Bar, a screenshot tool) may still be writing them.
pub const SETTLE: Duration = Duration::from_secs(30);

/// Result of [`compress_file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `from` was replaced by `to`; `saved` = old size - new size in bytes.
    Converted { from: PathBuf, to: PathBuf, saved: i64 },
    /// Left as PNG (`reason` for the log): WebP cannot hold it exactly, the
    /// WebP was not smaller, or it is not finished being written.
    Kept { reason: String },
    /// The cancel flag was raised; the PNG is untouched.
    Cancelled,
}

/// Converts one PNG. `Err` means something unexpected (unreadable file,
/// pixel mismatch): the PNG is left as it was.
pub fn compress_file(path: &Path, cancel: &AtomicBool) -> Result<Outcome, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Ok(kept("not a regular file"));
    }
    if age(&meta) < SETTLE {
        return Ok(kept("changed moments ago"));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut target = path.with_extension("webp");
    if target.exists() {
        if let Some(done) = finish_interrupted(path, &target, &meta, &name)? {
            return Ok(done);
        }
        target = free_name(&target);
    }

    match png::probe(path).map_err(|e| e.to_string())? {
        png::Probe::Convertible => {}
        png::Probe::Kept => return Ok(kept("kept as PNG earlier")),
        png::Probe::Unsupported(reason) => return Ok(kept(reason)),
    }
    let img = png::decode(path)?;
    let simple = match webp::encode(img.width, img.height, &img.pixels, img.alpha, cancel) {
        Ok(bytes) => bytes,
        Err(webp::EncodeError::Cancelled) => return Ok(Outcome::Cancelled),
        Err(webp::EncodeError::Failed(e)) => return Err(e),
    };
    let marker = Marker { compression: marker::LOSSLESS_WEBP.into(), source_name: name, source_size: meta.len() };
    let xmp = marker::to_xmp(&marker, &webp::encoder_name(), &img.text);
    let extras = webp::Extras { icc: img.icc.as_deref(), exif: img.exif.as_deref(), xmp: &xmp };
    let bytes = webp::with_extras(&simple, img.width, img.height, img.alpha, &extras).ok_or("unexpected WebP layout")?;
    drop(simple);
    if bytes.len() as u64 >= meta.len() {
        png::mark_kept(path).map_err(|e| e.to_string())?;
        return Ok(kept("the WebP would not be smaller"));
    }
    if !same_pixels(&bytes, &img) {
        return Err("the WebP does not decode to the same pixels; the PNG was kept".into());
    }
    drop(img);
    replace(path, &target, &meta, &bytes)?;
    Ok(Outcome::Converted { from: path.to_path_buf(), to: target, saved: meta.len() as i64 - bytes.len() as i64 })
}

fn kept(reason: &str) -> Outcome {
    Outcome::Kept { reason: reason.to_owned() }
}

fn age(meta: &fs::Metadata) -> Duration {
    meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).unwrap_or_default()
}

/// `shot.webp` exists next to `shot.png`. When it is our conversion of this
/// very PNG (marker with the same name and size, same pixels), an earlier run
/// died before removing the PNG: remove it now.
fn finish_interrupted(png_path: &Path, webp_path: &Path, meta: &fs::Metadata, name: &str) -> Result<Option<Outcome>, String> {
    let Ok(Some(m)) = webp::read_marker(webp_path) else { return Ok(None) };
    if m.source_name != name || m.source_size != meta.len() {
        return Ok(None);
    }
    let bytes = fs::read(webp_path).map_err(|e| e.to_string())?;
    let img = png::decode(png_path)?;
    if !same_pixels(&bytes, &img) {
        return Ok(None);
    }
    fs::remove_file(png_path).map_err(|e| e.to_string())?;
    log::info!("finished an interrupted conversion of {}", png_path.display());
    let saved = meta.len() as i64 - bytes.len() as i64;
    Ok(Some(Outcome::Converted { from: png_path.to_path_buf(), to: webp_path.to_path_buf(), saved }))
}

/// `shot-2.webp`, `shot-3.webp`... when `shot.webp` is another file.
fn free_name(target: &Path) -> PathBuf {
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    (2..).map(|n| target.with_file_name(format!("{stem}-{n}.webp"))).find(|p| !p.exists()).expect("an unused name")
}

/// Decodes the WebP (the same decoder the gallery uses) and compares it with
/// the PNG's pixels byte for byte.
fn same_pixels(webp_bytes: &[u8], img: &png::Decoded) -> bool {
    let Ok(decoded) = image::load_from_memory_with_format(webp_bytes, image::ImageFormat::WebP) else { return false };
    if (decoded.width(), decoded.height()) != (img.width, img.height) {
        return false;
    }
    if img.alpha {
        decoded.to_rgba8().as_raw() == &img.pixels
    } else {
        decoded.to_rgb8().as_raw() == &img.pixels
    }
}

/// Writes the WebP under a temp name with the PNG's mode and times, makes it
/// durable, renames it into place, then removes the PNG.
fn replace(png_path: &Path, target: &Path, meta: &fs::Metadata, bytes: &[u8]) -> Result<(), String> {
    let tmp = heronite_data::temps::unique_tmp_path(target);
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.set_permissions(meta.permissions())?;
        f.set_times(times_of(meta))?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(format!("writing {}: {e}", tmp.display()));
    }
    // Something may have taken the name (or changed the PNG) meanwhile.
    let unchanged = fs::symlink_metadata(png_path).is_ok_and(|now| now.len() == meta.len() && now.modified().ok() == meta.modified().ok());
    if target.exists() || !unchanged {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{} changed while it was being compressed", png_path.display()));
    }
    if let Err(e) = fs::rename(&tmp, target) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("renaming to {}: {e}", target.display()));
    }
    if let Some(dir) = target.parent() {
        // The new name must be on disk before the PNG goes.
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    fs::remove_file(png_path).map_err(|e| format!("removing {}: {e}", png_path.display()))
}

fn times_of(meta: &fs::Metadata) -> fs::FileTimes {
    let mut times = fs::FileTimes::new();
    if let Ok(m) = meta.modified() {
        times = times.set_modified(m);
    }
    if let Ok(a) = meta.accessed() {
        times = times.set_accessed(a);
    }
    times
}

/// Puts back the times recorded in `meta` (a rewrite updated them).
fn restore_times(path: &Path, meta: &fs::Metadata) -> std::io::Result<()> {
    fs::File::options().write(true).open(path)?.set_times(times_of(meta))
}

#[cfg(unix)]
fn unix_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn unix_mode(_meta: &fs::Metadata) -> u32 {
    0o644
}

#[cfg(test)]
mod tests {
    use super::png::tests::{noisy_rgb, write_png};
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// An RGBA capture (opaque) with an sRGB chunk, an hour old.
    fn shot(path: &Path, w: u32, h: u32) -> Vec<u8> {
        let rgb = noisy_rgb(w, h);
        let rgba: Vec<u8> = rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        write_png(path, w, h, ::png::ColorType::Rgba, &rgba, &[(b"sRGB", &[0])]);
        set_old(path);
        rgb
    }

    fn set_old(path: &Path) {
        let old = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
    }

    #[test]
    fn converts_keeps_pixels_times_and_marks_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("shot.png");
        let rgb = shot(&p, 200, 120);
        let mtime = fs::metadata(&p).unwrap().modified().unwrap();
        let size = fs::metadata(&p).unwrap().len();

        let out = compress_file(&p, &AtomicBool::new(false)).unwrap();
        let Outcome::Converted { to, saved, .. } = out else { panic!("{out:?}") };
        assert_eq!(to, dir.path().join("shot.webp"));
        assert!(!p.exists());
        assert!(saved > 0);
        assert_eq!(fs::metadata(&to).unwrap().modified().unwrap(), mtime);
        assert_eq!(image::open(&to).unwrap().to_rgb8().into_raw(), rgb);
        let m = webp::read_marker(&to).unwrap().unwrap();
        assert_eq!((m.source_name.as_str(), m.source_size), ("shot.png", size));
        // Only our temp names would be left over; there are none.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn recent_files_wait_and_cancel_leaves_the_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("shot.png");
        shot(&p, 64, 64);
        fs::File::options().write(true).open(&p).unwrap().set_modified(SystemTime::now()).unwrap();
        assert!(matches!(compress_file(&p, &AtomicBool::new(false)).unwrap(), Outcome::Kept { .. }));
        set_old(&p);
        assert_eq!(compress_file(&p, &AtomicBool::new(true)).unwrap(), Outcome::Cancelled);
        assert!(p.exists());
        assert!(!dir.path().join("shot.webp").exists());
    }

    #[test]
    fn a_crash_after_the_rename_is_finished_by_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("shot.png");
        shot(&p, 80, 50);
        let backup = fs::read(&p).unwrap();
        let mtime = fs::metadata(&p).unwrap().modified().unwrap();
        compress_file(&p, &AtomicBool::new(false)).unwrap();
        // As if the PNG had not been removed.
        fs::write(&p, &backup).unwrap();
        fs::File::options().write(true).open(&p).unwrap().set_modified(mtime).unwrap();
        let out = compress_file(&p, &AtomicBool::new(false)).unwrap();
        assert!(matches!(out, Outcome::Converted { ref to, .. } if *to == dir.path().join("shot.webp")), "{out:?}");
        assert!(!p.exists());
        assert!(!dir.path().join("shot-2.webp").exists());
    }

    #[test]
    fn an_unrelated_webp_of_the_same_name_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("shot.png");
        shot(&p, 40, 40);
        fs::write(dir.path().join("shot.webp"), b"someone else's file").unwrap();
        let out = compress_file(&p, &AtomicBool::new(false)).unwrap();
        assert!(matches!(out, Outcome::Converted { ref to, .. } if *to == dir.path().join("shot-2.webp")), "{out:?}");
        assert_eq!(fs::read(dir.path().join("shot.webp")).unwrap(), b"someone else's file");
    }

    #[test]
    fn a_png_that_would_grow_is_marked_and_skipped_next_time() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("flat.png");
        // A 1x1 PNG is tiny; the WebP container with XMP is bigger.
        write_png(&p, 1, 1, ::png::ColorType::Rgb, &[1, 2, 3], &[]);
        set_old(&p);
        assert!(matches!(compress_file(&p, &AtomicBool::new(false)).unwrap(), Outcome::Kept { .. }));
        assert_eq!(png::probe(&p).unwrap(), png::Probe::Kept);
        assert_eq!(png::decode(&p).unwrap().pixels, [1, 2, 3]);
    }
}
