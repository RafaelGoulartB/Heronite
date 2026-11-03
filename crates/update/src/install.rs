//! Replacing the running AppImage with a newer, signed one.
//!
//! The new file is downloaded next to the old one (same folder, so same file
//! system) under a hidden name, checked while it streams (its SHA-256 in the
//! release's signed `SHA256SUMS`, or minisign over the whole file; size,
//! AppImage magic) and only then renamed over `$APPIMAGE`.
//! Never write into the old file: the AppImage runtime of the running app
//! reads its squashfs from it, so truncating it would crash the app. After
//! the rename the running app keeps the old inode open; the next launch
//! picks the new file. The path stays the same, so menu entries and game
//! shortcuts that point at it keep working.

use heronite_i18n::tr;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use minisign_verify::{PublicKey, Signature, StreamVerifier};
use sha2::{Digest, Sha256};

use crate::release::{Asset, Checksums, Proof, Release};

/// The running AppImage, writable in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub appimage: PathBuf,
}

/// Why this process cannot update itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Not started from an AppImage nor from one of our packages (built from
    /// source, a distribution's own package). On Windows: neither installed
    /// by our setup nor a portable copy (a build folder).
    NotAppImage,
    /// A Flatpak: Flatpak installs its updates.
    Flatpak,
    /// `$APPIMAGE` does not point at a file.
    Missing(PathBuf),
    /// The folder of the AppImage (of the portable copy on Windows) is not
    /// writable by this user.
    ReadOnly(PathBuf),
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unsupported::NotAppImage if cfg!(windows) => {
                write!(f, "Heronite was neither installed by its setup nor unpacked as a portable copy")
            }
            Unsupported::NotAppImage => write!(f, "Heronite was not started from an AppImage"),
            Unsupported::Flatpak => write!(f, "Heronite runs as a Flatpak, which Flatpak updates"),
            Unsupported::Missing(p) => write!(f, "the AppImage {} was not found", p.display()),
            Unsupported::ReadOnly(dir) => write!(f, "{} is not writable", dir.display()),
        }
    }
}

impl Target {
    /// The AppImage this process runs from: `appimage` is the `$APPIMAGE` the
    /// AppImage runtime sets.
    pub fn detect(appimage: Option<OsString>) -> Result<Self, Unsupported> {
        let raw = PathBuf::from(appimage.filter(|v| !v.is_empty()).ok_or(Unsupported::NotAppImage)?);
        // A symlink would be replaced by the file instead of updating its target.
        let path = fs::canonicalize(&raw).map_err(|_| Unsupported::Missing(raw.clone()))?;
        if !path.is_file() {
            return Err(Unsupported::Missing(raw));
        }
        let dir = path.parent().ok_or_else(|| Unsupported::Missing(raw.clone()))?;
        if !heronite_platform::fs::writable_dir(dir) {
            return Err(Unsupported::ReadOnly(dir.to_path_buf()));
        }
        Ok(Self { appimage: path })
    }

    /// The hidden file a download goes to, next to the AppImage.
    pub fn staging(&self) -> PathBuf {
        let name = self.appimage.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.appimage.with_file_name(format!(".{name}.update"))
    }

