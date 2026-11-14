//! The first seconds of some trailers, kept on this PC so they start at
//! once while the rest streams. Only what the user allows (Settings →
//! Appearance → Game sections, 50 MB by default, 0 keeps nothing), only for
//! the games they visit, the least recently played going first.
//!
//! `<store-media>/videos/<video id>/` holds the init segment and the first
//! [`PREFIX_SECS`] of the picture (`v-*`) and of the sound (`a-*`), as the
//! store serves them, and `.used` (its time is when the trailer was last
//! played or kept). Playing writes `video.m3u8` / `audio.m3u8` there: the
//! store's playlists with the kept segments read from the files.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, Result};

use super::hls::{self, Media};
use super::StoreVideo;
use crate::http::Http;

/// Seconds of each trailer kept (about 3 MB at 480p).
pub const PREFIX_SECS: f64 = 15.;
/// The tallest picture asked for: a trailer fills the panel's column, and
/// 480p keeps the stream near 1.5 Mbit/s.
pub const MAX_HEIGHT: u32 = 480;
const USED: &str = ".used";
/// How recent a change to an unfinished trailer means it is being kept now.
const IN_PROGRESS: std::time::Duration = std::time::Duration::from_secs(120);

/// What the player opens for a trailer.
#[derive(Debug, Clone, PartialEq)]
pub struct Playable {
    /// A playlist address, or the path of a local playlist when some of it
    /// is kept on this PC.
    pub video: String,
    /// The sound's playlist, when it is apart from the picture.
    pub audio: Option<String>,
    /// Length in seconds.
    pub seconds: f64,
    /// The picture's size; 0 when the store does not say.
    pub width: u32,
    pub height: u32,
    /// Part of it is read from this PC (the playlists are files).
    pub local: bool,
}

fn videos_dir(root: &Path) -> PathBuf {
    root.join("videos")
}

fn folder(root: &Path, video: &StoreVideo) -> PathBuf {
    let id: String = video.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    videos_dir(root).join(if id.is_empty() { "_".into() } else { id })
}

/// The file a segment is kept in: `v-` or `a-` and its own name.
fn file_name(kind: char, uri: &str) -> String {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let name: String =
        path.rsplit('/').next().unwrap_or(path).chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).collect();
    format!("{kind}-{name}")
}

struct Playlists {
    variant: hls::Variant,
    video: Media,
    audio: Option<(String, Media)>,
}

fn text(http: &Http, url: &str) -> Result<String> {
    Ok(http.get(url).timeout_secs(15).max_bytes(1 << 20).send_ok()?.text())
}

fn playlists(http: &Http, video: &StoreVideo) -> Result<Playlists> {
    let master = hls::parse_master(&video.hls, &text(http, &video.hls)?);
    let variant = master.pick(MAX_HEIGHT).cloned().ok_or_else(|| anyhow!("no video in {}", video.hls))?;
    let media = hls::parse_media(&variant.uri, &text(http, &variant.uri)?);
    let audio = match master.audio {
        Some(uri) => {
            let m = hls::parse_media(&uri, &text(http, &uri)?);
            Some((uri, m))
        }
        None => None,
    };
    Ok(Playlists { variant, video: media, audio })
}

/// Gets a trailer ready to play: its playlists, with the segments kept on
/// this PC read from their files. Blocking (three small requests).
pub fn prepare(http: &Http, root: &Path, video: &StoreVideo) -> Result<Playable> {
    let p = playlists(http, video)?;
    let dir = folder(root, video);
    let kept = |kind: char| {
        let dir = dir.clone();
        move |uri: &str| {
            let file = dir.join(file_name(kind, uri));
            file.is_file().then(|| file.to_string_lossy().into_owned())
        }
    };
    let any_kept = p.video.map().is_some_and(|m| kept('v')(m).is_some());
    let seconds = p.video.seconds();
    let (width, height) = (p.variant.width, p.variant.height);
    if !any_kept {
        return Ok(Playable { video: p.variant.uri, audio: p.audio.map(|(u, _)| u), seconds, width, height, local: false });
    }
    let write = |name: &str, text: String| -> Result<String> {
        let path = dir.join(name);
        fs::write(&path, text)?;
        Ok(path.to_string_lossy().into_owned())
    };
    let video_path = write("video.m3u8", p.video.with_local(kept('v')))?;
    let audio_path = match &p.audio {
        Some((_, m)) => Some(write("audio.m3u8", m.with_local(kept('a')))?),
        None => None,
    };
    touch(&dir);
    Ok(Playable { video: video_path, audio: audio_path, seconds, width, height, local: true })
}

