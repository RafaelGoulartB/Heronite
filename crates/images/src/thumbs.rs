//! Persistent resized-image disk cache.
//!
//! One file per (source identity, variant, target size): 16-byte header +
//! BGRA pixels, either raw or as one LZ4 block (whichever is smaller; LZ4 is
//! only kept if it saves >=10%). Decoding is a single `read` plus a memcpy-speed
//! LZ4 decompress straight into the final pixel `Vec` (no intermediate image
//! objects, no channel swizzle: pixels are stored in GPUI's BGRA order).

use crate::error::Result;
use crate::fetch::{hex, write_atomic};
use crate::{Decoded, ImageRef, Opts, Variant};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MAGIC: &[u8; 4] = b"HIT1";
const HEADER: usize = 16;
const CODEC_RAW: u8 = 0;
const CODEC_LZ4: u8 = 1;
/// Bump to invalidate all cached thumbs when the pipeline output changes.
const PIPELINE_VERSION: u8 = 1;
const EXT: &str = "hit";

/// Cache key (32 hex chars = 128 bits of SHA-256).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThumbKey(pub String);

/// Identity string of one chain candidate. For files this folds in size and
/// mtime so an edited custom image is a different key; returns `None` if the
/// file cannot be stat'ed (missing => candidate unusable).
pub fn source_identity(c: &ImageRef) -> Option<String> {
    match c {
        ImageRef::Url(u) => Some(format!("url:{u}")),
        ImageRef::File(p) => {
            let m = std::fs::metadata(p).ok()?;
            let mt = m.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?;
            Some(format!("file:{}|{}|{}.{:09}", p.display(), m.len(), mt.as_secs(), mt.subsec_nanos()))
        }
    }
}

pub fn thumb_key(identity: &str, variant: Variant, tw: u32, th: u32, opts: &Opts) -> ThumbKey {
    let mut h = Sha256::new();
    h.update([PIPELINE_VERSION, opts.filter.id()]);
    h.update(tw.to_le_bytes());
    h.update(th.to_le_bytes());
    match variant {
        Variant::Cover => h.update([0u8]),
        Variant::Stage { blur_px, saturation } => {
            h.update([1u8]);
            h.update(blur_px.to_bits().to_le_bytes());
            h.update(saturation.to_bits().to_le_bytes());
            h.update(opts.stage_max_width.to_le_bytes());
            h.update(opts.stage_sigma_out.to_bits().to_le_bytes());
        }
        Variant::Backdrop { blur_px, saturation } => {
            h.update([3u8]);
            h.update(blur_px.to_bits().to_le_bytes());
            h.update(saturation.to_bits().to_le_bytes());
            h.update(opts.stage_max_width.to_le_bytes());
            h.update(opts.stage_sigma_out.to_bits().to_le_bytes());
            h.update(crate::BACKDROP_KNEE.to_bits().to_le_bytes());
            h.update(crate::BACKDROP_CAP.to_bits().to_le_bytes());
        }
        Variant::Glass { blur_px, saturation } => {
            h.update([2u8]);
            h.update(blur_px.to_bits().to_le_bytes());
            h.update(saturation.to_bits().to_le_bytes());
            h.update(opts.stage_max_width.to_le_bytes());
            h.update(opts.stage_sigma_out.to_bits().to_le_bytes());
        }
    }
    h.update(identity.as_bytes());
    ThumbKey(hex(&h.finalize()[..16]))
}

fn path_of(dir: &Path, key: &ThumbKey) -> PathBuf {
    dir.join(format!("{}.{EXT}", key.0))
}

/// Try to load a cached thumb. Any problem (missing, truncated, corrupt) is a miss.
pub fn read(dir: &Path, key: &ThumbKey) -> Option<Decoded> {
    let path = path_of(dir, key);
    let mut f = std::fs::File::open(&path).ok()?;
    let mut head = [0u8; HEADER];
    f.read_exact(&mut head).ok()?;
    if &head[..4] != MAGIC {
        return None;
    }
    let w = u32::from_le_bytes(head[4..8].try_into().unwrap());
    let h = u32::from_le_bytes(head[8..12].try_into().unwrap());
    let codec = head[12];
    let n = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if w == 0 || h == 0 || n > (1 << 28) {
        return None;
    }
    let mut out;
    match codec {
        CODEC_RAW => {
            out = vec![0u8; n];
            f.read_exact(&mut out).ok()?;
        }
        CODEC_LZ4 => {
            let clen = (f.metadata().ok()?.len() as usize).checked_sub(HEADER)?;
            let mut comp = vec![0u8; clen];
            f.read_exact(&mut comp).ok()?;
            out = vec![0u8; n];
            let got = lz4_flex::block::decompress_into(&comp, &mut out).ok()?;
            if got != n {
                return None;
            }
        }
        _ => return None,
    }
    touch_if_old(&f);
    Some(Decoded { width: w, height: h, bgra: out })
}

