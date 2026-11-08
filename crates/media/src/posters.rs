//! Posters of replays: one frame of the video as a preview, kept in the
//! screenshot preview cache (the same 320x180 JPEGs and LRU budget) with the
//! video's length in the sidecar (`"seconds"`).
//!
//! Made to stay cheap: ffprobe reads the length from the container, then
//! ffmpeg seeks to the keyframe before two thirds of it and decodes that one
//! frame only (`-skip_frame nokey`), already scaled and cropped, as raw BGRA
//! on its stdout (about 0.1 s for a 1440p replay). A black frame (a fade, a
//! loading screen) is passed over for one at another point. Nothing is
//! written but the cached JPEG.
//!
//! The key holds the file's size: trimming a replay (`heronite_overlay`'s
//! `trim`) keeps its name and date, so a poster drawn before would otherwise
//! stay with the old length.

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use heronite_data::proc::Run;
use heronite_images::Decoded;
use sha2::{Digest, Sha256};

use crate::thumbnails::{ThumbError, THUMB_HEIGHT, THUMB_QUALITY, THUMB_WIDTH};

/// Containers a replay is saved in (gpu-screen-recorder writes `.mp4`).
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "mov"];
/// Where a frame is taken, as shares of the length, in the order tried: a
/// replay ends on the moment it was saved for, its start is often a calm
/// second.
const FRAME_AT: [f64; 5] = [2. / 3., 1. / 2., 5. / 6., 1. / 3., 0.];
/// Mean of the colour channels (0..=255) under which a frame counts as black.
const BLACK_BELOW: f64 = 6.;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);

/// A replay's preview.
pub struct Poster {
    pub image: Decoded,
    /// Length in seconds; `None` when ffprobe could not read it.
    pub seconds: Option<f64>,
}

/// Whether `path` names a video (by its extension).
pub fn is_video(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

/// Whether ffmpeg and ffprobe are on the `PATH` (looked up once).
pub fn available() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let exe = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_owned() };
        ["ffmpeg", "ffprobe"]
            .iter()
            .all(|n| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe(n)).is_file())))
    })
}