/// Keeps the first [`PREFIX_SECS`] of `video` (unless they are kept
/// already), then trims the kept trailers to `limit_bytes`, this one last.
/// Blocking; nothing when `limit_bytes` is 0.
pub fn keep(http: &Http, root: &Path, video: &StoreVideo, limit_bytes: u64) -> Result<()> {
    if limit_bytes == 0 {
        return Ok(());
    }
    let dir = folder(root, video);
    if dir.join(USED).is_file() {
        touch(&dir);
        return Ok(());
    }
    let p = playlists(http, video)?;
    fs::create_dir_all(&dir)?;
    let get = |kind: char, media: &Media| -> Result<()> {
        let mut uris: Vec<&str> = media.map().into_iter().collect();
        let mut secs = 0.;
        for (uri, s) in media.segments() {
            if secs >= PREFIX_SECS {
                break;
            }
            uris.push(uri);
            secs += s;
        }
        for uri in uris {
            let dest = dir.join(file_name(kind, uri));
            if dest.is_file() {
                continue;
            }
            let body = http.get(uri).timeout_secs(30).max_bytes(16 << 20).send_ok()?.body;
            let tmp = dest.with_extension("part");
            fs::write(&tmp, body)?;
            fs::rename(&tmp, &dest)?;
        }
        Ok(())
    };
    get('v', &p.video)?;
    if let Some((_, audio)) = &p.audio {
        get('a', audio)?;
    }
    fs::write(dir.join(USED), b"")?;
    trim(root, limit_bytes, Some(&dir));
    Ok(())
}

fn touch(dir: &Path) {
    let used = dir.join(USED);
    if let Ok(f) = fs::OpenOptions::new().write(true).open(&used) {
        let _ = f.set_modified(SystemTime::now());
    }
}

fn dir_size(dir: &Path) -> u64 {
    fs::read_dir(dir).map(|e| e.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()).unwrap_or(0)
}

/// Bytes the kept trailers take.
pub fn size(root: &Path) -> u64 {
    fs::read_dir(videos_dir(root)).map(|e| e.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

/// Deletes the least recently used trailers until they take at most
/// `limit_bytes` (`last` goes only if it alone is too big). A trailer
/// never finished (no `.used`) goes first, unless it changed in the last
/// [`IN_PROGRESS`]: another keep may still be writing it.
pub fn trim(root: &Path, limit_bytes: u64, last: Option<&Path>) {
    let Ok(entries) = fs::read_dir(videos_dir(root)) else { return };
    let mut dirs: Vec<(bool, SystemTime, u64, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let unfinished = !p.join(USED).is_file();
            let changed = fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            !(unfinished && changed.is_some_and(|age| age < IN_PROGRESS))
        })
        .map(|p| {
            let used = fs::metadata(p.join(USED)).and_then(|m| m.modified()).ok();
            (Some(p.as_path()) == last, used.unwrap_or(SystemTime::UNIX_EPOCH), dir_size(&p), p)
        })
        .collect();
    let mut total: u64 = dirs.iter().map(|d| d.2).sum();
    dirs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for (_, _, bytes, path) in dirs {
        if total <= limit_bytes {
            break;
        }
        if fs::remove_dir_all(&path).is_ok() {
            total = total.saturating_sub(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str) -> StoreVideo {
        StoreVideo { id: id.into(), name: String::new(), thumb: String::new(), hls: String::new() }
    }

    #[test]
    fn segments_keep_their_names() {
        assert_eq!(file_name('v', "https://v/x/dash_h264/chunk-stream2-00001.m4s?t=1"), "v-chunk-stream2-00001.m4s");
        assert_eq!(file_name('a', "https://v/x/init-stream4.m4s"), "a-init-stream4.m4s");
        assert_eq!(file_name('v', "https://v/../../etc/pa ss"), "v-pass");
    }

    #[test]
    fn trimming_drops_the_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut made = Vec::new();
        for (i, id) in ["old", "mid", "new"].iter().enumerate() {
            let d = folder(root, &video(id));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("v-1.m4s"), vec![0u8; 1000]).unwrap();
            fs::write(d.join(USED), b"").unwrap();
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64 * 100);
            fs::OpenOptions::new().write(true).open(d.join(USED)).unwrap().set_modified(t).unwrap();
            made.push(d);
        }
        assert_eq!(size(root), 3000);
        trim(root, 2500, Some(&made[0]));
        assert!(made[0].exists(), "the one just kept stays");
        assert!(!made[1].exists());
        assert!(made[2].exists());
        trim(root, 0, None);
        assert_eq!(size(root), 0);
    }
}
