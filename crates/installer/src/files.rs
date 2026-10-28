//! The `files` of a script: each has an id (a variable the steps use, `$setup`)
//! and a source:
//!
//! * an `http(s)://` URL, downloaded into the install cache (kept between
//!   attempts, so a failed install does not download again);
//! * a local path (`/x`, `file:///x`, `$SCRIPTDIR/x`, or relative to the
//!   script's folder), used where it is;
//! * `N/A` or `N/A:<what to pick>`, a file the user selects (bought games,
//!   files behind a login);
//! * anything else (`$STEAM`, other schemes) cannot be fetched here.
//!
//! A file can be a map (`url`, `filename`, `checksum` as `md5:<hex>`,
//! `sha1:`, `sha256:` or `sha512:`, `referer`).

use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use heronite_i18n::tr;
use serde_json::Value;

use crate::text::{get_nonempty, scalar};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpec {
    pub id: String,
    pub source: Source,
    pub filename: Option<String>,
    pub checksum: Option<String>,
    pub referer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Url(String),
    /// A path, maybe with variables or relative to the script.
    Local(String),
    /// The user picks the file; the text says which one.
    Ask(String),
    /// Cannot be fetched here (the raw value).
    Unsupported(String),
}

/// Reads one entry of the `files` list (`- id: <source or map>`).
pub fn parse_entry(entry: &Value) -> Vec<FileSpec> {
    let Value::Object(m) = entry else { return Vec::new() };
    m.iter().map(|(id, meta)| parse_meta(id, meta)).collect()
}

fn parse_meta(id: &str, meta: &Value) -> FileSpec {
    let (url, filename, checksum, referer) = match meta {
        Value::Object(o) => (
            get_nonempty(o, "url").unwrap_or_default(),
            get_nonempty(o, "filename"),
            get_nonempty(o, "checksum"),
            get_nonempty(o, "referer"),
        ),
        v => (scalar(v).unwrap_or_default(), None, None, None),
    };
    FileSpec { id: id.to_string(), source: source_of(url.trim()), filename, checksum, referer }
}

fn source_of(url: &str) -> Source {
    if let Some(rest) = url.strip_prefix("N/A") {
        return Source::Ask(rest.trim_start_matches(':').trim().to_string());
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Source::Url(url.to_string());
    }
    if let Some(path) = url.strip_prefix("file://") {
        return Source::Local(percent_decode(path));
    }
    if url.is_empty() || lower.starts_with("$steam") || lower.starts_with("$winesteam") || url.contains("://") {
        return Source::Unsupported(url.to_string());
    }
    Source::Local(url.to_string())
}

impl FileSpec {
    /// The file name it gets in the cache: `filename`, else the URL's.
    pub fn file_name(&self) -> String {
        if let Some(f) = self.filename.as_deref().map(str::trim).filter(|f| !f.is_empty() && !f.contains('/')) {
            return f.to_string();
        }
        let name = match &self.source {
            Source::Url(u) | Source::Local(u) => {
                let path = u.split(['?', '#']).next().unwrap_or("");
                percent_decode(path.trim_end_matches('/').rsplit('/').next().unwrap_or(""))
            }
            Source::Ask(_) | Source::Unsupported(_) => String::new(),
        };
        if name.is_empty() || name == "." || name == ".." {
            self.id.clone()
        } else {
            name
        }
    }

    /// What to ask the user for a file they provide.
    pub fn prompt(&self) -> String {
        match &self.source {
            Source::Ask(p) if !p.is_empty() => p.clone(),
            _ => match &self.filename {
                Some(f) => tr!("Select {file}", file = f),
                None => tr!("Select the file \"{id}\"", id = self.id),
            },
        }
    }
}