/// Cache file stem of the poster of `source`, `len` bytes long.
pub(crate) fn key(source: &Path, len: u64) -> String {
    let mut h = Sha256::new();
    h.update(source.to_string_lossy().as_bytes());
    h.update(format!("\0poster\0{len}\0{THUMB_WIDTH}x{THUMB_HEIGHT}q{THUMB_QUALITY}").as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The length stored in a sidecar.
pub(crate) fn seconds_in(meta: &serde_json::Map<String, serde_json::Value>) -> Option<f64> {
    meta.get("seconds").and_then(serde_json::Value::as_f64).filter(|s| s.is_finite() && *s > 0.)
}

/// Draws the poster of `source`: the raster and the sidecar's fields.
pub(crate) fn render(source: &Path) -> Result<(Decoded, serde_json::Map<String, serde_json::Value>), ThumbError> {
    if !available() {
        return Err(ThumbError::Image("ffmpeg is not installed".into()));
    }
    let seconds = duration(source);
    let shares: &[f64] = if seconds.is_some() { &FRAME_AT } else { &[0.] };
    let mut first: Option<Vec<u8>> = None;
    let mut picked = None;
    for share in shares {
        let bgra = frame(source, seconds.unwrap_or(0.) * share)?;
        if brightness(&bgra) >= BLACK_BELOW {
            picked = Some(bgra);
            break;
        }
        first.get_or_insert(bgra);
    }
    let Some(bgra) = picked.or(first) else { return Err(ThumbError::Image("no frame".into())) };
    let mut meta = serde_json::Map::new();
    if let Some(s) = seconds {
        meta.insert("seconds".into(), s.into());
    }
    Ok((Decoded { width: THUMB_WIDTH, height: THUMB_HEIGHT, bgra }, meta))
}

/// The keyframe at or before `at` seconds, as preview-sized BGRA.
fn frame(source: &Path, at: f64) -> Result<Vec<u8>, ThumbError> {
    let (w, h) = (THUMB_WIDTH, THUMB_HEIGHT);
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-threads", "1", "-skip_frame", "nokey", "-noaccurate_seek", "-ss", &format!("{at:.3}"), "-i"])
        .arg(source)
        .args(["-frames:v", "1", "-an", "-sn", "-dn"])
        .args(["-vf", &format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")])
        .args(["-pix_fmt", "bgra", "-f", "rawvideo", "-"]);
    let out = Run::new().timeout(FRAME_TIMEOUT).run(&mut cmd)?;
    let want = (w * h * 4) as usize;
    if !out.success() || out.stdout.len() != want {
        let why = out.stderr_text();
        let why = why.trim();
        return Err(ThumbError::Image(if why.is_empty() { format!("ffmpeg gave {} bytes", out.stdout.len()) } else { why.to_owned() }));
    }
    Ok(out.stdout)
}

/// Mean of the colour channels of a BGRA raster, 0..=255.
fn brightness(bgra: &[u8]) -> f64 {
    let px = bgra.as_chunks::<4>().0;
    if px.is_empty() {
        return 0.;
    }
    px.iter().map(|p| (p[0] as u64 + p[1] as u64 + p[2] as u64) as f64 / 3.).sum::<f64>() / px.len() as f64
}

/// Length in seconds (ffprobe), when it reads one.
fn duration(path: &Path) -> Option<f64> {
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"]).arg(path);
    let out = Run::new().timeout(PROBE_TIMEOUT).run(&mut cmd).ok()?;
    out.stdout_text().trim().parse::<f64>().ok().filter(|s| s.is_finite() && *s > 0.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThumbnailCache;
    use std::path::PathBuf;

    /// A 3 s test pattern, or `None` without ffmpeg (the test is skipped).
    fn sample(dir: &Path) -> Option<PathBuf> {
        if !available() {
            eprintln!("ffmpeg not found: skipped");
            return None;
        }
        let path = dir.join("Replay_2026-11-06_01-57-00.mp4");
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-v", "error", "-nostdin", "-f", "lavfi", "-i", "testsrc=duration=3:size=640x360:rate=30", "-g", "30"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&path);
        Run::new().timeout(Duration::from_secs(60)).run(&mut cmd).ok().filter(|o| o.success())?;
        Some(path)
    }

    #[test]
    fn videos_are_told_by_their_extension() {
        assert!(is_video(Path::new("/c/G/replays/a.mp4")));
        assert!(is_video(Path::new("/c/G/replays/a.MKV")));
        assert!(!is_video(Path::new("/c/G/a.webp")));
        assert!(!is_video(Path::new("/c/G/mp4")));
    }

    #[test]
    fn the_key_follows_the_size() {
        let p = Path::new("/c/G/replays/a.mp4");
        assert_ne!(key(p, 10), key(p, 11), "a trimmed replay gets a new poster");
        assert_ne!(key(p, 10), crate::thumbnails::cache_key(p));
    }

    #[test]
    fn black_frames_are_told_apart() {
        assert!(brightness(&[0, 0, 0, 255].repeat(100)) < BLACK_BELOW);
        assert!(brightness(&[40, 10, 90, 255].repeat(100)) >= BLACK_BELOW, "a dark red scene is not black");
        assert_eq!(brightness(&[]), 0.);
    }

    #[test]
    fn a_poster_is_drawn_once_with_its_length() {
        let dir = tempfile::tempdir().unwrap();
        let Some(video) = sample(dir.path()) else { return };
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.);
        let first = cache.poster(&video).unwrap();
        assert_eq!((first.image.width, first.image.height), (THUMB_WIDTH, THUMB_HEIGHT));
        assert_eq!(first.image.bgra.len(), (THUMB_WIDTH * THUMB_HEIGHT * 4) as usize);
        let seconds = first.seconds.expect("ffprobe reads the length");
        assert!((seconds - 3.).abs() < 0.2, "{seconds}");

        let cached = cache.poster(&video).unwrap();
        assert_eq!(cached.seconds, Some(seconds), "the length comes back from the sidecar");
        assert_eq!(cache.info().item_count, 1);
    }

    #[test]
    fn a_broken_video_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        if !available() {
            return;
        }
        let video = dir.path().join("broken.mp4");
        std::fs::write(&video, b"not a video").unwrap();
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.);
        assert!(cache.poster(&video).is_err());
        assert!(cache.poster(&dir.path().join("gone.mp4")).is_err());
    }
}