/// Refresh mtime (used as the LRU clock) at most once per 6 hours per file.
pub(crate) fn touch_if_old(f: &std::fs::File) {
    const MIN_AGE: Duration = Duration::from_secs(6 * 3600);
    if let Ok(m) = f.metadata().and_then(|m| m.modified()) {
        if m.elapsed().map(|e| e > MIN_AGE).unwrap_or(false) {
            let _ = f.set_modified(SystemTime::now());
        }
    }
}

pub fn write(dir: &Path, key: &ThumbKey, d: &Decoded, compress: bool) -> Result<()> {
    let comp = if compress { lz4_flex::block::compress(&d.bgra) } else { Vec::new() };
    let (codec, payload): (u8, &[u8]) =
        if compress && comp.len() * 10 < d.bgra.len() * 9 { (CODEC_LZ4, &comp) } else { (CODEC_RAW, &d.bgra) };
    let mut buf = Vec::with_capacity(HEADER + payload.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&d.width.to_le_bytes());
    buf.extend_from_slice(&d.height.to_le_bytes());
    buf.push(codec);
    buf.extend_from_slice(&[0, 0, 0]);
    buf.extend_from_slice(payload);
    write_atomic(&path_of(dir, key), &buf)
}

// Negative cache: remember that a candidate failed (404, not an image, ...)
// so every app start does not retry the network for the same dead URL.
fn neg_path(dir: &Path, identity: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(b"neg1");
    h.update(identity.as_bytes());
    dir.join("neg").join(hex(&h.finalize()[..16]))
}

pub(crate) fn is_negative(dir: &Path, identity: &str, ttl: Duration) -> bool {
    std::fs::metadata(neg_path(dir, identity))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|age| age < ttl)
        .unwrap_or(false)
}

