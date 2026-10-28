//! Download, verification, extraction and removal of a Wine/Proton build
//! (port of `downloader/main.ts::installVersion`, `utils.ts::installWineVersion`
//! / `removeWineVersion` and `getInstallDir`).

use heronite_i18n::tr;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};
use heronite_data::proc::Run;
use heronite_data::DataPaths;

use crate::github;
use crate::store;
use crate::types::{Progress, WineConfig, WineVersionInfo};

#[derive(Debug)]
pub enum InstallError {
    /// The user cancelled; everything this install created was cleaned up.
    Aborted,
    Failed(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Aborted => write!(f, "Installation was aborted"),
            InstallError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<anyhow::Error> for InstallError {
    fn from(e: anyhow::Error) -> Self {
        InstallError::Failed(format!("{e:#}"))
    }
}

/// Folder the build is installed into (`getInstallDir`): wine builds in
/// `tools/wine`, GPTK in `tools/game-porting-toolkit`, Proton in
/// `tools/proton` or Steam's `compatibilitytools.d` when
/// `downloadProtonToSteam` is on and the Steam path is valid.
pub fn install_dir_for(data_paths: &DataPaths, cfg: &WineConfig, info: &WineVersionInfo) -> PathBuf {
    let tools = data_paths.tools_dir();
    if info.kind.contains("Wine") {
        return tools.join("wine");
    }
    if info.kind.contains("Toolkit") {
        return tools.join("game-porting-toolkit");
    }
    if cfg.download_proton_to_steam && !cfg.default_steam_path.is_empty() {
        let steam = Path::new(&cfg.default_steam_path);
        if steam.join("steam.sh").exists() {
            return steam.join("compatibilitytools.d");
        }
        log::warn!(
            "Configured Steam path (\"{}\") does not appear to be valid, installing into the tools folder instead",
            cfg.default_steam_path
        );
    }
    tools.join("proton")
}

/// Full install into `install_dir` and update of the release cache. Blocking.
pub fn install_version(
    data_paths: &DataPaths,
    cfg: &WineConfig,
    info: &WineVersionInfo,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<WineVersionInfo, InstallError> {
    let install_dir = install_dir_for(data_paths, cfg, info);
    let overwrite = info.has_update();
    let updated = install_into(info, &install_dir, overwrite, cancel, on_progress)?;
    store::mark_installed(data_paths, &updated).map_err(InstallError::from)?;
    Ok(updated)
}

/// The part of the install that does not depend on Heroic's folders.
pub fn install_into(
    info: &WineVersionInfo,
    install_dir: &Path,
    overwrite: bool,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<WineVersionInfo, InstallError> {
    let download = info
        .download
        .as_deref()
        .filter(|d| !d.is_empty())
        .ok_or_else(|| InstallError::Failed(tr!("No download link provided for {version}!", version = info.version)))?;
    if !is_safe_name(&info.version) {
        return Err(InstallError::Failed(format!("Invalid version name {:?}", info.version)));
    }
    fs::create_dir_all(install_dir).with_context(|| format!("creating {}", install_dir.display()))?;
    if !install_dir.is_dir() {
        return Err(InstallError::Failed(tr!("Installation directory {path} is not a directory!", path = install_dir.display())));
    }

    let file_name = download.rsplit('/').next().unwrap_or("download.tar");
    let tar_file = install_dir.join(file_name);
    let install_sub_dir = match info.install_dir.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => install_dir.join(&info.version),
    };

    // Source checksum (a `.sha512sum` or `.sha256sum` file next to the archive).
    let source_checksum = match info.checksum.as_deref().filter(|c| !c.is_empty()) {
        Some(url) => Some(
            github::http_get_text(url, 1024 * 1024)
                .map_err(|e| InstallError::Failed(tr!("Could not fetch the checksum: {error}", error = e)))?,
        ),
        None => None,
    };

    if install_sub_dir.exists() && !overwrite {
        log::warn!("{} is already installed, skipping", info.version);
        let mut done = info.clone();
        done.disksize = folder_size(&install_sub_dir);
        done.install_dir = Some(install_sub_dir.to_string_lossy().into_owned());
        done.is_installed = Some(true);
        done.has_update = Some(false);
        return Ok(done);
    }

    let _ = fs::remove_file(&tar_file);

    // Download; the archive only gets its name once the checksum matched.
    let algorithm = source_checksum.as_deref().map(HashKind::of_checksum).unwrap_or(HashKind::Sha512);
    if source_checksum.is_none() {
        log::warn!("No checksum provided. Download of {} could be invalid!", info.version);
    }
    let mismatch = Cell::new(false);
    let matches_checksum = |o: &heronite_net::Outcome| {
        let digest = o.hash.as_deref().unwrap_or_default();
        match &source_checksum {
            Some(sum) if !checksum_matches(sum, digest, file_name) => {
                mismatch.set(true);
                Err("checksum mismatch".to_string())
            }
            _ => Ok(()),
        }
    };
    if let Err(e) = download_file(download, &tar_file, info.downsize.unwrap_or(0), algorithm, &matches_checksum, cancel, on_progress) {
        return Err(match e {
            heronite_net::Error::Cancelled => InstallError::Aborted,
            _ if mismatch.get() => InstallError::Failed(tr!("Checksum verification failed").into()),
            e => {
                let error = match e {
                    heronite_net::Error::Truncated { got, total } => {
                        tr!("connection closed after {downloaded} of {total} bytes", downloaded = got, total)
                    }
                    e => e.to_string(),
                };
                InstallError::Failed(tr!("Download of {version} failed with:\n {error}", version = info.version, error))
            }
        });
    }

    // Extract next to the final folder under a hidden name and rename when
    // done, so a crash or cancel never leaves a half-extracted install (and an
    // update keeps the old build until the new one is complete).
    let parent = install_sub_dir.parent().unwrap_or(install_dir);
    let tmp_dir = parent.join(format!(".{}.extracting", info.version));
    let _ = fs::remove_dir_all(&tmp_dir);
    if let Err(e) = fs::create_dir_all(&tmp_dir) {
        let _ = fs::remove_file(&tar_file);
        return Err(InstallError::Failed(tr!("Failed to make folder {path} with:\n {error}", path = tmp_dir.display(), error = e)));
    }

    on_progress(Progress::Unzipping);
    let extracted = extract_tar(&tar_file, &tmp_dir, 1, cancel);
    let _ = fs::remove_file(&tar_file);
    if let Err(e) = extracted {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(match e {
            InstallError::Aborted => InstallError::Aborted,
            InstallError::Failed(m) => InstallError::Failed(tr!("Unzip of {file_name} failed with:\n {error}", file_name, error = m)),
        });
    }

    // Swap in (old build moved to `_backup` first when updating).
    let backup = PathBuf::from(format!("{}_backup", install_sub_dir.display()));
    let mut backed_up = false;
    if install_sub_dir.exists() {
        // `overwrite` (update) or an empty/stale leftover folder.
        let _ = fs::remove_dir_all(&backup);
        if let Err(e) = fs::rename(&install_sub_dir, &backup) {
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(InstallError::Failed(tr!("Could not back up {path}: {error}", path = install_sub_dir.display(), error = e)));
        }
        backed_up = true;
    }
    if let Err(e) = fs::rename(&tmp_dir, &install_sub_dir) {
        let _ = fs::remove_dir_all(&tmp_dir);
        if backed_up {
            let _ = fs::rename(&backup, &install_sub_dir);
        }
        return Err(InstallError::Failed(tr!(
            "Could not move the build into {path}: {error}",
            path = install_sub_dir.display(),
            error = e
        )));
    }
    if backed_up {
        let _ = fs::remove_dir_all(&backup);
    }

    let mut done = info.clone();
    done.disksize = folder_size(&install_sub_dir);
    done.install_dir = Some(install_sub_dir.to_string_lossy().into_owned());
    done.is_installed = Some(true);
    done.has_update = Some(false);
    Ok(done)
}

/// Versions become folder names: no separators or traversal.
fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

/// Whether `checksums` (the text of a `.sha256sum` / `.sha512sum` file)
/// lists `digest` for `file_name`. A digest must be a whole token: a longer
/// hex string that merely contains it, or the digest of another file in a
/// multi-line list, does not count. A file with one line needs no name.
fn checksum_matches(checksums: &str, digest: &str, file_name: &str) -> bool {
    if digest.is_empty() {
        return false;
    }
    let lines: Vec<&str> = checksums.lines().filter(|l| !l.trim().is_empty()).collect();
    // `<hex>  <name>`, `<hex> *<name>` and BSD's `SHA256 (<name>) = <hex>`.
    let tokens = |line: &str| -> Vec<String> {
        line.split_whitespace().map(|t| t.trim_matches(|c| c == '*' || c == '(' || c == ')').to_ascii_lowercase()).collect()
    };
    let names_file = |tokens: &[String]| {
        let name = file_name.to_ascii_lowercase();
        tokens.iter().any(|t| *t == name || t.rsplit('/').next() == Some(name.as_str()))
    };
    match lines.as_slice() {
        [only] => tokens(only).iter().any(|t| t == digest),
        _ => lines.iter().map(|l| tokens(l)).any(|t| t.iter().any(|x| x == digest) && names_file(&t)),
    }
}

/// Hash a published checksum file is made with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashKind {
    Sha256,
    Sha512,
}

impl HashKind {
    /// From the length of the first hex digest in the file (`<hex>  <name>`).
    fn of_checksum(text: &str) -> Self {
        let is_hex = |w: &&str| w.bytes().all(|b| b.is_ascii_hexdigit());
        match text.split_whitespace().find(is_hex).map(str::len) {
            Some(64) => HashKind::Sha256,
            _ => HashKind::Sha512,
        }
    }

    fn net(self) -> heronite_net::Hash {
        match self {
            HashKind::Sha256 => heronite_net::Hash::Sha256,
            HashKind::Sha512 => heronite_net::Hash::Sha512,
        }
    }
}

/// Streams `url` to `dest` (through `<dest>.part`) and hands the body's hash
/// to `validate` before the file gets its name.
fn download_file(
    url: &str,
    dest: &Path,
    expected_size: u64,
    algorithm: HashKind,
    validate: &dyn Fn(&heronite_net::Outcome) -> Result<(), String>,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), heronite_net::Error> {
    let start = Instant::now();
    let mut report = |done: u64, total: Option<u64>| {
        let speed = done as f64 / start.elapsed().as_secs_f64().max(0.001);
        on_progress(progress(done, total.unwrap_or(0), speed));
    };
    let opts = heronite_net::Options {
        size_hint: Some(expected_size).filter(|n| *n > 0),
        hash: Some(algorithm.net()),
        validate: Some(validate),
        on_progress: Some(&mut report),
        ..Default::default()
    };
    heronite_net::download(url, dest, opts, &|| cancel.load(Ordering::Relaxed)).map(|_| ())
}

fn progress(downloaded: u64, total: u64, speed: f64) -> Progress {
    let percentage = if total > 0 { (downloaded as f64 / total as f64 * 100.0).min(100.0) } else { 0.0 };
    let eta_secs = (speed > 0.0 && total > downloaded).then(|| ((total - downloaded) as f64 / speed) as u64);
    Progress::Downloading { percentage, downloaded, total, speed, eta_secs }
}

/// Extracts with the system `tar` (handles .tar.gz/.xz/.zst like Heroic) and
/// can be cancelled by killing it.
fn extract_tar(archive: &Path, dest: &Path, strip: u32, cancel: &AtomicBool) -> Result<(), InstallError> {
    let mut cmd = Command::new("tar");
    cmd.arg("-xf").arg(archive).arg("-C").arg(dest).arg(format!("--strip-components={strip}"));
    // A guarded helper: told to stop if the app dies, whole group killed on cancel.
    let poll = || cancel.load(Ordering::Relaxed);
    let out = Run::new()
        .poll(&poll)
        .grace(Duration::from_secs(3))
        .run(&mut cmd)
        .map_err(|e| InstallError::Failed(tr!("could not run tar: {error}", error = e)))?;
    if out.aborted {
        Err(InstallError::Aborted)
    } else if out.success() {
        Ok(())
    } else {
        Err(InstallError::Failed(out.stderr_text().trim().to_owned()))
    }
}

/// Apparent size of a folder in bytes (`du -sb`).
pub fn folder_size(dir: &Path) -> u64 {
    fn walk(p: &Path) -> u64 {
        let Ok(md) = fs::symlink_metadata(p) else { return 0 };
        let mut total = md.len();
        if md.is_dir() {
            if let Ok(rd) = fs::read_dir(p) {
                for e in rd.flatten() {
                    total += walk(&e.path());
                }
            }
        }
        total
    }
    walk(dir)
}

/// Only folders directly inside Heroic's tools folders or a Steam
/// `compatibilitytools.d` may be deleted by the manager.
pub fn is_managed_dir(data_paths: &DataPaths, path: &Path) -> bool {
    let Some(parent) = path.parent() else { return false };
    if path.file_name().is_none() {
        return false;
    }
    let tools = data_paths.tools_dir();
    ["wine", "proton", "game-porting-toolkit"].iter().any(|d| parent == tools.join(d))
        || parent.file_name().is_some_and(|n| n == "compatibilitytools.d")
}

/// Deletes the build and marks it as not installed. Blocking. A folder that
/// cannot be deleted is an error and the build stays listed as installed.
pub fn remove_version(data_paths: &DataPaths, info: &WineVersionInfo) -> anyhow::Result<()> {
    if let Some(dir) = info.install_dir.as_deref().filter(|d| !d.is_empty()) {
        let path = Path::new(dir);
        if path.exists() {
            if !is_managed_dir(data_paths, path) {
                return Err(anyhow!(tr!("Refusing to delete {dir}: it is not inside a Wine/Proton tools folder", dir)));
            }
            if let Err(e) = fs::remove_dir_all(path) {
                log::warn!("Couldn't remove folder {dir}: {e}");
                return Err(anyhow!(tr!("Could not delete {dir}: {error}", dir, error = e.to_string())));
            }
        }
    }
    store::mark_removed(data_paths, &info.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256, Sha512};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

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

    fn make_tar_gz(tmp: &Path, top: &str) -> Vec<u8> {
        let src = tmp.join("src");
        fs::create_dir_all(src.join(top).join("files")).unwrap();
        fs::write(src.join(top).join("proton"), "#!/bin/sh\n").unwrap();
        fs::write(src.join(top).join("files/blob"), vec![7u8; 300_000]).unwrap();
        let out = tmp.join("a.tar.gz");
        let st = Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(&src).arg(top).status().unwrap();
        assert!(st.success());
        fs::read(out).unwrap()
    }

    fn sha512_hex(data: &[u8]) -> String {
        Sha512::digest(data).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn info_for(base: &str, name: &str, data: &[u8]) -> WineVersionInfo {
        WineVersionInfo {
            version: "GE-Proton-test".into(),
            kind: "GE-Proton".into(),
            date: "2026-01-01".into(),
            download: Some(format!("{base}/{name}")),
            downsize: Some(data.len() as u64),
            checksum: Some(format!("{base}/{name}.sha512sum")),
            ..Default::default()
        }
    }

    #[test]
    fn installs_verifies_and_strips_top_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let data = make_tar_gz(tmp.path(), "GE-Proton-test");
        let sum = format!("{}  a.tar.gz\n", sha512_hex(&data));
        let base = serve(vec![("/a.tar.gz".into(), data.clone()), ("/a.tar.gz.sha512sum".into(), sum.into_bytes())], 0);
        let info = info_for(&base, "a.tar.gz", &data);
        let dest = tmp.path().join("tools/proton");
        // leftover of a crashed extraction is cleaned up
        fs::create_dir_all(dest.join(".GE-Proton-test.extracting")).unwrap();
        fs::write(dest.join(".GE-Proton-test.extracting/junk"), "x").unwrap();
        let mut seen_unzip = false;
        let mut last_pct = 0.0;
        let done = install_into(&info, &dest, false, &AtomicBool::new(false), &mut |p| match p {
            Progress::Unzipping => seen_unzip = true,
            Progress::Downloading { percentage, .. } => last_pct = percentage,
        })
        .unwrap();
        assert!(seen_unzip);
        assert_eq!(last_pct, 100.0);
        let sub = dest.join("GE-Proton-test");
        assert!(sub.join("proton").exists() && sub.join("files/blob").exists());
        assert_eq!(done.install_dir.as_deref(), Some(sub.to_str().unwrap()));
        assert_eq!(done.is_installed, Some(true));
        assert!(done.disksize >= 300_000);
        // archive and partial file are gone
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
    }

    #[test]
    fn sha256_checksums_are_verified_too() {
        let tmp = tempfile::tempdir().unwrap();
        let data = make_tar_gz(tmp.path(), "proton-EM-test");
        let sha256: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
        let good = format!("{sha256}  proton-EM-test.tar.xz\n");
        let bad = format!("{}  proton-EM-test.tar.xz\n", "0".repeat(64));
        let base =
            serve(vec![("/a.tar.gz".into(), data.clone()), ("/good".into(), good.into_bytes()), ("/bad".into(), bad.into_bytes())], 0);
        let mut info = info_for(&base, "a.tar.gz", &data);
        info.checksum = Some(format!("{base}/bad"));
        let dest = tmp.path().join("tools/proton");
        let err = install_into(&info, &dest, false, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("Checksum verification failed"), "{err}");
        info.checksum = Some(format!("{base}/good"));
        assert!(install_into(&info, &dest, false, &AtomicBool::new(false), &mut |_| {}).unwrap().installed());
    }

    #[test]
    fn checksum_kind_follows_the_digest_length() {
        assert_eq!(HashKind::of_checksum(&format!("{}  a.tar.xz", "a".repeat(64))), HashKind::Sha256);
        assert_eq!(HashKind::of_checksum(&format!("{}  a.tar.gz\n", "b".repeat(128))), HashKind::Sha512);
        assert_eq!(HashKind::of_checksum("deadbeef  a.tar.gz"), HashKind::Sha512);
    }

    #[test]
    fn a_digest_matches_as_a_whole_token_on_the_line_of_the_file() {
        let d = "ab".repeat(32);
        let other = "cd".repeat(32);
        assert!(checksum_matches(&d, &d, "a.tar.gz"));
        assert!(checksum_matches(&format!("{}  anything.tar\n", d.to_uppercase()), &d, "a.tar.gz"));
        assert!(checksum_matches(&format!("{other}  b.tar.gz\n{d}  a.tar.gz\n"), &d, "a.tar.gz"));
        assert!(checksum_matches(&format!("{d} *dist/a.tar.gz\n{other}  b.tar.gz"), &d, "a.tar.gz"));
        assert!(checksum_matches(&format!("SHA256 (a.tar.gz) = {d}"), &d, "a.tar.gz"));
        // part of a longer token, or the digest of another file
        assert!(!checksum_matches(&format!("00{d}"), &d, "a.tar.gz"));
        assert!(!checksum_matches(&format!("{d}00  a.tar.gz"), &d, "a.tar.gz"));
        assert!(!checksum_matches(&format!("{other}  a.tar.gz\n{d}  b.tar.gz\n"), &d, "a.tar.gz"));
        assert!(!checksum_matches("", &d, "a.tar.gz"));
        assert!(!checksum_matches(&d, "", "a.tar.gz"));
    }

    #[test]
    fn bad_checksum_leaves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let data = make_tar_gz(tmp.path(), "x");
        let base = serve(vec![("/a.tar.gz".into(), data.clone()), ("/a.tar.gz.sha512sum".into(), b"deadbeef  a.tar.gz".to_vec())], 0);
        let info = info_for(&base, "a.tar.gz", &data);
        let dest = tmp.path().join("tools/proton");
        let err = install_into(&info, &dest, false, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("Checksum verification failed"), "{err}");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn cancel_during_download_cleans_up_and_keeps_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let data = make_tar_gz(tmp.path(), "x");
        let sum = sha512_hex(&data).to_string();
        let base = serve(vec![("/a.tar.gz".into(), data.clone()), ("/a.tar.gz.sha512sum".into(), sum.into_bytes())], 30);
        let mut info = info_for(&base, "a.tar.gz", &data);
        let dest = tmp.path().join("tools/proton");
        // An existing install being updated must survive an aborted update.
        let existing = dest.join("GE-Proton-test");
        fs::create_dir_all(&existing).unwrap();
        fs::write(existing.join("proton"), "old").unwrap();
        info.install_dir = Some(existing.to_string_lossy().into_owned());
        info.has_update = Some(true);
        let cancel = AtomicBool::new(false);
        let err = install_into(&info, &dest, true, &cancel, &mut |p| {
            if let Progress::Downloading { downloaded, .. } = p {
                if downloaded > 0 {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        })
        .unwrap_err();
        assert!(matches!(err, InstallError::Aborted), "{err}");
        assert_eq!(fs::read_to_string(existing.join("proton")).unwrap(), "old");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
    }

    #[test]
    fn update_replaces_and_failed_extract_restores_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let data = make_tar_gz(tmp.path(), "GE-Proton-test");
        let sum = sha512_hex(&data);
        let base = serve(vec![("/a.tar.gz".into(), data.clone()), ("/a.tar.gz.sha512sum".into(), sum.into_bytes())], 0);
        let mut info = info_for(&base, "a.tar.gz", &data);
        let dest = tmp.path().join("tools/proton");
        let existing = dest.join("GE-Proton-test");
        fs::create_dir_all(&existing).unwrap();
        fs::write(existing.join("old-file"), "old").unwrap();
        info.install_dir = Some(existing.to_string_lossy().into_owned());
        info.has_update = Some(true);
        install_into(&info, &dest, true, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(existing.join("proton").exists());
        assert!(!existing.join("old-file").exists());
        assert!(!dest.join("GE-Proton-test_backup").exists());

        // Not a tar archive: extraction fails and the old install comes back.
        let junk = b"this is not a tar archive".to_vec();
        let sum = sha512_hex(&junk);
        let base = serve(vec![("/b.tar.gz".into(), junk.clone()), ("/b.tar.gz.sha512sum".into(), sum.into_bytes())], 0);
        let info2 = info_for(&base, "b.tar.gz", &junk);
        let mut info2 = info2;
        info2.install_dir = info.install_dir.clone();
        info2.has_update = Some(true);
        let err = install_into(&info2, &dest, true, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("Unzip of b.tar.gz failed"), "{err}");
        assert!(existing.join("proton").exists());
        assert!(!dest.join("GE-Proton-test_backup").exists());
    }

    #[test]
    fn install_dir_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(tmp.path().join("h"));
        let steam = tmp.path().join("steam");
        fs::create_dir_all(&steam).unwrap();
        let mut cfg = WineConfig { default_steam_path: steam.to_string_lossy().into_owned(), ..Default::default() };
        let ge = WineVersionInfo { version: "GE-Proton9-1".into(), kind: "GE-Proton".into(), ..Default::default() };
        let lutris = WineVersionInfo { kind: "Wine-Lutris".into(), ..ge.clone() };
        assert_eq!(install_dir_for(&data_paths, &cfg, &ge), data_paths.tools_dir().join("proton"));
        assert_eq!(install_dir_for(&data_paths, &cfg, &lutris), data_paths.tools_dir().join("wine"));
        cfg.download_proton_to_steam = true;
        // invalid steam path (no steam.sh) falls back to the tools folder
        assert_eq!(install_dir_for(&data_paths, &cfg, &ge), data_paths.tools_dir().join("proton"));
        fs::write(steam.join("steam.sh"), "").unwrap();
        assert_eq!(install_dir_for(&data_paths, &cfg, &ge), steam.join("compatibilitytools.d"));
    }

    /// A build whose folder cannot be emptied is reported and stays listed as installed.
    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_deleted_is_an_error_and_stays_installed() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(tmp.path().join("h"));
        let dir = data_paths.tools_dir().join("proton/GE-Proton9-2");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("proton"), "x").unwrap();
        let info = WineVersionInfo {
            version: "GE-Proton9-2".into(),
            kind: "GE-Proton".into(),
            install_dir: Some(dir.to_string_lossy().into_owned()),
            is_installed: Some(true),
            ..Default::default()
        };
        store::mark_installed(&data_paths, &info).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let root = fs::write(dir.join("probe"), "").is_ok();
        let result = remove_version(&data_paths, &info);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        if root {
            return; // permissions do not stop root
        }
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("GE-Proton9-2"), "{err}");
        assert!(store::read_releases(&data_paths)[0].installed());
        remove_version(&data_paths, &info).unwrap();
        assert!(!dir.exists());
        assert!(!store::read_releases(&data_paths)[0].installed());
    }

    #[test]
    fn remove_deletes_only_managed_dirs_and_updates_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(tmp.path().join("h"));
        let dir = data_paths.tools_dir().join("proton/GE-Proton9-1");
        fs::create_dir_all(dir.join("files")).unwrap();
        fs::write(dir.join("proton"), "x").unwrap();
        let info = WineVersionInfo {
            version: "GE-Proton9-1".into(),
            kind: "GE-Proton".into(),
            install_dir: Some(dir.to_string_lossy().into_owned()),
            is_installed: Some(true),
            disksize: 5,
            ..Default::default()
        };
        store::mark_installed(&data_paths, &info).unwrap();
        remove_version(&data_paths, &info).unwrap();
        assert!(!dir.exists());
        assert!(!store::read_releases(&data_paths)[0].installed());

        let outside = tmp.path().join("precious");
        fs::create_dir_all(&outside).unwrap();
        let bad = WineVersionInfo { install_dir: Some(outside.to_string_lossy().into_owned()), ..info.clone() };
        assert!(remove_version(&data_paths, &bad).is_err());
        assert!(outside.exists());
        assert!(!is_managed_dir(&data_paths, Path::new("/")));
        assert!(!is_managed_dir(&data_paths, &data_paths.tools_dir().join("proton")));
        assert!(is_managed_dir(&data_paths, Path::new("/home/u/.local/share/Steam/compatibilitytools.d/X")));
    }
}
