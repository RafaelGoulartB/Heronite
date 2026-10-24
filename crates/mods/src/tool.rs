//! `fuse-overlayfs` and `fusermount3`: finding them, and fetching the
//! overlay tool once when the system has none.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;
use sha2::{Digest, Sha256};

use crate::model::Progress;

const VERSION: &str = "v1.18";
const BASE_URL: &str = "https://github.com/containers/fuse-overlayfs/releases/download";
const SHA256_X86_64: &str = "56b0ae0aeb8abb308b068af2f137ed8d1bd239f4f27e21672ff0def861eea1e8";
const SHA256_AARCH64: &str = "82fed736197b2a881a822e5357b488796f654e8371ce8573a1592331510a0133";
/// The static builds are about 1.5 MB; anything near this is not them.
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// One download at a time per process.
static BUSY: Mutex<()> = Mutex::new(());

/// `$HERONITE_FUSE_OVERLAYFS` (tests, a build of the user's own), then
/// `fuse-overlayfs` on `$PATH`, then `<tools_bin>/fuse-overlayfs`.
pub fn find(tools_bin: &Path) -> Option<PathBuf> {
    find_in(std::env::var_os("HERONITE_FUSE_OVERLAYFS").as_deref(), std::env::var_os("PATH").as_deref(), tools_bin)
}

fn find_in(env: Option<&OsStr>, path: Option<&OsStr>, tools_bin: &Path) -> Option<PathBuf> {
    if let Some(p) = env.filter(|p| !p.is_empty()).map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    on_path("fuse-overlayfs", path).or_else(|| Some(tools_bin.join("fuse-overlayfs")).filter(|p| p.is_file()))
}

fn on_path(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?).map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `fusermount3`, else `fusermount`: what libfuse calls to mount and unmount
/// without being root.
pub fn fusermount() -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    ["fusermount3", "fusermount"].iter().find_map(|n| on_path(n, path.as_deref()))
}

/// What this machine has for applying mods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Support {
    pub overlay: Option<PathBuf>,
    pub fusermount: Option<PathBuf>,
    pub dev_fuse: bool,
}

pub fn support(tools_bin: &Path) -> Support {
    Support { overlay: find(tools_bin), fusermount: fusermount(), dev_fuse: Path::new("/dev/fuse").exists() }
}

impl Support {
    pub fn ready(&self) -> bool {
        self.overlay.is_some() && self.fusermount.is_some() && self.dev_fuse
    }

    /// What is missing, as a sentence for the user; `None` when all is there.
    pub fn problem(&self) -> Option<String> {
        if self.fusermount.is_none() {
            Some(tr!("Heronite needs FUSE (fusermount3) to apply mods. Install the fuse3 package.").to_string())
        } else if !self.dev_fuse {
            Some(tr!("Heronite needs FUSE to apply mods, but /dev/fuse is missing. Load the fuse kernel module.").to_string())
        } else if self.overlay.is_none() && !downloadable() {
            Some(tr!("fuse-overlayfs is needed to apply mods and could not be found. Install the fuse-overlayfs package.").to_string())
        } else {
            None
        }
    }
}

/// Whether a release build of fuse-overlayfs exists for this CPU: then a
/// missing one is fetched at the first modded launch, not a problem.
fn downloadable() -> bool {
    matches!(std::env::consts::ARCH, "x86_64" | "aarch64")
}

/// The overlay tool, downloaded to `<tools_bin>/fuse-overlayfs` when the
/// system has none. The download is checked against a pinned SHA-256 before it
/// is written, so a bad file is never executable.
pub fn ensure(tools_bin: &Path, progress: &mut dyn FnMut(Progress), cancel: &AtomicBool) -> Result<PathBuf, String> {
    if let Some(found) = find(tools_bin) {
        return Ok(found);
    }
    let _one_at_a_time = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = find(tools_bin) {
        return Ok(found);
    }
    let (asset, sha) = match std::env::consts::ARCH {
        "x86_64" => ("fuse-overlayfs-x86_64", SHA256_X86_64),
        "aarch64" => ("fuse-overlayfs-aarch64", SHA256_AARCH64),
        arch => return Err(tr!("fuse-overlayfs is not available for {arch}; install it with your package manager", arch)),
    };
    let url = format!("{BASE_URL}/{VERSION}/{asset}");
    let bytes = download(&url, progress, cancel).map_err(|e| tr!("Could not download fuse-overlayfs: {error}", error = e))?;
    let target = tools_bin.join("fuse-overlayfs");
    install_verified(&bytes, sha, &target)?;
    log::info!("mods: fuse-overlayfs {VERSION} downloaded to {}", target.display());
    Ok(target)
}