pub(crate) fn mark_negative(dir: &Path, identity: &str) {
    let _ = write_atomic(&neg_path(dir, identity), b"");
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EvictStats {
    pub files_before: usize,
    pub bytes_before: u64,
    pub files_removed: usize,
    pub bytes_removed: u64,
}

/// Size-capped LRU eviction: deletes the least recently used thumbs (oldest
/// mtime; hits refresh it) until the total is at most `max_bytes`. Also drops
/// stale temp files and expired negative markers. Safe to run on a background
/// thread while the loader is active (readers treat vanished files as a miss).
pub fn evict_lru(dir: &Path, max_bytes: u64) -> std::io::Result<EvictStats> {
    evict_oldest(dir, max_bytes, |name| name.ends_with(EXT))
}

/// [`evict_lru`] for the shared download cache (`images-cache`: one file per
/// URL, named by its SHA-256; hits refresh the mtime like thumbs). A dropped
/// file is downloaded again when something still shows it.
pub fn evict_downloads(dir: &Path, max_bytes: u64) -> std::io::Result<EvictStats> {
    evict_oldest(dir, max_bytes, |name| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn evict_oldest(dir: &Path, max_bytes: u64, is_entry: impl Fn(&str) -> bool) -> std::io::Result<EvictStats> {
    let mut st = EvictStats::default();
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let now = SystemTime::now();
    for e in std::fs::read_dir(dir)? {
        let Ok(e) = e else { continue };
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let path = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        let mtime = m.modified().unwrap_or(now);
        if name.starts_with(".tmp-") {
            if now.duration_since(mtime).map(|d| d > Duration::from_secs(3600)).unwrap_or(false) {
                let _ = std::fs::remove_file(&path);
            }
        } else if is_entry(&name) {
            st.files_before += 1;
            st.bytes_before += m.len();
            files.push((mtime, m.len(), path));
        }
    }
    if let Ok(rd) = std::fs::read_dir(dir.join("neg")) {
        for e in rd.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|a| a > Duration::from_secs(30 * 86400))
                .unwrap_or(false);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut total = st.bytes_before;
    if total <= max_bytes {
        return Ok(st);
    }
    files.sort_by_key(|f| f.0); // oldest first
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            st.files_removed += 1;
            st.bytes_removed += len;
        }
    }
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Opts {
        Opts::new(PathBuf::from("/nonexistent"), None)
    }

    fn sample(w: u32, h: u32, noisy: bool) -> Decoded {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        let mut s = 12345u32;
        for i in 0..w * h {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            if noisy {
                v.extend_from_slice(&s.to_le_bytes());
            } else {
                let b = (i / w) as u8;
                v.extend_from_slice(&[b, b / 2, 255 - b, 255]);
            }
        }
        Decoded { width: w, height: h, bgra: v }
    }

    #[test]
    fn roundtrip_lz4_and_raw() {
        let dir = tempfile::tempdir().unwrap();
        for noisy in [false, true] {
            let d = sample(64, 48, noisy);
            let k = thumb_key("url:a", Variant::Cover, 64, 48, &opts());
            write(dir.path(), &k, &d, true).unwrap();
            let len = std::fs::metadata(path_of(dir.path(), &k)).unwrap().len() as usize;
            if noisy {
                assert_eq!(len, HEADER + d.bgra.len(), "incompressible data stored raw");
            } else {
                assert!(len < d.bgra.len() / 2, "smooth data stored as lz4");
            }
            let back = read(dir.path(), &k).unwrap();
            assert_eq!(back, d);
            assert_eq!(back.bgra.capacity(), back.bgra.len());
        }
    }

    #[test]
    fn corrupt_or_truncated_is_miss() {
        let dir = tempfile::tempdir().unwrap();
        let d = sample(16, 16, false);
        let k = thumb_key("url:a", Variant::Cover, 16, 16, &opts());
        write(dir.path(), &k, &d, true).unwrap();
        let p = path_of(dir.path(), &k);
        let full = std::fs::read(&p).unwrap();
        std::fs::write(&p, &full[..full.len() - 3]).unwrap();
        assert!(read(dir.path(), &k).is_none());
        std::fs::write(&p, b"garbage").unwrap();
        assert!(read(dir.path(), &k).is_none());
        assert!(read(dir.path(), &ThumbKey("0".repeat(32))).is_none());
    }

    #[test]
    fn keys_depend_on_everything() {
        let o = opts();
        let base = thumb_key("url:a", Variant::Cover, 250, 375, &o);
        assert_eq!(base, thumb_key("url:a", Variant::Cover, 250, 375, &o));
        assert_ne!(base, thumb_key("url:b", Variant::Cover, 250, 375, &o));
        assert_ne!(base, thumb_key("url:a", Variant::Cover, 251, 375, &o));
        assert_ne!(base, thumb_key("url:a", Variant::Cover, 250, 376, &o));
        let s1 = thumb_key("url:a", Variant::Stage { blur_px: 40.0, saturation: 1.2 }, 250, 375, &o);
        let s2 = thumb_key("url:a", Variant::Stage { blur_px: 41.0, saturation: 1.2 }, 250, 375, &o);
        let s3 = thumb_key("url:a", Variant::Stage { blur_px: 40.0, saturation: 1.3 }, 250, 375, &o);
        let g1 = thumb_key("url:a", Variant::Glass { blur_px: 40.0, saturation: 1.2 }, 250, 375, &o);
        let b1 = thumb_key("url:a", Variant::Backdrop { blur_px: 40.0, saturation: 1.2 }, 250, 375, &o);
        let all = [&base, &s1, &s2, &s3, &g1, &b1];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(base.0.len(), 32);
    }

    #[test]
    fn file_identity_tracks_mtime_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.png");
        assert!(source_identity(&ImageRef::File(p.clone())).is_none());
        std::fs::write(&p, b"abc").unwrap();
        let a = source_identity(&ImageRef::File(p.clone())).unwrap();
        std::fs::write(&p, b"abcd").unwrap();
        let b = source_identity(&ImageRef::File(p.clone())).unwrap();
        assert_ne!(a, b);
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1000)).unwrap();
        let c = source_identity(&ImageRef::File(p)).unwrap();
        assert_ne!(b, c);
    }

    #[test]
    fn evict_removes_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let d = sample(32, 32, true); // 4 KB raw each
        let mut keys = Vec::new();
        for i in 0..5 {
            let k = thumb_key(&format!("url:{i}"), Variant::Cover, 32, 32, &opts());
            write(dir.path(), &k, &d, true).unwrap();
            let f = std::fs::OpenOptions::new().write(true).open(path_of(dir.path(), &k)).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(1000 - i * 100)).unwrap();
            keys.push(k);
        }
        let one = std::fs::metadata(path_of(dir.path(), &keys[0])).unwrap().len();
        let st = evict_lru(dir.path(), one * 2 + 10).unwrap();
        assert_eq!(st.files_before, 5);
        assert_eq!(st.files_removed, 3);
        assert!(read(dir.path(), &keys[0]).is_none());
        assert!(read(dir.path(), &keys[2]).is_none());
        assert!(read(dir.path(), &keys[3]).is_some());
        assert!(read(dir.path(), &keys[4]).is_some());
        // under cap: nothing happens
        assert_eq!(evict_lru(dir.path(), u64::MAX).unwrap().files_removed, 0);
    }

    #[test]
    fn evict_downloads_keeps_recent_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..4u64 {
            let p = dir.path().join(format!("{i:064x}"));
            std::fs::write(&p, [0u8; 100]).unwrap();
            let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(1000 - i * 100)).unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), [0u8; 1000]).unwrap();
        let st = evict_downloads(dir.path(), 250).unwrap();
        assert_eq!((st.files_before, st.files_removed, st.bytes_removed), (4, 2, 200));
        assert!(!dir.path().join(format!("{:064x}", 0)).exists());
        assert!(dir.path().join(format!("{:064x}", 3)).exists());
        assert!(dir.path().join("notes.txt").exists());
    }

    #[test]
    fn negative_marker_expires() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_negative(dir.path(), "url:x", Duration::from_secs(60)));
        mark_negative(dir.path(), "url:x");
        assert!(is_negative(dir.path(), "url:x", Duration::from_secs(60)));
        assert!(!is_negative(dir.path(), "url:x", Duration::ZERO));
    }
}
