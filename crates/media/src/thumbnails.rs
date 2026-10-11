//! Screenshot preview cache. Port of `src/local-library/screenshots/thumbnails.ts`.
//!
//! Same cache format as the Electron app (so a migrated cache stays valid):
//! previews are 320x180 quality-60 JPEGs named
//! `<sha256(sourcePath + "\0320x180q60")>.jpg`, each with a `<name>.jpg.json`
//! sidecar `{"sourcePath": "..."}`, in `local_library/screenshot-thumbnails`
//! under an LRU budget (`cacheLimitMb`).
//!
//! * `cover` semantics like sharp's `fit: 'cover', withoutEnlargement: true`:
//!   sources of at least 320x180 are scaled (Lanczos3, via
//!   [`heronite_images::load_cover`]) and center-cropped to exactly 320x180;
//!   a source smaller than that in either dimension is not enlarged and is
//!   only center-cropped to `min(w, 320) x min(h, 180)`.
//! * at most two previews are generated at a time ([`MAX_CONCURRENT_JOBS`]);
//!   concurrent requests for the same source share one job.
//! * least-recently-used eviction (access time, refreshed on every hit) down
//!   to the limit, and entries whose original vanished are removed.
//!
//! All methods are blocking; call them from worker threads. The UI-facing
//! entry point is [`ThumbnailCache::thumbnail`], which returns a BGRA
//! [`heronite_images::Decoded`].

use heronite_images::{Decoded, Filter, ImageRef, Opts};
use image::codecs::jpeg::JpegEncoder;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

pub const THUMB_WIDTH: u32 = 320;
pub const THUMB_HEIGHT: u32 = 180;
pub const THUMB_QUALITY: u8 = 60;
pub const MAX_CONCURRENT_JOBS: usize = 2;
/// Quiet period after a generation before the cache is trimmed.
pub const MAINTENANCE_DELAY: Duration = Duration::from_millis(1500);

/// Size of the cache (`CollectionScreenshotCacheInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheInfo {
    pub size_bytes: u64,
    pub item_count: usize,
    pub limit_bytes: u64,
}

#[derive(Debug)]
pub enum ThumbError {
    /// The cache was cleared while the job waited.
    Cancelled,
    Io(std::io::Error),
    Image(String),
}

impl std::fmt::Display for ThumbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("thumbnail job cancelled"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Image(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ThumbError {}

impl From<std::io::Error> for ThumbError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Default)]
struct State {
    active_jobs: usize,
    /// Cache paths being generated (one job per path).
    pending: HashSet<PathBuf>,
    /// Cache paths whose files are half-written (skipped by the scan).
    writing: HashSet<PathBuf>,
    maintenance_epoch: u64,
    maintenance_thread: bool,
}

struct Inner {
    root: PathBuf,
    limit_bytes: AtomicU64,
    generation: AtomicU64,
    state: Mutex<State>,
    cv: Condvar,
}

/// Disk cache of 320x180 previews. Cheap to clone (shared state).
#[derive(Clone)]
pub struct ThumbnailCache {
    inner: Arc<Inner>,
}

fn limit_from_mb(mb: f64) -> u64 {
    if mb.is_finite() && mb > 0.0 {
        (mb * 1024.0 * 1024.0) as u64
    } else {
        0
    }
}