    /// Removes what an interrupted download left (a quit, a crash).
    pub fn sweep_staging(&self) {
        let staging = self.staging();
        if fs::remove_file(&staging).is_ok() {
            log::info!("removed the unfinished update {}", staging.display());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    pub downloaded: u64,
    /// 0 = unknown.
    pub total: u64,
    pub bytes_per_sec: f64,
}

impl Progress {
    /// 0..=100, `None` while the size is unknown.
    pub fn percent(&self) -> Option<f32> {
        (self.total > 0).then(|| (self.downloaded as f64 / self.total as f64 * 100.).min(100.) as f32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// Cancelled by the user; nothing was changed.
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Cancelled => write!(f, "The update was cancelled"),
            InstallError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for InstallError {}

fn failed(m: impl std::fmt::Display) -> InstallError {
    InstallError::Failed(m.to_string())
}

/// The largest signature file read.
const MAX_SIGNATURE_BYTES: u64 = 16 * 1024;
/// The largest `SHA256SUMS` read (a line per file of the release).
const MAX_CHECKSUMS_BYTES: u64 = 64 * 1024;

/// What a download must match, from its [`Proof`].
enum Expected {
    /// Its SHA-256, from the signed checksums.
    Sha256([u8; 32]),
    /// Its own signature.
    Signature(Signature),
}

/// What `asset` of the release `version` must match: its line of the
/// checksums once their signature, made with `key` for `version`, checks
/// out; or its own signature, made for `version`.
fn expected(asset: &Asset, proof: &Proof, version: &str, key: &PublicKey) -> Result<Expected, InstallError> {
    match proof {
        Proof::Signature(sig) => {
            let signature = fetch_signature(&sig.url)?;
            check_trusted_version(&signature, version)?;
            Ok(Expected::Signature(signature))
        }
        Proof::Checksums(Checksums { list, signature }) => {
            let signature = fetch_signature(&signature.url)?;
            check_trusted_version(&signature, version)?;
            let text = fetch_text(&list.url, MAX_CHECKSUMS_BYTES)
                .map_err(|e| failed(tr!("Could not download the checksums: {error}", error = format!("{e:#}"))))?;
            key.verify(text.as_bytes(), &signature, false).map_err(|_| failed(tr!("The checksums do not match their signature")))?;
            checksum_of(&text, &asset.name)
                .map(Expected::Sha256)
                .ok_or_else(|| failed(tr!("The checksums do not list {file}", file = asset.name.as_str())))
        }
    }
}

/// The SHA-256 `sha256sum` lists for `name` (`<hex>  <name>`, or
/// `<hex> *<name>` for its binary mode).
fn checksum_of(text: &str, name: &str) -> Option<[u8; 32]> {
    text.lines().find_map(|line| {
        let (hex, file) = line.trim_end().split_once(' ')?;
        let file = file.trim_start_matches(' ');
        (file.strip_prefix('*').unwrap_or(file) == name).then(|| parse_sha256(hex)).flatten()
    })
}

fn parse_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Downloads `release` and puts it in place of `target`. `public_key` is
/// minisign's base64 key ([`crate::PUBLIC_KEY`]). Nothing changes unless the
/// whole file arrived and its signature, trusted version and format check out.
pub fn install(
    release: &Release,
    target: &Target,
    public_key: &str,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), InstallError> {
    let asset = release.appimage.as_ref().ok_or_else(|| failed(tr!("This release has no AppImage")))?;
    let proof = release.appimage_proof().ok_or_else(|| failed(tr!("This release is not signed")))?;
    let key = PublicKey::from_base64(public_key).map_err(|e| failed(tr!("Bad update key: {error}", error = e)))?;
    let expected = expected(asset, &proof, &release.version, &key)?;

    let staging = target.staging();
    let _ = fs::remove_file(&staging);
    let result = download_verified(&asset.url, asset.size, &staging, &key, &expected, cancel, on_progress)
        .and_then(|()| check_appimage(&staging))
        .and_then(|()| swap_in(&staging, &target.appimage));
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

/// A small text file, at most `limit` bytes.
fn fetch_text(url: &str, limit: u64) -> anyhow::Result<String> {
    crate::agent()
        .get(url)
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .call()
        .map_err(crate::http_error)
        .and_then(|mut r| r.body_mut().with_config().limit(limit).read_to_string().map_err(|e| anyhow!("{e}")))
}

fn fetch_signature(url: &str) -> Result<Signature, InstallError> {
    let text = fetch_text(url, MAX_SIGNATURE_BYTES)
        .map_err(|e| failed(tr!("Could not download the signature: {error}", error = format!("{e:#}"))))?;
    Signature::decode(&text).map_err(|e| failed(tr!("Unreadable signature: {error}", error = e)))
}

/// Checks a download as it streams, against what [`expected`] found.
enum Verifier<'a> {
    Sha256(Sha256, &'a [u8; 32]),
    Signature(Box<StreamVerifier<'a>>),
}

impl<'a> Verifier<'a> {
    fn new(key: &'a PublicKey, expected: &'a Expected) -> Result<Self, InstallError> {
        Ok(match expected {
            Expected::Sha256(hash) => Verifier::Sha256(Sha256::new(), hash),
            Expected::Signature(signature) => Verifier::Signature(Box::new(
                key.verify_stream(signature).map_err(|e| failed(tr!("The signature does not match the update key: {error}", error = e)))?,
            )),
        })
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Verifier::Sha256(hasher, _) => hasher.update(data),
            Verifier::Signature(verifier) => verifier.update(data),
        }
    }

    fn finalize(self) -> Result<(), InstallError> {
        let matches = match self {
            Verifier::Sha256(hasher, hash) => hasher.finalize().as_slice() == hash.as_slice(),
            Verifier::Signature(mut verifier) => verifier.finalize().is_ok(),
        };
        matches.then_some(()).ok_or_else(|| failed(tr!("The downloaded file does not match its signature")))
    }
}

/// The signed comment must name the version the release claims, or an old
/// signed build could be offered as a new one (and installed as a downgrade).
fn check_trusted_version(signature: &Signature, version: &str) -> Result<(), InstallError> {
    let wanted = format!("version:{version}");
    if signature.trusted_comment().split_whitespace().any(|t| t == wanted) {
        Ok(())
    } else {
        Err(failed(tr!("The signature is for another version ({version})", version = format!("{:?}", signature.trusted_comment()))))
    }
}

fn download_verified(
    url: &str,
    expected_size: u64,
    dest: &Path,
    key: &PublicKey,
    expected: &Expected,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), InstallError> {
    let mut verifier = Verifier::new(key, expected)?;
    let mut resp = crate::agent()
        .get(url)
        .call()
        .map_err(|e| failed(tr!("Download failed: {error}", error = format!("{:#}", crate::http_error(e)))))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(expected_size);
    let mut body = resp.body_mut().with_config().limit(u64::MAX).reader();
    let mut file = File::create(dest).map_err(|e| failed(tr!("Could not create {path}: {error}", path = dest.display(), error = e)))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded = 0u64;
    let start = Instant::now();
    let mut last = Instant::now() - Duration::from_secs(1);
    on_progress(Progress { downloaded: 0, total, bytes_per_sec: 0. });
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        let n = body.read(&mut buf).map_err(|e| failed(tr!("Download failed: {error}", error = e)))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| failed(tr!("Could not write {path}: {error}", path = dest.display(), error = e)))?;
        verifier.update(&buf[..n]);
        downloaded += n as u64;
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            let bytes_per_sec = downloaded as f64 / start.elapsed().as_secs_f64().max(0.001);
            on_progress(Progress { downloaded, total, bytes_per_sec });
        }
    }
    if total > 0 && downloaded != total {
        return Err(failed(tr!("The download stopped after {downloaded} of {total} bytes", downloaded, total)));
    }
    verifier.finalize()?;
    file.sync_all().map_err(|e| failed(tr!("Could not write {path}: {error}", path = dest.display(), error = e)))?;
    on_progress(Progress { downloaded, total: downloaded, bytes_per_sec: 0. });
    Ok(())
}

/// Downloads `asset` into `dir` (as `dir/<name>`, through a hidden partial
/// file) and checks it against `proof` like [`install`] does. Returns the
/// file. Used for the system packages, which their package manager installs.
pub fn download_signed(
    asset: &Asset,
    proof: &Proof,
    version: &str,
    dir: &Path,
    public_key: &str,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf, InstallError> {
    if asset.name.is_empty() || asset.name.contains('/') || asset.name.starts_with('.') {
        return Err(failed(format!("Bad file name {:?}", asset.name)));
    }
    let key = PublicKey::from_base64(public_key).map_err(|e| failed(tr!("Bad update key: {error}", error = e)))?;
    let expected = expected(asset, proof, version, &key)?;
    fs::create_dir_all(dir).map_err(|e| failed(tr!("Could not create {path}: {error}", path = dir.display(), error = e)))?;
    let dest = dir.join(&asset.name);
    let part = dir.join(format!(".{}.part", asset.name));
    let _ = fs::remove_file(&part);
    let result = download_verified(&asset.url, asset.size, &part, &key, &expected, cancel, on_progress).and_then(|()| {
        fs::rename(&part, &dest).map_err(|e| failed(tr!("Could not write {path}: {error}", path = dest.display(), error = e)))
    });
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result.map(|()| dest)
}

/// An ELF file with the type 2 AppImage magic (`AI\x02` at offset 8).
fn check_appimage(path: &Path) -> Result<(), InstallError> {
    let mut head = [0u8; 11];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .map_err(|e| failed(tr!("Could not read the download: {error}", error = e)))?;
    if &head[..4] == b"\x7fELF" && &head[8..11] == b"AI\x02" {
        Ok(())
    } else {
        Err(failed(tr!("The download is not an AppImage")))
    }
}

/// Gives `staging` the old file's permissions (and the owner's execute bit),
/// then renames it over `appimage` and syncs the folder.
fn swap_in(staging: &Path, appimage: &Path) -> Result<(), InstallError> {
    let mode = fs::metadata(appimage).ok().and_then(|m| heronite_platform::fs::mode(&m)).unwrap_or(0o755) | 0o700;
    heronite_platform::fs::set_mode(staging, mode).map_err(|e| failed(tr!("Could not make the update executable: {error}", error = e)))?;
    fs::rename(staging, appimage).map_err(|e| failed(tr!("Could not replace {path}: {error}", path = appimage.display(), error = e)))?;
    if let Some(dir) = appimage.parent() {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// Test key and signatures of [`blob`] made with `minisign -S -t
    /// "version:<v> file:test.AppImage"`. Not the release key.
    pub(crate) const TEST_KEY: &str = "RWQrG3DRkOljn+WcHNIkIS0MboE2BJk4FRyrB6GkIvasTo+w/iXW1BLu";
    pub(crate) const SIG_014: &str = include_str!("../tests/fixtures/blob-0.1.4.minisig");
    const SIG_012: &str = include_str!("../tests/fixtures/blob-0.1.2.minisig");
    /// A second test key, which signed `SHA256SUMS-0.1.4` (the SHA-256 of
    /// [`blob`] as `test.AppImage` and `heronite.deb`) with `minisign -S -t
    /// "version:0.1.4 file:SHA256SUMS"`. Not the release key.
    const SUMS_KEY: &str = "RWRcOm8haq8yg+KXgDnqxn6yIw8XFIwPJZG/ONHDETQqiyO82Tqf5hHV";
    const SUMS_014: &str = include_str!("../tests/fixtures/SHA256SUMS-0.1.4");
    const SUMS_014_SIG: &str = include_str!("../tests/fixtures/SHA256SUMS-0.1.4.minisig");

    /// The signed test file: an AppImage header and a fixed pattern.
    pub(crate) fn blob() -> Vec<u8> {
        let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02".to_vec();
        b.extend((0u32..300_000).map(|i| (i.wrapping_mul(7) % 251) as u8));
        b
    }

    /// Serves `files` (path -> bytes) over HTTP on a random local port.
    pub(crate) fn serve(files: Vec<(String, Vec<u8>)>, chunk_delay_ms: u64) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let files = Arc::new(files);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    loop {
                        let mut h = String::new();
                        reader.read_line(&mut h).unwrap();
                        if h == "\r\n" || h.is_empty() {
                            break;
                        }
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    match files.iter().find(|(p, _)| *p == path) {
                        Some((_, body)) => {
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            for c in body.chunks(32 * 1024) {
                                if stream.write_all(c).is_err() {
                                    return;
                                }
                                if chunk_delay_ms > 0 {
                                    std::thread::sleep(Duration::from_millis(chunk_delay_ms));
                                }
                            }
                        }
                        None => {
                            let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    fn release(base: &str, version: &str, size: u64) -> Release {
        Release {
            version: version.into(),
            name: format!("Heronite v{version}"),
            notes: String::new(),
            page_url: String::new(),
            published_at: String::new(),
            appimage: Some(Asset { name: "test.AppImage".into(), url: format!("{base}/test.AppImage"), size }),
            signature: Some(Asset { name: "test.AppImage.minisig".into(), url: format!("{base}/test.AppImage.minisig"), size: 0 }),
            packages: Vec::new(),
            windows: Vec::new(),
            checksums: None,
        }
    }

    /// The release at `base` proven by its signed checksums only.
    fn listed(base: &str, version: &str) -> Release {
        let file = |name: &str| Asset { name: name.into(), url: format!("{base}/{name}"), size: 0 };
        Release {
            signature: None,
            checksums: Some(Checksums { list: file("SHA256SUMS"), signature: file("SHA256SUMS.minisig") }),
            ..release(base, version, 0)
        }
    }

    /// An "installed" AppImage in a temp folder.
    fn installed(dir: &Path) -> Target {
        let path = dir.join("Heronite-0.1.3-x86_64.AppImage");
        fs::write(&path, b"old").unwrap();
        heronite_platform::fs::set_mode(&path, 0o750).unwrap();
        Target::detect(Some(path.into_os_string())).unwrap()
    }

    #[test]
    fn installs_a_signed_update_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let target = installed(tmp.path());
        let data = blob();
        let base = serve(vec![("/test.AppImage".into(), data.clone()), ("/test.AppImage.minisig".into(), SIG_014.into())], 0);
        let mut last = Progress::default();
        install(&release(&base, "0.1.4", data.len() as u64), &target, TEST_KEY, &AtomicBool::new(false), &mut |p| last = p).unwrap();
        assert_eq!(fs::read(&target.appimage).unwrap(), data);
        #[cfg(unix)]
        assert_eq!(heronite_platform::fs::mode(&fs::metadata(&target.appimage).unwrap()).unwrap() & 0o777, 0o750);
        assert_eq!(last.downloaded, data.len() as u64);
        assert_eq!(last.percent(), Some(100.));
        assert!(!target.staging().exists());
    }

    #[test]
    fn refuses_tampered_files_wrong_versions_and_other_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let target = installed(tmp.path());
        let data = blob();
        let mut tampered = data.clone();
        tampered[5000] ^= 1;
        let base = serve(
            vec![
                ("/test.AppImage".into(), tampered),
                ("/test.AppImage.minisig".into(), SIG_014.into()),
                ("/good/test.AppImage".into(), data.clone()),
                ("/good/test.AppImage.minisig".into(), SIG_012.into()),
            ],
            0,
        );
        let no = AtomicBool::new(false);
        let size = data.len() as u64;
        let err = install(&release(&base, "0.1.4", size), &target, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("does not match its signature"), "{err}");
        // A genuine file signed for 0.1.2 offered as 0.1.4.
        let err = install(&release(&format!("{base}/good"), "0.1.4", size), &target, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("another version"), "{err}");
        let err = install(&release(&format!("{base}/good"), "0.1.2", size), &target, crate::PUBLIC_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("update key"), "{err}");
        assert_eq!(fs::read(&target.appimage).unwrap(), b"old");
        assert!(!target.staging().exists());
    }

    #[test]
    fn downloads_a_signed_package_and_refuses_a_bad_one() {
        let tmp = tempfile::tempdir().unwrap();
        let data = blob();
        let mut tampered = data.clone();
        tampered[10] ^= 1;
        let base = serve(
            vec![("/heronite.deb".into(), data.clone()), ("/heronite.deb.minisig".into(), SIG_014.into()), ("/bad.deb".into(), tampered)],
            0,
        );
        let asset = |name: &str| Asset { name: name.into(), url: format!("{base}/{name}"), size: 0 };
        let sig = Proof::Signature(asset("heronite.deb.minisig"));
        let no = AtomicBool::new(false);
        let dir = tmp.path().join("updates");
        let file = download_signed(&asset("heronite.deb"), &sig, "0.1.4", &dir, TEST_KEY, &no, &mut |_| {}).unwrap();
        assert_eq!(file, dir.join("heronite.deb"));
        assert_eq!(fs::read(&file).unwrap(), data);
        let err = download_signed(&asset("bad.deb"), &sig, "0.1.4", &dir, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
        assert!(!dir.join("bad.deb").exists() && !dir.join(".bad.deb.part").exists());
        let err = download_signed(&asset("../x.deb"), &sig, "0.1.4", &dir, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("Bad file name"));
    }

    #[test]
    fn installs_an_update_its_signed_checksums_vouch_for() {
        let tmp = tempfile::tempdir().unwrap();
        let target = installed(tmp.path());
        let data = blob();
        let base = serve(
            vec![
                ("/test.AppImage".into(), data.clone()),
                ("/SHA256SUMS".into(), SUMS_014.into()),
                ("/SHA256SUMS.minisig".into(), SUMS_014_SIG.into()),
            ],
            0,
        );
        install(&listed(&base, "0.1.4"), &target, SUMS_KEY, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(fs::read(&target.appimage).unwrap(), data);
        // The packages and the Windows files the same way.
        let dir = tmp.path().join("updates");
        let deb = Asset { name: "heronite.deb".into(), url: format!("{base}/test.AppImage"), size: 0 };
        let proof = listed(&base, "0.1.4").appimage_proof().unwrap();
        let file = download_signed(&deb, &proof, "0.1.4", &dir, SUMS_KEY, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(fs::read(file).unwrap(), data);
    }

    #[test]
    fn refuses_what_the_signed_checksums_do_not_vouch_for() {
        let tmp = tempfile::tempdir().unwrap();
        let target = installed(tmp.path());
        let data = blob();
        let mut tampered = data.clone();
        tampered[5000] ^= 1;
        let edited = SUMS_014.replacen("ee90", "ee91", 1);
        let serve_with = |file: Vec<u8>, sums: &str| {
            serve(
                vec![
                    ("/test.AppImage".into(), file),
                    ("/SHA256SUMS".into(), sums.into()),
                    ("/SHA256SUMS.minisig".into(), SUMS_014_SIG.into()),
                ],
                0,
            )
        };
        let no = AtomicBool::new(false);
        let try_install =
            |base: &str, version: &str, key: &str| install(&listed(base, version), &target, key, &no, &mut |_| {}).unwrap_err().to_string();

        let err = try_install(&serve_with(tampered, SUMS_014), "0.1.4", SUMS_KEY);
        assert!(err.contains("does not match its signature"), "{err}");
        let err = try_install(&serve_with(data.clone(), &edited), "0.1.4", SUMS_KEY);
        assert!(err.contains("checksums do not match their signature"), "{err}");
        // Genuine checksums of 0.1.4 offered as 0.1.5.
        let err = try_install(&serve_with(data.clone(), SUMS_014), "0.1.5", SUMS_KEY);
        assert!(err.contains("another version"), "{err}");
        let err = try_install(&serve_with(data.clone(), SUMS_014), "0.1.4", TEST_KEY);
        assert!(err.contains("checksums do not match their signature"), "{err}");
        // A file the list does not name.
        let base = serve_with(data, SUMS_014);
        let other = Asset { name: "heronite.rpm".into(), url: format!("{base}/test.AppImage"), size: 0 };
        let proof = listed(&base, "0.1.4").appimage_proof().unwrap();
        let dir = tmp.path().join("updates");
        let err = download_signed(&other, &proof, "0.1.4", &dir, SUMS_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("do not list heronite.rpm"), "{err}");

        assert_eq!(fs::read(&target.appimage).unwrap(), b"old");
        assert!(!target.staging().exists());
    }

    #[test]
    fn reads_sha256sum_lines() {
        let hash = "ee90ba26bf17c62d3806805a01554132522335e733b4d8f68e6e41bba4facd11";
        let text = format!("{hash}  Heronite-x86_64.AppImage\n{hash} *heronite_1.1.0-1_amd64.deb\nnot a line\n");
        let bytes = parse_sha256(hash).unwrap();
        assert_eq!((bytes[0], bytes[31]), (0xee, 0x11));
        assert_eq!(checksum_of(&text, "Heronite-x86_64.AppImage"), Some(bytes));
        assert_eq!(checksum_of(&text, "heronite_1.1.0-1_amd64.deb"), Some(bytes), "binary mode");
        assert_eq!(checksum_of(&text, "Heronite-x86_64.flatpak"), None);
        assert_eq!(parse_sha256("ee90"), None);
        assert_eq!(parse_sha256(&"zz".repeat(32)), None);
    }

    #[test]
    fn cancel_leaves_the_old_file() {
        let tmp = tempfile::tempdir().unwrap();
        let data = blob();
        let base = serve(vec![("/test.AppImage".into(), data.clone()), ("/test.AppImage.minisig".into(), SIG_014.into())], 20);
        let target = installed(tmp.path());
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let err = install(&release(&base, "0.1.4", data.len() as u64), &target, TEST_KEY, &cancel, &mut |p| {
            if p.downloaded > 0 {
                flag.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
        assert_eq!(err, InstallError::Cancelled);
        assert_eq!(fs::read(&target.appimage).unwrap(), b"old");
        assert!(!target.staging().exists());
    }

    #[test]
    fn missing_signature_or_non_appimage_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let target = installed(tmp.path());
        let mut unsigned = release("http://127.0.0.1:9", "0.1.4", 0);
        unsigned.signature = None;
        let err = install(&unsigned, &target, TEST_KEY, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("not signed"));

        let plain = tmp.path().join("plain");
        fs::write(&plain, b"#!/bin/sh\necho hi\n").unwrap();
        assert!(check_appimage(&plain).is_err());
        fs::write(&plain, &blob()[..64]).unwrap();
        assert!(check_appimage(&plain).is_ok());
    }

    // Windows makes symlinks only in Developer Mode or as an administrator.
    #[cfg(unix)]
    #[test]
    fn detect_needs_an_existing_file_in_a_writable_folder() {
        assert_eq!(Target::detect(None), Err(Unsupported::NotAppImage));
        assert_eq!(Target::detect(Some(OsString::new())), Err(Unsupported::NotAppImage));
        assert!(matches!(Target::detect(Some("/nonexistent/x.AppImage".into())), Err(Unsupported::Missing(_))));
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("Heronite.AppImage");
        fs::write(&real, b"x").unwrap();
        let link = tmp.path().join("link.AppImage");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let t = Target::detect(Some(link.into_os_string())).unwrap();
        assert_eq!(t.appimage, fs::canonicalize(&real).unwrap());
        assert_eq!(t.staging().file_name().unwrap(), ".Heronite.AppImage.update");
        fs::write(t.staging(), b"partial").unwrap();
        t.sweep_staging();
        assert!(!t.staging().exists());
    }
}
