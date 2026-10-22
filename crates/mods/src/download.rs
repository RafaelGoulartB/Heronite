//! Streams a file to disk with progress and cancellation.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use heronite_data::paths::percent_decode;
use heronite_i18n::tr;
use ureq::ResponseExt;

use crate::http;
use crate::paths::safe_name;
use crate::Progress;

/// Progress callbacks come at most this often (and once at the end).
const TICK: Duration = Duration::from_millis(100);

/// Downloads `url` into `dir` and gives the file's path. The name comes from
/// the server's `Content-Disposition`, else `name_hint`, else the URL's last
/// segment. A file of that name with the size announced is reused; any other
/// is kept and the new one gets `name (2).ext`. `cancel` is checked between
/// chunks; the partial file is removed on cancel or error.
pub fn download(
    url: &str,
    dir: &Path,
    name_hint: Option<&str>,
    headers: &[(&str, &str)],
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let mut resp = http::get_stream(url, headers)?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let disposition = resp.headers().get("content-disposition").and_then(|v| v.to_str().ok()).and_then(disposition_name);
    let name = file_name(disposition.as_deref(), name_hint, &resp.get_uri().to_string());
    fs::create_dir_all(dir).map_err(|e| write_error(dir, e))?;
    let (dest, reuse) = pick_path(dir, &name, total);
    if reuse {
        progress(Progress { done: total, total });
        return Ok(dest);
    }
    let part = part_path(&dest);
    let mut file = fs::File::create(&part).map_err(|e| write_error(&part, e))?;
    let result = copy_body(resp.body_mut().with_config().limit(u64::MAX).reader(), &mut file, &part, total, progress, cancel);
    drop(file);
    match result.and_then(|()| fs::rename(&part, &dest).map_err(|e| write_error(&dest, e))) {
        Ok(()) => Ok(dest),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

fn copy_body(
    mut body: impl Read,
    file: &mut fs::File,
    part: &Path,
    total: u64,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut buf = vec![0u8; 128 * 1024];
    let mut done = 0u64;
    let mut last = Instant::now();
    progress(Progress { done, total });
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(tr!("The download was cancelled").to_string());
        }
        let n = body.read(&mut buf).map_err(|e| tr!("The download stopped: {error}", error = e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| write_error(part, e))?;
        done += n as u64;
        if last.elapsed() >= TICK {
            last = Instant::now();
            progress(Progress { done, total });
        }
    }
    file.flush().map_err(|e| write_error(part, e))?;
    if total > 0 && done < total {
        return Err(tr!("The download stopped before the end").to_string());
    }
    progress(Progress { done, total: total.max(done) });
    Ok(())
}

fn write_error(path: &Path, e: std::io::Error) -> String {
    tr!("Could not write {path}: {error}", path = path.display(), error = e)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

/// The path to write and whether it is already there complete.
fn pick_path(dir: &Path, name: &str, total: u64) -> (PathBuf, bool) {
    let (stem, ext) = split_ext(name);
    let mut n = 1u32;
    loop {
        let candidate = if n == 1 { dir.join(name) } else { dir.join(format!("{stem} ({n}){ext}")) };
        match fs::metadata(&candidate) {
            Err(_) => return (candidate, false),
            Ok(m) if total > 0 && m.is_file() && m.len() == total => return (candidate, true),
            Ok(_) => n += 1,
        }
    }
}

/// `("a.tar", ".gz")`-style split that keeps `.tar.*` together.
fn split_ext(name: &str) -> (&str, &str) {
    let Some(dot) = name.rfind('.').filter(|d| *d > 0) else { return (name, "") };
    let dot = match name[..dot].strip_suffix(".tar") {
        Some(_) => dot - 4,
        None => dot,
    };
    (&name[..dot], &name[dot..])
}

/// The file name to save as (see [`download`]), already safe as a path part.
fn file_name(disposition: Option<&str>, hint: Option<&str>, final_url: &str) -> String {
    let from_url = || {
        let path = final_url.split(['?', '#']).next().unwrap_or("");
        let path = path.split_once("://").map_or(path, |(_, r)| r.split_once('/').map_or("", |(_, p)| p));
        percent_decode(path.rsplit('/').next().unwrap_or(""))
    };
    let raw = [disposition.map(str::to_string), hint.map(str::to_string), Some(from_url())]
        .into_iter()
        .flatten()
        .map(|n| safe_name(n.trim()))
        .find(|n| n != "_" && !n.is_empty());
    raw.unwrap_or_else(|| "download".to_string())
}

/// The file name of a `Content-Disposition` value (`filename*` wins).
fn disposition_name(value: &str) -> Option<String> {
    let param = |key: &str| {
        let lower = value.to_ascii_lowercase();
        let start = lower.find(key)? + key.len();
        let rest = value[start..].trim_start();
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut out = String::new();
            let mut chars = quoted.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => out.extend(chars.next()),
                    c => out.push(c),
                }
            }
            return Some(out);
        }
        Some(rest.split(';').next().unwrap_or("").trim().to_string())
    };
    if let Some(v) = param("filename*=") {
        // RFC 5987: `charset'language'percent-encoded`.
        let encoded = v.splitn(3, '\'').nth(2).unwrap_or(&v);
        let name = percent_decode(encoded);
        if !name.is_empty() {
            return Some(name);
        }
    }
    param("filename=").filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn disposition_forms() {
        assert_eq!(disposition_name("attachment; filename=\"SkyUI 5.2.zip\"").as_deref(), Some("SkyUI 5.2.zip"));
        assert_eq!(disposition_name("attachment; filename=plain.7z").as_deref(), Some("plain.7z"));
        assert_eq!(
            disposition_name("attachment; filename=\"fallback.zip\"; filename*=UTF-8''Caf%C3%A9%20mod.zip").as_deref(),
            Some("Café mod.zip")
        );
        assert_eq!(disposition_name("inline"), None);
        assert_eq!(disposition_name("attachment; filename=\"a\\\"b.zip\"").as_deref(), Some("a\"b.zip"));
    }

    #[test]
    fn names_are_picked_in_order_and_sanitized() {
        let url = "https://cdn.example.org/files/My%20Mod-1-2.zip?md5=abc&expires=9";
        assert_eq!(file_name(Some("server.zip"), Some("hint.zip"), url), "server.zip");
        assert_eq!(file_name(None, Some("hint.zip"), url), "hint.zip");
        assert_eq!(file_name(None, None, url), "My Mod-1-2.zip");
        assert_eq!(file_name(Some("../../etc/passwd"), None, url), "_.._etc_passwd");
        assert_eq!(file_name(Some(".hidden.zip"), None, url), "hidden.zip");
        assert_eq!(file_name(None, None, "https://example.org/"), "download");
    }

    #[test]
    fn collisions_get_a_counter_and_equal_sizes_are_reused() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(pick_path(dir.path(), "a.zip", 3), (dir.path().join("a.zip"), false));
        fs::write(dir.path().join("a.zip"), b"abc").unwrap();
        assert_eq!(pick_path(dir.path(), "a.zip", 3), (dir.path().join("a.zip"), true));
        assert_eq!(pick_path(dir.path(), "a.zip", 5), (dir.path().join("a (2).zip"), false));
        assert_eq!(pick_path(dir.path(), "a.zip", 0), (dir.path().join("a (2).zip"), false));
        fs::write(dir.path().join("a (2).zip"), b"abcde").unwrap();
        assert_eq!(pick_path(dir.path(), "a.zip", 5), (dir.path().join("a (2).zip"), true));
        assert_eq!(split_ext("x.tar.gz"), ("x", ".tar.gz"));
        assert_eq!(split_ext("noext"), ("noext", ""));
        assert_eq!(split_ext(".rc"), (".rc", ""));
    }

    /// Serves `body` once for every connection until the listener is dropped.
    fn serve(headers: &'static str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}/files/fallback.bin")
    }

    #[test]
    fn downloads_a_file_with_progress_and_the_server_name() {
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let url = serve("Content-Disposition: attachment; filename=\"served.zip\"\r\n", body.clone());
        let dir = tempfile::tempdir().unwrap();
        let mut seen = Vec::new();
        let path = download(&url, dir.path(), Some("hint.zip"), &[], &mut |p| seen.push(p), &AtomicBool::new(false)).unwrap();
        assert_eq!(path, dir.path().join("served.zip"));
        assert_eq!(fs::read(&path).unwrap(), body);
        assert_eq!(seen.last().copied(), Some(Progress { done: 300_000, total: 300_000 }));
        assert!(!dir.path().join("served.zip.part").exists());
        // Same name and size: reused, not downloaded twice.
        let again = download(&url, dir.path(), None, &[], &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(again, path);
        // Same name, other content: a new file next to it.
        fs::write(&path, b"old").unwrap();
        let other = download(&url, dir.path(), None, &[], &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(other, dir.path().join("served (2).zip"));
        assert_eq!(fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn the_url_names_the_file_without_a_hint() {
        let url = serve("", b"hello".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let path = download(&url, dir.path(), None, &[], &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(path, dir.path().join("fallback.bin"));
    }

    #[test]
    fn cancel_leaves_nothing_behind() {
        let url = serve("", vec![7u8; 500_000]);
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(true);
        let err = download(&url, dir.path(), None, &[], &mut |_| {}, &cancel).unwrap_err();
        assert_eq!(err, "The download was cancelled");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn http_errors_are_sentences() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let err = download(&format!("http://{addr}/x"), dir.path(), None, &[], &mut |_| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(err.starts_with("Not found on 127.0.0.1"), "{err}");
    }
}
