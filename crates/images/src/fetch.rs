//! Source bytes: local files, and HTTP images through Heronite's shared disk
//! cache (`<images-cache>/<hex sha256 of the exact URL string>`, raw bytes).

use crate::decode::sniff;
use crate::error::{ImageError, Result};
use crate::Opts;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// File name Heroic uses in `images-cache` for `url`.
pub fn url_digest(url: &str) -> String {
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    hex(&h.finalize())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const T: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(T[(b >> 4) as usize] as char);
        s.push(T[(b & 15) as usize] as char);
    }
    s
}

/// Path of the shared-cache entry for `url` under `cache_root`.
pub fn cache_path(cache_root: &Path, url: &str) -> PathBuf {
    cache_root.join(url_digest(url))
}

pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| ImageError::Io(format!("{}: {e}", path.display())))
}

/// Bytes for a URL: shared cache hit, else download + atomic cache write.
pub(crate) fn url_bytes(url: &str, opts: &Opts) -> Result<Vec<u8>> {
    let path = cache_path(&opts.images_cache_dir, url);
    if let Some(b) = read_valid_cache(&path) {
        return Ok(b);
    }
    if !opts.allow_network {
        return Err(ImageError::Http(format!("not cached and network disabled: {url}")));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ImageError::Http(format!("unsupported url scheme: {url}")));
    }
    // One download per URL at a time (several variants may want the same URL).
    let lock = url_lock(&path);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(b) = read_valid_cache(&path) {
        return Ok(b); // another thread finished while we waited
    }
    let bytes = download(url, opts)?;
    write_atomic(&path, &bytes)?;
    Ok(bytes)
}

/// Reads a cache entry; an entry that is not an image (HTML error page, empty
/// or truncated file) is deleted so it can be re-downloaded.
fn read_valid_cache(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if sniff(&bytes) {
        Some(bytes)
    } else {
        log::debug!("removing invalid cache entry {}", path.display());
        let _ = std::fs::remove_file(path);
        None
    }
}

fn url_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut m = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if m.len() > 4096 {
        m.retain(|_, v| Arc::strong_count(v) > 1);
    }
    m.entry(path.to_path_buf()).or_default().clone()
}

fn agent(opts: &Opts) -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(opts.connect_timeout))
            .timeout_global(Some(opts.http_timeout))
            .max_redirects(8)
            .user_agent("heronite-images/0.1")
            .build()
            .into()
    })
}

fn download(url: &str, opts: &Opts) -> Result<Vec<u8>> {
    let mut resp = agent(opts)
        .get(url)
        .call()
        .map_err(|e| ImageError::Http(format!("{url}: {e}")))?; // 4xx/5xx are errors
    let status = resp.status();
    if !status.is_success() {
        return Err(ImageError::Http(format!("{url}: status {status}")));
    }
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ctype.starts_with("text/") {
        return Err(ImageError::NotAnImage(format!("{url}: content-type {ctype}")));
    }
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(opts.max_download_bytes)
        .read_to_vec()
        .map_err(|e| ImageError::Http(format!("{url}: {e}")))?;
    if !sniff(&bytes) {
        return Err(ImageError::NotAnImage(format!("{url}: body is not a supported image ({} bytes)", bytes.len())));
    }
    Ok(bytes)
}

/// Write `bytes` to `path` via temp file + rename in the same directory.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().ok_or_else(|| ImageError::Io("no parent dir".into()))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".tmp-{}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        path.file_name().and_then(|s| s.to_str()).unwrap_or("x").chars().take(16).collect::<String>()
    ));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn digest_matches_node_sha256() {
        // sha256("hello") per `createHash('sha256').update('hello').digest('hex')`
        assert_eq!(url_digest("hello"), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    fn png_bytes() -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// Tiny one-thread HTTP server: path -> (status, content-type, body).
    fn serve(routes: Vec<(&'static str, u16, &'static str, Vec<u8>)>) -> (String, std::thread::JoinHandle<()>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let h = std::thread::spawn(move || {
            for _ in 0..routes.len() {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap().to_string();
                let r = routes.iter().find(|r| r.0 == path).unwrap();
                let head = format!("HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", r.1, r.2, r.3.len());
                s.write_all(head.as_bytes()).unwrap();
                s.write_all(&r.3).unwrap();
            }
        });
        (format!("http://{addr}"), h)
    }

    #[test]
    fn download_caches_ok_and_rejects_404_and_html() {
        let png = png_bytes();
        let (base, h) = serve(vec![
            ("/ok.png", 200, "image/png", png.clone()),
            ("/missing", 404, "text/html", b"<html>nope</html>".to_vec()),
            ("/fake", 200, "image/png", b"<html>captive portal</html>".to_vec()),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let opts = Opts::new(dir.path().join("images-cache"), None);

        let ok = format!("{base}/ok.png");
        assert_eq!(url_bytes(&ok, &opts).unwrap(), png);
        assert_eq!(std::fs::read(cache_path(&opts.images_cache_dir, &ok)).unwrap(), png);

        for bad in ["/missing", "/fake"] {
            let u = format!("{base}{bad}");
            assert!(url_bytes(&u, &opts).is_err(), "{bad}");
            assert!(!cache_path(&opts.images_cache_dir, &u).exists(), "{bad} must not be cached");
        }
        h.join().unwrap();
        // no temp files left behind
        let leftovers: Vec<_> = std::fs::read_dir(&opts.images_cache_dir).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1);

        // second read is a pure cache hit (server is gone)
        assert_eq!(url_bytes(&ok, &opts).unwrap(), png);
    }

    #[test]
    fn invalid_cache_entry_is_dropped_and_network_off_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = Opts::new(dir.path().to_path_buf(), None);
        opts.allow_network = false;
        let url = "https://example.invalid/a.jpg";
        std::fs::write(cache_path(dir.path(), url), b"<html>").unwrap();
        assert!(url_bytes(url, &opts).is_err());
        assert!(!cache_path(dir.path(), url).exists());
    }
}
