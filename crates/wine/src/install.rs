//! Download, verification, extraction and removal of a Wine/Proton build
//! (port of `downloader/main.ts::installVersion`, `utils.ts::installWineVersion`
//! / `removeWineVersion` and `getInstallDir`).

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};
use heronite_data::HeroicPaths;
use sha2::{Digest, Sha512};

use crate::repos;
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
pub fn install_dir_for(heroic: &HeroicPaths, cfg: &WineConfig, info: &WineVersionInfo) -> PathBuf {
    let tools = heroic.tools_dir();
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
            "Configured Steam path (\"{}\") does not appear to be valid, installing into Heroic tools path instead",
            cfg.default_steam_path
        );
    }
    tools.join("proton")
}

/// Full install into `install_dir` and update of the release cache. Blocking.
pub fn install_version(
    heroic: &HeroicPaths,
    cfg: &WineConfig,
    info: &WineVersionInfo,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<WineVersionInfo, InstallError> {
    let install_dir = install_dir_for(heroic, cfg, info);
    let overwrite = info.has_update();
    let updated = install_into(info, &install_dir, overwrite, cancel, on_progress)?;
    store::mark_installed(heroic, &updated).map_err(InstallError::from)?;
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
        .ok_or_else(|| InstallError::Failed(format!("No download link provided for {}!", info.version)))?;
    if !is_safe_name(&info.version) {
        return Err(InstallError::Failed(format!("Invalid version name {:?}", info.version)));
    }
    fs::create_dir_all(install_dir).with_context(|| format!("creating {}", install_dir.display()))?;
    if !install_dir.is_dir() {
        return Err(InstallError::Failed(format!(
            "Installation directory {} is not a directory!",
            install_dir.display()
        )));
    }

    let file_name = download.rsplit('/').next().unwrap_or("download.tar");
    let tar_file = install_dir.join(file_name);
    let part_file = install_dir.join(format!("{file_name}.part"));
    let install_sub_dir = match info.install_dir.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => install_dir.join(&info.version),
    };

    // Source checksum (a `.sha512sum` file next to the archive).
    let source_checksum = match info.checksum.as_deref().filter(|c| !c.is_empty()) {
        Some(url) => Some(
            repos::http_get_text(url, 1024 * 1024)
                .map_err(|e| InstallError::Failed(format!("Could not fetch the checksum: {e}")))?,
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
    let _ = fs::remove_file(&part_file);

    // Download (+ sha512 while streaming).
    let digest = match download_file(download, &part_file, info.downsize.unwrap_or(0), cancel, on_progress) {
        Ok(d) => d,
        Err(e) => {
            let _ = fs::remove_file(&part_file);
            return Err(match e {
                InstallError::Aborted => InstallError::Aborted,
                InstallError::Failed(m) => {
                    InstallError::Failed(format!("Download of {} failed with:\n {m}", info.version))
                }
            });
        }
    };
    if let Some(sum) = &source_checksum {
        if !sum.to_ascii_lowercase().contains(&digest) {
            let _ = fs::remove_file(&part_file);
            return Err(InstallError::Failed("Checksum verification failed".into()));
        }
    } else {
        log::warn!("No checksum provided. Download of {} could be invalid!", info.version);
    }
    fs::rename(&part_file, &tar_file).with_context(|| format!("renaming {}", part_file.display()))?;

    // Backup of the old folder when updating.
    let backup = PathBuf::from(format!("{}_backup", install_sub_dir.display()));
    let mut backed_up = false;
    if overwrite && install_sub_dir.exists() {
        let _ = fs::remove_dir_all(&backup);
        fs::rename(&install_sub_dir, &backup)
            .map_err(|e| InstallError::Failed(format!("Could not back up {}: {e}", install_sub_dir.display())))?;
        backed_up = true;
    }
    let restore = |remove_new: bool| {
        if remove_new {
            let _ = fs::remove_dir_all(&install_sub_dir);
        }
        if backed_up {
            let _ = fs::rename(&backup, &install_sub_dir);
        }
    };

    if let Err(e) = fs::create_dir(&install_sub_dir) {
        let _ = fs::remove_file(&tar_file);
        restore(false);
        return Err(InstallError::Failed(format!(
            "Failed to make folder {} with:\n {e}",
            install_sub_dir.display()
        )));
    }

    on_progress(Progress::Unzipping);
    let extracted = extract_tar(&tar_file, &install_sub_dir, 1, cancel);
    let _ = fs::remove_file(&tar_file);
    if let Err(e) = extracted {
        restore(true);
        return Err(match e {
            InstallError::Aborted => InstallError::Aborted,
            InstallError::Failed(m) => InstallError::Failed(format!(
                "Unzip of {file_name} failed with:\n {m}"
            )),
        });
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

/// Streams `url` to `dest`, returning the lowercase sha512 hex of the body.
fn download_file(
    url: &str,
    dest: &Path,
    expected_size: u64,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<String, InstallError> {
    let fail = |m: String| InstallError::Failed(m);
    let mut resp = repos::http_agent()
        .get(url)
        .call()
        .map_err(|e| fail(format!("{url}: {e}")))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(expected_size);
    let mut body = resp.body_mut().with_config().limit(u64::MAX).reader();
    let mut file = File::create(dest).map_err(|e| fail(format!("creating {}: {e}", dest.display())))?;
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded = 0u64;
    let start = Instant::now();
    let mut last = Instant::now() - Duration::from_secs(1);
    on_progress(progress(0, total, 0.0));
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Aborted);
        }
        let n = body.read(&mut buf).map_err(|e| fail(format!("{url}: {e}")))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| fail(format!("writing {}: {e}", dest.display())))?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            let speed = downloaded as f64 / start.elapsed().as_secs_f64().max(0.001);
            on_progress(progress(downloaded, total, speed));
        }
    }
    file.flush().map_err(|e| fail(e.to_string()))?;
    if total > 0 && downloaded < total {
        return Err(fail(format!("connection closed after {downloaded} of {total} bytes")));
    }
    let speed = downloaded as f64 / start.elapsed().as_secs_f64().max(0.001);
    on_progress(progress(downloaded, downloaded.max(total), speed));
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn progress(downloaded: u64, total: u64, speed: f64) -> Progress {
    let percentage = if total > 0 { (downloaded as f64 / total as f64 * 100.0).min(100.0) } else { 0.0 };
    let eta_secs = (speed > 0.0 && total > downloaded).then(|| ((total - downloaded) as f64 / speed) as u64);
    Progress::Downloading { percentage, downloaded, total, speed, eta_secs }
}

/// Extracts with the system `tar` (handles .tar.gz/.xz/.zst like Heroic) and
/// can be cancelled by killing it.
fn extract_tar(archive: &Path, dest: &Path, strip: u32, cancel: &AtomicBool) -> Result<(), InstallError> {
    let mut child = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .arg(format!("--strip-components={strip}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| InstallError::Failed(format!("could not run tar: {e}")))?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(InstallError::Aborted);
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(80)),
            Err(e) => return Err(InstallError::Failed(e.to_string())),
        }
    };
    let err = reader.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(InstallError::Failed(err.trim().to_owned()))
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
pub fn is_managed_dir(heroic: &HeroicPaths, path: &Path) -> bool {
    let Some(parent) = path.parent() else { return false };
    if path.file_name().is_none() {
        return false;
    }
    let tools = heroic.tools_dir();
    ["wine", "proton", "game-porting-toolkit"].iter().any(|d| parent == tools.join(d))
        || parent.file_name().map_or(false, |n| n == "compatibilitytools.d")
}

/// Deletes the build and marks it as not installed. Blocking.
pub fn remove_version(heroic: &HeroicPaths, info: &WineVersionInfo) -> anyhow::Result<()> {
    if let Some(dir) = info.install_dir.as_deref().filter(|d| !d.is_empty()) {
        let path = Path::new(dir);
        if path.exists() {
            if !is_managed_dir(heroic, path) {
                return Err(anyhow!("Refusing to delete {dir}: it is not inside a Wine/Proton tools folder"));
            }
            if let Err(e) = fs::remove_dir_all(path) {
                log::warn!("Couldn't remove folder {dir}: {e}. Still marking {} as not installed", info.version);
            }
        }
    }
    store::mark_removed(heroic, &info.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
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
        let sum = format!("{}", sha512_hex(&data));
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
        let heroic = HeroicPaths::at(tmp.path().join("h"));
        let steam = tmp.path().join("steam");
        fs::create_dir_all(&steam).unwrap();
        let mut cfg = WineConfig { default_steam_path: steam.to_string_lossy().into_owned(), ..Default::default() };
        let ge = WineVersionInfo { version: "GE-Proton9-1".into(), kind: "GE-Proton".into(), ..Default::default() };
        let lutris = WineVersionInfo { kind: "Wine-Lutris".into(), ..ge.clone() };
        assert_eq!(install_dir_for(&heroic, &cfg, &ge), heroic.tools_dir().join("proton"));
        assert_eq!(install_dir_for(&heroic, &cfg, &lutris), heroic.tools_dir().join("wine"));
        cfg.download_proton_to_steam = true;
        // invalid steam path (no steam.sh) falls back to the tools folder
        assert_eq!(install_dir_for(&heroic, &cfg, &ge), heroic.tools_dir().join("proton"));
        fs::write(steam.join("steam.sh"), "").unwrap();
        assert_eq!(install_dir_for(&heroic, &cfg, &ge), steam.join("compatibilitytools.d"));
    }

    #[test]
    fn remove_deletes_only_managed_dirs_and_updates_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(tmp.path().join("h"));
        let dir = heroic.tools_dir().join("proton/GE-Proton9-1");
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
        store::mark_installed(&heroic, &info).unwrap();
        remove_version(&heroic, &info).unwrap();
        assert!(!dir.exists());
        assert!(!store::read_releases(&heroic)[0].installed());

        let outside = tmp.path().join("precious");
        fs::create_dir_all(&outside).unwrap();
        let bad = WineVersionInfo { install_dir: Some(outside.to_string_lossy().into_owned()), ..info.clone() };
        assert!(remove_version(&heroic, &bad).is_err());
        assert!(outside.exists());
        assert!(!is_managed_dir(&heroic, Path::new("/")));
        assert!(!is_managed_dir(&heroic, &heroic.tools_dir().join("proton")));
        assert!(is_managed_dir(&heroic, Path::new("/home/u/.local/share/Steam/compatibilitytools.d/X")));
    }
}