/// Writes `bytes` as the executable `target` when their SHA-256 is `sha256`.
fn install_verified(bytes: &[u8], sha256: &str, target: &Path) -> Result<(), String> {
    let got = hex(&Sha256::digest(bytes));
    if !got.eq_ignore_ascii_case(sha256) {
        log::warn!("mods: fuse-overlayfs download has SHA-256 {got}, expected {sha256}");
        return Err(tr!("The fuse-overlayfs download is corrupt (its checksum does not match)").to_string());
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    write_atomic(target, bytes, Some(0o755)).map_err(|e| format!("{}: {e}", target.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn download(url: &str, progress: &mut dyn FnMut(Progress), cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .max_redirects(8)
        .user_agent("heronite-rs")
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        e => e.to_string(),
    })?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut body = resp.body_mut().with_config().limit(MAX_BYTES).reader();
    let mut out = Vec::with_capacity(total.min(MAX_BYTES) as usize);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(heronite_installer::engine::CANCELLED.to_string());
        }
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        progress(Progress { done: out.len() as u64, total });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn exe(path: &Path) {
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn find_prefers_env_then_path_then_tools_bin() {
        let t = tempfile::tempdir().unwrap();
        let (bin, sys, tools) = (t.path().join("env-bin"), t.path().join("sys"), t.path().join("tools"));
        for d in [&sys, &tools] {
            std::fs::create_dir_all(d).unwrap();
        }
        exe(&bin);
        exe(&sys.join("fuse-overlayfs"));
        exe(&tools.join("fuse-overlayfs"));
        let path = std::env::join_paths([&sys]).unwrap();
        assert_eq!(find_in(Some(bin.as_os_str()), Some(&path), &tools), Some(bin.clone()));
        assert_eq!(find_in(Some(OsStr::new("/nope")), Some(&path), &tools), Some(sys.join("fuse-overlayfs")));
        assert_eq!(find_in(None, None, &tools), Some(tools.join("fuse-overlayfs")));
        assert_eq!(find_in(None, None, &sys.join("missing")), None);
        std::fs::remove_file(sys.join("fuse-overlayfs")).unwrap();
        std::fs::write(sys.join("fuse-overlayfs"), b"not executable").unwrap();
        std::fs::set_permissions(sys.join("fuse-overlayfs"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(find_in(None, Some(&path), &sys.join("none")), None, "a file without the x bit is not a tool");
    }

    #[test]
    fn a_download_is_verified_before_it_becomes_executable() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("bin").join("fuse-overlayfs");
        let good = b"pretend binary".to_vec();
        let sha = hex(&Sha256::digest(&good));
        assert!(install_verified(b"tampered", &sha, &target).is_err());
        assert!(!target.exists(), "nothing is written for a bad hash");
        install_verified(&good, &sha.to_uppercase(), &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), good);
        assert_eq!(target.metadata().unwrap().permissions().mode() & 0o777, 0o755);
    }

    #[test]
    fn says_what_is_missing() {
        let all = Support { overlay: Some("/x".into()), fusermount: Some("/y".into()), dev_fuse: true };
        assert!(all.ready() && all.problem().is_none());
        let no_fuse = Support { fusermount: None, ..all.clone() };
        assert!(!no_fuse.ready());
        assert!(no_fuse.problem().unwrap().contains("fusermount3"));
        assert!(Support { dev_fuse: false, ..all.clone() }.problem().unwrap().contains("/dev/fuse"));
        // Downloaded at the first modded launch where a release build exists.
        assert_eq!(Support { overlay: None, ..all }.problem().is_some(), !downloadable());
    }

    #[test]
    fn pinned_hashes_look_like_sha256() {
        for h in [SHA256_X86_64, SHA256_AARCH64] {
            assert_eq!(h.len(), 64);
            assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
}