/// `%20` and friends.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let digit = |c: u8| (c as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (digit(b[i + 1]), digit(b[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Checks `path` against a `algo:hex` checksum (a bare hex is guessed from
/// its length). An unknown algorithm is not an error: there is nothing to
/// compare with.
pub fn verify(path: &Path, checksum: &str, cancel: &dyn Fn() -> bool) -> Result<(), String> {
    let (algo, expected) = match checksum.split_once(':') {
        Some((a, h)) => (a.trim().to_ascii_lowercase(), h.trim().to_ascii_lowercase()),
        None => {
            let h = checksum.trim().to_ascii_lowercase();
            let algo = match h.len() {
                32 => "md5",
                40 => "sha1",
                64 => "sha256",
                128 => "sha512",
                _ => return Ok(()),
            };
            (algo.to_string(), h)
        }
    };
    let actual = match algo.as_str() {
        "md5" => digest_file::<md5::Md5>(path, cancel)?,
        "sha256" => digest_file::<sha2::Sha256>(path, cancel)?,
        "sha512" => digest_file::<sha2::Sha512>(path, cancel)?,
        "sha1" => sha1_file(path, cancel)?,
        _ => {
            log::warn!("unknown checksum type {algo} for {}", path.display());
            return Ok(());
        }
    };
    if actual == expected {
        Ok(())
    } else {
        Err(tr!("{file} is not the expected file (its checksum differs)", file = file_label(path)))
    }
}

fn file_label(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn read_chunks(path: &Path, cancel: &dyn Fn() -> bool, mut f: impl FnMut(&[u8])) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel() {
            return Err(crate::engine::CANCELLED.into());
        }
        let n = file.read(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            return Ok(());
        }
        f(&buf[..n]);
    }
}

fn digest_file<D: sha2::Digest>(path: &Path, cancel: &dyn Fn() -> bool) -> Result<String, String> {
    let mut h = D::new();
    read_chunks(path, cancel, |b| h.update(b))?;
    Ok(hex(&h.finalize()))
}