/// `sha256(sourcePath \0 320x180q60)` as lowercase hex (the cache file stem).
pub fn cache_key(source: &Path) -> String {
    let mut h = Sha256::new();
    h.update(source.to_string_lossy().as_bytes());
    h.update(format!("\0{THUMB_WIDTH}x{THUMB_HEIGHT}q{THUMB_QUALITY}").as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn mtime(meta: &fs::Metadata) -> SystemTime {
    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Marks the cache file as used now (atime is the LRU clock) keeping mtime.
fn touch_access(path: &Path) {
    if let Ok(f) = fs::File::open(path) {
        let _ = f.set_times(fs::FileTimes::new().set_accessed(SystemTime::now()));
    }
}

fn bgra_from_rgb(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for p in rgb.as_chunks::<3>().0 {
        out.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    out
}

/// Decode a cached preview into BGRA.
fn decode_cached(path: &Path) -> Result<Decoded, ThumbError> {
    let img = image::open(path).map_err(|e| ThumbError::Image(e.to_string()))?;
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    Ok(Decoded { width, height, bgra: bgra_from_rgb(rgb.as_raw()) })
}

/// Build the preview raster of `source` (sharp `cover` + `withoutEnlargement`).
fn render_preview(source: &Path) -> Result<Decoded, ThumbError> {
    let (sw, sh) = image::image_dimensions(source).map_err(|e| ThumbError::Image(e.to_string()))?;
    if sw >= THUMB_WIDTH && sh >= THUMB_HEIGHT {
        let mut opts = Opts::new(PathBuf::new(), None);
        opts.allow_network = false;
        opts.filter = Filter::Lanczos3;
        return heronite_images::load_cover(
            &[ImageRef::File(source.to_path_buf())],
            THUMB_WIDTH,
            THUMB_HEIGHT,
            &opts,
        )
        .map_err(|e| ThumbError::Image(e.to_string()));
    }
    // Not enlarged: center-crop to what fits.
    let mut img = image::open(source).map_err(|e| ThumbError::Image(e.to_string()))?.to_rgb8();
    let (w, h) = (sw.min(THUMB_WIDTH), sh.min(THUMB_HEIGHT));
    let (x, y) = ((sw - w) / 2, (sh - h) / 2);
    let cropped = image::imageops::crop(&mut img, x, y, w, h).to_image();
    Ok(Decoded { width: w, height: h, bgra: bgra_from_rgb(cropped.as_raw()) })
}

fn encode_jpeg(d: &Decoded) -> Result<Vec<u8>, ThumbError> {
    let mut rgb = Vec::with_capacity(d.bgra.len() / 4 * 3);
    for p in d.bgra.as_chunks::<4>().0 {
        rgb.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, THUMB_QUALITY)
        .encode(&rgb, d.width, d.height, image::ExtendedColorType::Rgb8)
        .map_err(|e| ThumbError::Image(e.to_string()))?;
    Ok(out)
}

impl ThumbnailCache {
    /// `root` is typically `DataPaths::screenshot_thumbnails_dir()`.
    pub fn new(root: impl Into<PathBuf>, limit_mb: f64) -> Self {
        Self {
            inner: Arc::new(Inner {
                root: root.into(),
                limit_bytes: AtomicU64::new(limit_from_mb(limit_mb)),
                generation: AtomicU64::new(0),
                state: Mutex::new(State::default()),
                cv: Condvar::new(),
            }),
        }
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn limit_bytes(&self) -> u64 {
        self.inner.limit_bytes.load(Ordering::Relaxed)
    }

    /// Changes the budget (`screenshots.cacheLimitMb`); call [`maintain`](Self::maintain) to apply it.
    pub fn set_limit_mb(&self, limit_mb: f64) {
        self.inner.limit_bytes.store(limit_from_mb(limit_mb), Ordering::Relaxed);
    }

    /// Cache file for `source` (it may not exist yet).
    pub fn cache_path_for(&self, source: &Path) -> PathBuf {
        self.inner.root.join(format!("{}.jpg", cache_key(source)))
    }

    /// The cached preview if it exists and is not older than `source`.
    /// A hit refreshes its LRU clock.
    pub fn cached_path(&self, source: &Path) -> Option<PathBuf> {
        let cache = self.cache_path_for(source);
        self.current(source, &cache).then_some(cache)
    }

    fn current(&self, source: &Path, cache: &Path) -> bool {
        let (Ok(s), Ok(c)) = (fs::metadata(source), fs::metadata(cache)) else { return false };
        if c.len() > 0 && mtime(&c) >= mtime(&s) {
            touch_access(cache);
            true
        } else {
            false
        }
    }

    /// `from` became `to` with the same pixels and times (a lossless
    /// conversion): its preview moves to the new key instead of being rebuilt.
    pub fn rename_source(&self, from: &Path, to: &Path) {
        let (old, new) = (self.cache_path_for(from), self.cache_path_for(to));
        if !old.is_file() || fs::rename(&old, &new).is_err() {
            return;
        }
        let _ = fs::remove_file(sidecar(&old));
        let meta = serde_json::json!({ "sourcePath": to.to_string_lossy() });
        let _ = fs::write(sidecar(&new), meta.to_string());
    }

    /// Makes sure the preview exists and returns its JPEG path.
    pub fn thumbnail_path(&self, source: &Path) -> Result<PathBuf, ThumbError> {
        self.ensure(source).map(|(p, _)| p)
    }

    /// Preview as a BGRA raster ready for the UI (320x180, or smaller for
    /// small sources).
    pub fn thumbnail(&self, source: &Path) -> Result<Decoded, ThumbError> {
        match self.ensure(source)? {
            (_, Some(fresh)) => Ok(fresh),
            (path, None) => decode_cached(&path),
        }
    }

    fn ensure(&self, source: &Path) -> Result<(PathBuf, Option<Decoded>), ThumbError> {
        let inner = &*self.inner;
        let cache = self.cache_path_for(source);
        if self.current(source, &cache) {
            return Ok((cache, None));
        }
        let generation = inner.generation.load(Ordering::SeqCst);

        // One job per cache path; at most MAX_CONCURRENT_JOBS at a time.
        {
            let mut st = inner.state.lock().unwrap();
            while st.pending.contains(&cache) {
                st = inner.cv.wait(st).unwrap();
                if self.current(source, &cache) {
                    return Ok((cache, None));
                }
            }
            st.pending.insert(cache.clone());
            while st.active_jobs >= MAX_CONCURRENT_JOBS {
                st = inner.cv.wait(st).unwrap();
            }
            st.active_jobs += 1;
            st.writing.insert(cache.clone());
        }

        let result = (|| {
            if generation != inner.generation.load(Ordering::SeqCst) {
                return Err(ThumbError::Cancelled);
            }
            if self.current(source, &cache) {
                return Ok((cache.clone(), None));
            }
            fs::create_dir_all(&inner.root)?;
            let raster = render_preview(source)?;
            let jpeg = encode_jpeg(&raster)?;
            let tmp = inner.root.join(format!(".{}.tmp", cache_key(source)));
            fs::write(&tmp, &jpeg)?;
            fs::rename(&tmp, &cache)?;
            if generation != inner.generation.load(Ordering::SeqCst) {
                let _ = fs::remove_file(&cache);
                return Err(ThumbError::Cancelled);
            }
            let meta = serde_json::json!({ "sourcePath": source.to_string_lossy() });
            fs::write(sidecar(&cache), meta.to_string())?;
            Ok((cache.clone(), Some(raster)))
        })();

        {
            let mut st = inner.state.lock().unwrap();
            st.active_jobs -= 1;
            st.pending.remove(&cache);
            st.writing.remove(&cache);
            inner.cv.notify_all();
        }
        if result.is_ok() {
            self.schedule_maintenance();
        }
        result
    }

    /// Trims the cache (debounced by [`MAINTENANCE_DELAY`], on a short-lived thread).
    pub fn schedule_maintenance(&self) {
        let mut st = self.inner.state.lock().unwrap();
        st.maintenance_epoch += 1;
        if st.maintenance_thread {
            return;
        }
        st.maintenance_thread = true;
        drop(st);
        let this = self.clone();
        std::thread::spawn(move || loop {
            let epoch = this.inner.state.lock().unwrap().maintenance_epoch;
            std::thread::sleep(MAINTENANCE_DELAY);
            let mut st = this.inner.state.lock().unwrap();
            if st.maintenance_epoch == epoch {
                st.maintenance_thread = false;
                drop(st);
                this.maintain();
                return;
            }
        });
    }

    /// Removes entries whose original vanished or whose files are inconsistent,
    /// then evicts least-recently-used previews until the cache fits the limit.
    pub fn maintain(&self) -> CacheInfo {
        let limit = self.limit_bytes();
        let mut entries = self.scan();
        let mut size: u64 = entries.iter().map(|e| e.size).sum();
        let mut count = entries.len();
        if size > limit {
            entries.sort_by_key(|e| e.last_access);
            for e in entries {
                if size <= limit {
                    break;
                }
                let _ = fs::remove_file(&e.image);
                let _ = fs::remove_file(sidecar(&e.image));
                size -= e.size;
                count -= 1;
            }
        }
        CacheInfo { size_bytes: size, item_count: count, limit_bytes: limit }
    }

    /// Size and count without modifying anything (valid entries are not verified).
    pub fn info(&self) -> CacheInfo {
        let mut size = 0u64;
        let mut count = 0usize;
        if let Ok(rd) = fs::read_dir(&self.inner.root) {
            for e in rd.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                let Ok(m) = e.metadata() else { continue };
                if name.ends_with(".jpg") {
                    count += 1;
                    size += m.len();
                } else if name.ends_with(".jpg.json") {
                    size += m.len();
                }
            }
        }
        CacheInfo { size_bytes: size, item_count: count, limit_bytes: self.limit_bytes() }
    }

    /// Deletes the whole cache folder (in-flight jobs finish first and nothing
    /// queued afterwards is written).
    pub fn clear(&self) -> CacheInfo {
        let inner = &*self.inner;
        inner.generation.fetch_add(1, Ordering::SeqCst);
        {
            let mut st = inner.state.lock().unwrap();
            st.maintenance_epoch += 1;
            while !st.pending.is_empty() {
                st = inner.cv.wait(st).unwrap();
            }
        }
        let _ = fs::remove_dir_all(&inner.root);
        CacheInfo { size_bytes: 0, item_count: 0, limit_bytes: self.limit_bytes() }
    }

    fn scan(&self) -> Vec<Entry> {
        let root = &self.inner.root;
        let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
        let names: Vec<String> = rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
        let writing = self.inner.state.lock().unwrap().writing.clone();
        let is_writing = |name: &str| writing.contains(&root.join(name));
        let images: HashSet<&str> = names.iter().map(String::as_str).filter(|n| n.ends_with(".jpg")).collect();

        // Orphan sidecars.
        for n in &names {
            if let Some(stem) = n.strip_suffix(".json").filter(|s| s.ends_with(".jpg")) {
                if !images.contains(stem) && !is_writing(stem) {
                    let _ = fs::remove_file(root.join(n));
                }
            }
            // Leftover temp files of crashed writes (an hour old).
            if n.starts_with('.') && n.ends_with(".tmp") {
                let old = fs::metadata(root.join(n))
                    .map(|m| mtime(&m).elapsed().map(|a| a > Duration::from_secs(3600)).unwrap_or(false))
                    .unwrap_or(false);
                if old {
                    let _ = fs::remove_file(root.join(n));
                }
            }
        }

        let mut out = Vec::new();
        for name in images {
            if is_writing(name) {
                continue;
            }
            let image = root.join(name);
            let meta_path = sidecar(&image);
            let entry = (|| {
                let im = fs::metadata(&image).ok()?;
                let text = fs::read_to_string(&meta_path).ok()?;
                let v: serde_json::Value = serde_json::from_str(&text).ok()?;
                let src = v.get("sourcePath")?.as_str()?;
                fs::metadata(src).ok()?;
                let mm = fs::metadata(&meta_path).ok()?;
                let atime = im.accessed().unwrap_or(SystemTime::UNIX_EPOCH);
                Some(Entry { image: image.clone(), size: im.len() + mm.len(), last_access: atime })
            })();
            match entry {
                Some(e) => out.push(e),
                None => {
                    let _ = fs::remove_file(&image);
                    let _ = fs::remove_file(&meta_path);
                }
            }
        }
        out
    }
}

struct Entry {
    image: PathBuf,
    size: u64,
    last_access: SystemTime,
}

fn sidecar(image: &Path) -> PathBuf {
    let mut s = image.as_os_str().to_owned();
    s.push(".json");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn write_png(path: &Path, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) {
        let img = RgbImage::from_fn(w, h, |x, y| Rgb(f(x, y)));
        img.save(path).unwrap();
    }

    #[test]
    fn key_matches_the_electron_derivation() {
        // node: sha256("/a/b.png" + "\0" + "320x180q60")
        assert_eq!(
            cache_key(Path::new("/a/b.png")),
            "4773d2a4aa1a4d6e2e6caea2b9e19019a1277ae88436d30ec4d021b6c3c11779"
        );
    }

    #[test]
    fn generates_cover_preview_and_hits_cache() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("shot.png");
        // 960x360 (wider than 16:9): red left, green middle band, blue right.
        write_png(&src, 960, 360, |x, _| if x < 150 { [255, 0, 0] } else if x < 810 { [0, 255, 0] } else { [0, 0, 255] });
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        let d = cache.thumbnail(&src).unwrap();
        assert_eq!((d.width, d.height), (320, 180));
        assert_eq!(d.bgra.len(), 320 * 180 * 4);
        // Center crop => all green (BGRA: b,g,r,a), allowing JPEG-free fresh raster.
        let p = &d.bgra[(90 * 320 + 160) * 4..][..4];
        assert!(p[1] > 240 && p[0] < 16 && p[2] < 16 && p[3] == 255, "{p:?}");
        let edge = &d.bgra[(90 * 320 + 2) * 4..][..4];
        assert!(edge[1] > 200, "left edge is still inside the green band: {edge:?}");

        let path = cache.cached_path(&src).expect("cached");
        assert!(path.to_string_lossy().ends_with(&format!("{}.jpg", cache_key(&src))));
        assert!(sidecar(&path).exists());
        let meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(sidecar(&path)).unwrap()).unwrap();
        assert_eq!(meta["sourcePath"], src.to_string_lossy().as_ref());
        // Second call decodes the cached JPEG to the same size.
        let again = cache.thumbnail(&src).unwrap();
        assert_eq!((again.width, again.height), (320, 180));
        let q = &again.bgra[(90 * 320 + 160) * 4..][..4];
        assert!(q[1] > 230 && q[0] < 30 && q[2] < 30, "{q:?}");
        let info = cache.info();
        assert_eq!(info.item_count, 1);
        assert!(info.size_bytes > 0);
    }

    #[test]
    fn a_renamed_source_keeps_its_preview() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("shot.png");
        write_png(&src, 400, 240, |x, _| [x as u8, 0, 0]);
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        cache.thumbnail(&src).unwrap();
        let to = dir.path().join("shot.webp");
        fs::rename(&src, &to).unwrap();
        cache.rename_source(&src, &to);
        let path = cache.cached_path(&to).expect("moved, still current");
        let meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(sidecar(&path)).unwrap()).unwrap();
        assert_eq!(meta["sourcePath"], to.to_string_lossy().as_ref());
        assert!(!cache.cache_path_for(&src).exists());
        assert_eq!(cache.info().item_count, 1);
    }

    #[test]
    fn small_sources_are_not_enlarged() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("small.png");
        write_png(&src, 200, 400, |_, _| [10, 20, 30]);
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        let d = cache.thumbnail(&src).unwrap();
        assert_eq!((d.width, d.height), (200, 180));
        let src2 = dir.path().join("tiny.png");
        write_png(&src2, 100, 50, |_, _| [10, 20, 30]);
        let d = cache.thumbnail(&src2).unwrap();
        assert_eq!((d.width, d.height), (100, 50));
        let again = cache.thumbnail(&src2).unwrap();
        assert_eq!((again.width, again.height), (100, 50));
    }

    #[test]
    fn stale_cache_is_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        write_png(&src, 640, 360, |_, _| [255, 0, 0]);
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        cache.thumbnail(&src).unwrap();
        let c = cache.cache_path_for(&src);
        // Make the source newer than the cached preview.
        let f = fs::File::options().write(true).open(&src).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(10)).unwrap();
        assert!(cache.cached_path(&src).is_none());
        write_png(&src, 640, 360, |_, _| [0, 0, 255]);
        let f = fs::File::options().write(true).open(&src).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(10)).unwrap();
        let d = cache.thumbnail(&src).unwrap();
        assert!(d.bgra[0] > 200 && d.bgra[2] < 50, "blue now: {:?}", &d.bgra[..4]);
        assert!(c.exists());
    }

    #[test]
    fn maintain_removes_vanished_originals_and_evicts_lru() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        let mut srcs = Vec::new();
        for i in 0..4u32 {
            let src = dir.path().join(format!("s{i}.png"));
            // Noisy so the JPEGs are not tiny.
            write_png(&src, 400, 300, |x, y| {
                let v = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503)).wrapping_add(i * 977);
                [v as u8, (v >> 8) as u8, (v >> 16) as u8]
            });
            cache.thumbnail(&src).unwrap();
            srcs.push(src);
        }
        assert_eq!(cache.info().item_count, 4);

        // Vanished original => removed together with its sidecar.
        let gone = cache.cache_path_for(&srcs[0]);
        fs::remove_file(&srcs[0]).unwrap();
        let info = cache.maintain();
        assert_eq!(info.item_count, 3);
        assert!(!gone.exists() && !sidecar(&gone).exists());

        // Give distinct access times: s1 oldest, s3 newest.
        for (i, age) in [(1usize, 300u64), (2, 200), (3, 100)] {
            let c = cache.cache_path_for(&srcs[i]);
            let f = fs::File::open(&c).unwrap();
            f.set_times(fs::FileTimes::new().set_accessed(SystemTime::now() - Duration::from_secs(age))).unwrap();
        }
        let total = cache.info().size_bytes;
        // Room for roughly two entries only.
        cache.inner.limit_bytes.store(total * 2 / 3 + 1, Ordering::Relaxed);
        let info = cache.maintain();
        assert!(info.item_count < 3 && info.size_bytes <= info.limit_bytes, "{info:?}");
        assert!(cache.info().item_count == info.item_count);
        // Oldest went first, newest survives.
        assert!(!cache.cache_path_for(&srcs[1]).exists());
        assert!(cache.cache_path_for(&srcs[3]).exists());
    }

    #[test]
    fn orphan_sidecars_and_corrupt_entries_are_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("aaa.jpg.json"), "{}").unwrap();
        fs::write(root.join("bbb.jpg"), "x").unwrap(); // no sidecar
        let cache = ThumbnailCache::new(&root, 50.0);
        let info = cache.maintain();
        assert_eq!(info.item_count, 0);
        assert!(!root.join("aaa.jpg.json").exists() && !root.join("bbb.jpg").exists());
    }

    #[test]
    fn clear_removes_everything() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        write_png(&src, 640, 360, |_, _| [1, 2, 3]);
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        cache.thumbnail(&src).unwrap();
        let info = cache.clear();
        assert_eq!((info.item_count, info.size_bytes), (0, 0));
        assert!(!cache.root().exists());
        // Usable again afterwards.
        cache.thumbnail(&src).unwrap();
        assert_eq!(cache.info().item_count, 1);
    }

    #[test]
    fn concurrent_requests_share_one_job() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        write_png(&src, 1280, 720, |x, y| [x as u8, y as u8, 0]);
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let (c, s) = (cache.clone(), src.clone());
                std::thread::spawn(move || c.thumbnail(&s).map(|d| (d.width, d.height)))
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap().unwrap(), (320, 180));
        }
        assert_eq!(cache.info().item_count, 1);
    }

    #[test]
    fn unreadable_source_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("cache"), 50.0);
        let bad = dir.path().join("bad.png");
        fs::write(&bad, b"not an image").unwrap();
        assert!(cache.thumbnail(&bad).is_err());
        assert!(cache.thumbnail(&dir.path().join("missing.png")).is_err());
        assert_eq!(cache.info().item_count, 0);
    }
}
