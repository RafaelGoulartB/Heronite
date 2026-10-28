//! Replacing the running AppImage with a newer, signed one.
//!
//! The new file is downloaded next to the old one (same folder, so same file
//! system) under a hidden name, checked while it streams (minisign over the
//! whole file, size, AppImage magic) and only then renamed over `$APPIMAGE`.
//! Never write into the old file: the AppImage runtime of the running app
//! reads its squashfs from it, so truncating it would crash the app. After
//! the rename the running app keeps the old inode open; the next launch
//! picks the new file. The path stays the same, so menu entries and game
//! shortcuts that point at it keep working.

use heronite_i18n::tr;
use std::cell::{Cell, RefCell};
use std::ffi::{CString, OsString};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use minisign_verify::{PublicKey, Signature};

use crate::release::{Asset, Release};

/// The running AppImage, writable in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub appimage: PathBuf,
}

/// Why this process cannot update itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Not started from an AppImage nor from one of our packages (built from
    /// source, a distribution's own package).
    NotAppImage,
    /// `$APPIMAGE` does not point at a file.
    Missing(PathBuf),
    /// The folder of the AppImage is not writable by this user.
    ReadOnly(PathBuf),
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unsupported::NotAppImage => write!(f, "Heronite was not started from an AppImage"),
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
        if !writable(dir) {
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

fn writable(dir: &Path) -> bool {
    let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
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
    let sig_asset = release.signature.as_ref().ok_or_else(|| failed(tr!("This release is not signed")))?;
    let key = PublicKey::from_base64(public_key).map_err(|e| failed(tr!("Bad update key: {error}", error = e)))?;
    let signature = fetch_signature(&sig_asset.url)?;
    check_trusted_version(&signature, &release.version)?;

    let staging = target.staging();
    let _ = fs::remove_file(&staging);
    let result = download_verified(&asset.url, asset.size, &staging, &key, &signature, cancel, on_progress)
        .and_then(|()| check_appimage(&staging))
        .and_then(|()| swap_in(&staging, &target.appimage));
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

fn fetch_signature(url: &str) -> Result<Signature, InstallError> {
    let text = crate::agent()
        .get(url)
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .call()
        .map_err(crate::http_error)
        .and_then(|mut r| r.body_mut().with_config().limit(MAX_SIGNATURE_BYTES).read_to_string().map_err(|e| anyhow!("{e}")))
        .map_err(|e| failed(tr!("Could not download the signature: {error}", error = format!("{e:#}"))))?;
    Signature::decode(&text).map_err(|e| failed(tr!("Unreadable signature: {error}", error = e)))
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

/// Streams `url` into `dest` through the signature check: the file only gets
/// its name once minisign accepted every byte of it.
fn download_verified(
    url: &str,
    expected_size: u64,
    dest: &Path,
    key: &PublicKey,
    signature: &Signature,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), InstallError> {
    let verifier =
        key.verify_stream(signature).map_err(|e| failed(tr!("The signature does not match the update key: {error}", error = e)))?;
    // Fed by `on_data` while the body streams, finished by `validate` before the rename.
    let verifier = RefCell::new(verifier);
    let mismatch = Cell::new(false);
    let mut feed = |chunk: &[u8]| verifier.borrow_mut().update(chunk);
    let signed = |_: &heronite_net::Outcome| {
        verifier.borrow_mut().finalize().map_err(|_| {
            mismatch.set(true);
            "signature mismatch".to_string()
        })
    };
    let start = Instant::now();
    let mut report = |downloaded: u64, total: Option<u64>| {
        let bytes_per_sec = downloaded as f64 / start.elapsed().as_secs_f64().max(0.001);
        on_progress(Progress { downloaded, total: total.unwrap_or(0), bytes_per_sec });
    };
    let opts = heronite_net::Options {
        user_agent: concat!("heronite-rs/", env!("CARGO_PKG_VERSION")),
        size_hint: Some(expected_size).filter(|n| *n > 0),
        validate: Some(&signed),
        on_data: Some(&mut feed),
        on_progress: Some(&mut report),
        ..Default::default()
    };
    heronite_net::download(url, dest, opts, &|| cancel.load(Ordering::Relaxed)).map_err(|e| download_error(e, mismatch.get()))?;
    File::open(dest)
        .and_then(|f| f.sync_all())
        .map_err(|e| failed(tr!("Could not write {path}: {error}", path = dest.display(), error = e)))
}

fn download_error(e: heronite_net::Error, signature_mismatch: bool) -> InstallError {
    use heronite_net::Error;
    match e {
        Error::Cancelled => InstallError::Cancelled,
        _ if signature_mismatch => failed(tr!("The downloaded file does not match its signature")),
        Error::Truncated { got, total } => failed(tr!("The download stopped after {downloaded} of {total} bytes", downloaded = got, total)),
        Error::RateLimited { .. } => failed(tr!(
            "Download failed: {error}",
            error = tr!("GitHub rate limit reached (HTTP {status}), try again later", status = "403/429")
        )),
        Error::Http(code) => {
            failed(tr!("Download failed: {error}", error = format!("{:#}", crate::http_error(ureq::Error::StatusCode(code)))))
        }
        e => failed(tr!("Download failed: {error}", error = e)),
    }
}

/// Downloads `asset` into `dir` (as `dir/<name>`, through a hidden partial
/// file) and checks it against `signature` like [`install`] does. Returns the
/// file. Used for the system packages, which their package manager installs.
pub fn download_signed(
    asset: &Asset,
    signature: &Asset,
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
    let signature = fetch_signature(&signature.url)?;
    check_trusted_version(&signature, version)?;
    fs::create_dir_all(dir).map_err(|e| failed(tr!("Could not create {path}: {error}", path = dir.display(), error = e)))?;
    let dest = dir.join(&asset.name);
    download_verified(&asset.url, asset.size, &dest, &key, &signature, cancel, on_progress)?;
    Ok(dest)
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
    let mode = fs::metadata(appimage).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o755) | 0o700;
    fs::set_permissions(staging, fs::Permissions::from_mode(mode))
        .map_err(|e| failed(tr!("Could not make the update executable: {error}", error = e)))?;
    fs::rename(staging, appimage).map_err(|e| failed(tr!("Could not replace {path}: {error}", path = appimage.display(), error = e)))?;
    if let Some(dir) = appimage.parent() {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// Test key and signatures of [`blob`] made with `minisign -S -t
    /// "version:<v> file:test.AppImage"`. Not the release key.
    const TEST_KEY: &str = "RWQrG3DRkOljn+WcHNIkIS0MboE2BJk4FRyrB6GkIvasTo+w/iXW1BLu";
    const SIG_014: &str = include_str!("../tests/fixtures/blob-0.1.4.minisig");
    const SIG_012: &str = include_str!("../tests/fixtures/blob-0.1.2.minisig");

    /// The signed test file: an AppImage header and a fixed pattern.
    fn blob() -> Vec<u8> {
        let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02".to_vec();
        b.extend((0u32..300_000).map(|i| (i.wrapping_mul(7) % 251) as u8));
        b
    }

    /// Serves `files` (path -> bytes) over HTTP on a random local port.
    fn serve(files: Vec<(String, Vec<u8>)>, chunk_delay_ms: u64) -> String {
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
        }
    }

    /// An "installed" AppImage in a temp folder.
    fn installed(dir: &Path) -> Target {
        let path = dir.join("Heronite-0.1.3-x86_64.AppImage");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
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
        assert_eq!(fs::metadata(&target.appimage).unwrap().permissions().mode() & 0o777, 0o750);
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
        let sig = asset("heronite.deb.minisig");
        let no = AtomicBool::new(false);
        let dir = tmp.path().join("updates");
        let file = download_signed(&asset("heronite.deb"), &sig, "0.1.4", &dir, TEST_KEY, &no, &mut |_| {}).unwrap();
        assert_eq!(file, dir.join("heronite.deb"));
        assert_eq!(fs::read(&file).unwrap(), data);
        let err = download_signed(&asset("bad.deb"), &sig, "0.1.4", &dir, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
        assert!(!dir.join("bad.deb").exists() && !dir.join("bad.deb.part").exists());
        let err = download_signed(&asset("../x.deb"), &sig, "0.1.4", &dir, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("Bad file name"));
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
        let part = tmp.path().join(".Heronite.AppImage.update.part");
        fs::write(t.staging(), b"partial").unwrap();
        fs::write(&part, b"partial").unwrap();
        t.sweep_staging();
        assert!(!t.staging().exists() && !part.exists());
    }
}