fn sha1_file(path: &Path, cancel: &dyn Fn() -> bool) -> Result<String, String> {
    let mut h = sha1_smol::Sha1::new();
    read_chunks(path, cancel, |b| h.update(b))?;
    Ok(h.digest().to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Downloads `url` into `dest`, through `dest.part` (renamed when complete).
/// `progress(bytes, total)` is called as data arrives. A connection that sends
/// nothing for 30 s fails, and `cancel` stops it within a fraction of a second.
pub fn download(
    url: &str,
    dest: &Path,
    referer: Option<&str>,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancel: &dyn Fn() -> bool,
) -> Result<(), String> {
    use heronite_net::Error;
    let headers = referer.map(|r| [("Referer", r)]);
    let opts = heronite_net::Options {
        user_agent: "Mozilla/5.0 (X11; Linux x86_64) heronite",
        headers: headers.as_ref().map_or(&[], |h| h.as_slice()),
        connect_timeout: Duration::from_secs(20),
        max_redirects: 10,
        on_progress: Some(progress),
        ..Default::default()
    };
    heronite_net::download(url, dest, opts, cancel).map(|_| ()).map_err(|e| {
        let url = heronite_net::redact_url(url);
        match e {
            Error::Cancelled => crate::engine::CANCELLED.to_string(),
            Error::Http(code) => tr!("The download of {url} failed (HTTP {code})", url, code),
            Error::Truncated { .. } => tr!("The download of {url} stopped before the end", url),
            Error::Unreachable { detail, .. } => tr!("The download of {url} failed: {error}", url, error = detail),
            e => tr!("The download of {url} failed: {error}", url, error = e),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    /// Answers one request: `reply` gets the connection after the request headers (lowercased) were read.
    fn serve_once(reply: impl FnOnce(TcpStream, String) + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/game.zip", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let Some(Ok(stream)) = listener.incoming().next() else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            reply(stream, headers);
        });
        url
    }

    #[test]
    fn a_download_sends_its_user_agent_and_referer_and_keeps_the_file() {
        let url = serve_once(|mut s, headers| {
            let ok = headers.contains("referer: https://shop.example/page")
                && headers.contains("user-agent: mozilla/5.0 (x11; linux x86_64) heronite");
            let body: &[u8] = if ok { b"the game" } else { b"wrong headers" };
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(body);
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("game.zip");
        let mut seen = Vec::new();
        download(&url, &dest, Some("https://shop.example/page"), &mut |got, total| seen.push((got, total)), &|| false).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"the game");
        assert_eq!(seen.last(), Some(&(8, Some(8))));
        assert!(!dir.path().join("game.zip.part").exists());
    }

    #[test]
    fn a_refused_download_names_the_url_without_its_query_and_the_status() {
        let url = serve_once(|mut s, _| {
            let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let dir = tempfile::tempdir().unwrap();
        let err = download(&format!("{url}?token=secret"), &dir.path().join("f"), None, &mut |_, _| {}, &|| false).unwrap_err();
        assert!(err.contains("404") && err.contains("/game.zip") && !err.contains("secret"), "{err}");
    }

    #[test]
    fn a_cancel_while_the_server_is_silent_stops_the_download() {
        let url = serve_once(|mut s, _| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n0123456789");
            std::thread::sleep(Duration::from_secs(4));
        });
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let err =
            download(&url, &dir.path().join("f"), None, &mut |_, _| {}, &|| started.elapsed() > Duration::from_millis(300)).unwrap_err();
        assert_eq!(err, crate::engine::CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn sources() {
        let files = parse_entry(&json!({"setup": "N/A:Select SPFL26_setup.exe"}));
        assert_eq!(files[0].source, Source::Ask("Select SPFL26_setup.exe".into()));
        assert_eq!(files[0].prompt(), "Select SPFL26_setup.exe");
        assert_eq!(parse_entry(&json!({"a": "N/A"}))[0].prompt(), "Select the file \"a\"");
        assert_eq!(parse_entry(&json!({"a": "https://x.org/a%20b.zip?dl=1"}))[0].file_name(), "a b.zip");
        assert_eq!(parse_entry(&json!({"a": "file:///home/u/My%20File.exe"}))[0].source, Source::Local("/home/u/My File.exe".into()));
        assert_eq!(parse_entry(&json!({"a": "$SCRIPTDIR/x.exe"}))[0].source, Source::Local("$SCRIPTDIR/x.exe".into()));
        assert!(matches!(parse_entry(&json!({"a": "$STEAM:123:x"}))[0].source, Source::Unsupported(_)));
        assert!(matches!(parse_entry(&json!({"a": "gogdownloader://x"}))[0].source, Source::Unsupported(_)));
        let f = &parse_entry(&json!({"p": {"url": "https://x.org/dl", "filename": "patch.zip", "checksum": "md5:abc"}}))[0];
        assert_eq!((f.file_name().as_str(), f.checksum.as_deref()), ("patch.zip", Some("md5:abc")));
        assert_eq!(parse_entry(&json!({"p": "https://x.org/"}))[0].file_name(), "x.org");
    }

    #[test]
    fn percent_decoding_is_lenient() {
        assert_eq!(percent_decode("a%20b%2"), "a b%2");
        assert_eq!(percent_decode("%zz%41"), "%zzA");
        assert_eq!(percent_decode("%é%41"), "%éA");
    }

    #[test]
    fn checksums() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        fs::write(&f, b"hello").unwrap();
        let no = || false;
        assert!(verify(&f, "md5:5d41402abc4b2a76b9719d911017c592", &no).is_ok());
        assert!(verify(&f, "5d41402abc4b2a76b9719d911017c592", &no).is_ok());
        assert!(verify(&f, "sha1:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d", &no).is_ok());
        assert!(verify(&f, "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", &no).is_ok());
        assert!(verify(&f, "md5:00000000000000000000000000000000", &no).is_err());
        assert!(verify(&f, "crc32:1234", &no).is_ok(), "unknown kinds are not checked");
    }
}
