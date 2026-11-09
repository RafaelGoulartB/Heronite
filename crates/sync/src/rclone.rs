//! rclone: the cloud accounts and servers the library syncs through (Google
//! Drive, Dropbox, OneDrive, Box, FTP, SMB, WebDAV), the way Ludusavi uses
//! it. Its settings, with the account's token, live in this app's own
//! `rclone.conf` (`AppPaths::rclone_conf`), never in the user's; a
//! [`Provider::Custom`] remote is the one place the user's own settings are
//! used.
//!
//! The binary is found (`HERONITE_RCLONE_BIN`, the data folder's copy,
//! `PATH`) or downloaded from rclone's site into `<data>/tools/bin`, checked
//! against the release's SHA-256 list.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use heronite_data::proc::{Cancel, Run};
use heronite_data::DataPaths;
use heronite_launch::net::{Net, SystemNet};
use sha2::{Digest, Sha256};

use crate::prefs::Provider;

/// The release downloaded when rclone is not installed.
pub const VERSION: &str = "v1.75.2";
/// The remote this app writes in its `rclone.conf`.
pub const REMOTE: &str = "heronite";
/// Override of the binary (tests, a build of one's own).
pub const ENV_BIN: &str = "HERONITE_RCLONE_BIN";

/// How long the user has to sign in in the browser.
const SIGN_IN_TIME: Duration = Duration::from_secs(10 * 60);
/// One copy to or from the account.
const COPY_TIME: Duration = Duration::from_secs(3 * 60);
/// Network flags of every transfer: give up early instead of hanging.
///
/// The accounts limit the requests a minute (Google Drive the hardest, and
/// for rclone's own app, shared by all its users, it is often spent): at
/// most a few requests a second, and when one is refused rclone waits and
/// tries again (its pacer backs off on every refused request).
const NET_FLAGS: [&str; 14] = [
    "--contimeout",
    "20s",
    "--timeout",
    "60s",
    "--retries",
    "3",
    "--retries-sleep",
    "20s",
    "--low-level-retries",
    "10",
    "--tpslimit",
    "4",
    "--tpslimit-burst",
    "4",
];

/// A whole upload (the first one of the screenshots can be gigabytes).
const TRANSFER_TIME: Duration = Duration::from_secs(12 * 3600);

/// rclone's exit code for "directory not found".
const EXIT_DIR_NOT_FOUND: i32 = 3;

/// A server's address and login ([`Provider::is_server`]).
#[derive(Debug, Clone, Default)]
pub struct Server {
    /// FTP / SMB: the host name; WebDAV: the URL.
    pub host: String,
    /// Empty: the protocol's usual port.
    pub port: String,
    pub user: String,
    pub password: String,
    /// WebDAV: `nextcloud`, `owncloud`...
    pub vendor: String,
    /// Google Drive: an app of one's own (`client_id`, `client_secret`),
    /// with requests of its own. Empty: rclone's, shared by all its users.
    pub client_id: String,
    pub client_secret: String,
}

impl Server {
    /// How the account shows once connected.
    pub fn shown(&self) -> String {
        let host = self.host.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
        match self.user.trim() {
            "" => host.to_owned(),
            user => format!("{user}@{host}"),
        }
    }
}

/// `<data>/tools/bin/rclone` (`.exe` on Windows).
pub fn data_bin(paths: &DataPaths) -> PathBuf {
    paths.tools_dir().join("bin").join(format!("rclone{}", std::env::consts::EXE_SUFFIX))
}

/// The binary to run, if there is one.
pub fn find(paths: &DataPaths) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(ENV_BIN).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let own = data_bin(paths);
    if own.is_file() {
        return Some(own);
    }
    let name = format!("rclone{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(&name)).find(|p| p.is_file())
}

/// The release file for this system and CPU.
fn asset() -> Option<String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(format!("rclone-{VERSION}-{os}-{arch}"))
}

/// The binary: one found, else a download. Blocking, network.
pub fn ensure(paths: &DataPaths, cancel: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    if let Some(found) = find(paths) {
        return Ok(found);
    }
    let asset = asset().ok_or_else(|| format!("rclone has no build for {} {}", std::env::consts::OS, std::env::consts::ARCH))?;
    let net = SystemNet;
    if !net.is_online() {
        return Err("rclone is not installed and there is no internet connection".into());
    }
    let target = data_bin(paths);
    let dir = target.parent().ok_or("rclone has no folder")?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let base = format!("https://downloads.rclone.org/{VERSION}");
    let zip = dir.join(format!("{asset}.zip.partial"));
    let sums = dir.join("rclone-SHA256SUMS.partial");
    let result = (|| {
        net.download(&format!("{base}/SHA256SUMS"), &sums, cancel)?;
        net.download(&format!("{base}/{asset}.zip"), &zip, cancel)?;
        let listed = fs::read_to_string(&sums).map_err(|e| e.to_string())?;
        let want = expected_sha256(&listed, &format!("{asset}.zip")).ok_or("the rclone download is not in its release's list")?;
        if sha256_of(&zip)? != want {
            return Err("the rclone download is damaged".to_owned());
        }
        unpack(&zip, &asset, &target)
    })();
    let _ = fs::remove_file(&zip);
    let _ = fs::remove_file(&sums);
    result.map_err(|e| format!("downloading rclone failed: {e}"))?;
    log::info!("rclone {VERSION} downloaded to {}", target.display());
    Ok(target)
}

/// The hash `SHA256SUMS` lists for `file` (lines `<hex>  <file>`, inside a
/// PGP signed message).
fn expected_sha256(listed: &str, file: &str) -> Option<String> {
    listed.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        (name.trim() == file && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_of(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Takes `<asset>/rclone` out of the zip into `target` (through a temp file).
fn unpack(zip: &Path, asset: &str, target: &Path) -> Result<(), String> {
    let file = fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let name = format!("{asset}/rclone{}", std::env::consts::EXE_SUFFIX);
    let mut entry = archive.by_name(&name).map_err(|e| format!("{name}: {e}"))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    heronite_data::temps::write_atomic(target, &bytes, Some(0o755)).map_err(|e| e.to_string())
}

/// rclone with this app's settings (`config: None`: the user's own).
#[derive(Debug, Clone)]
pub struct Rclone {
    pub bin: PathBuf,
    pub config: Option<PathBuf>,
}

impl Rclone {
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        if let Some(config) = &self.config {
            cmd.arg("--config").arg(config);
        }
        // An encrypted config of the user's must fail, not wait for a
        // password nobody can type.
        cmd.arg("--ask-password=false");
        cmd
    }

    fn run(
        &self,
        args: &[&str],
        timeout: Duration,
        cancel: Option<&Cancel>,
        on_line: Option<&mut dyn FnMut(&str)>,
    ) -> Result<String, RcloneError> {
        let mut cmd = self.command();
        cmd.args(args);
        let mut run = Run::new().timeout(timeout).grace(Duration::from_secs(3));
        if let Some(cancel) = cancel {
            run = run.cancel(cancel);
        }
        if let Some(f) = on_line {
            run = run.on_line(f);
        }
        let out = run.run(&mut cmd).map_err(|e| RcloneError::Failed(format!("running rclone: {e}")))?;
        if out.aborted {
            return Err(RcloneError::Cancelled);
        }
        if out.timed_out {
            return Err(RcloneError::Failed("rclone took too long".into()));
        }
        match out.code {
            Some(0) => Ok(out.stdout_text()),
            Some(EXIT_DIR_NOT_FOUND) => Err(RcloneError::NotFound),
            _ => Err(RcloneError::Failed(last_error(&out.stderr_text()))),
        }
    }

    /// Connects the account of `provider` as [`REMOTE`]: a browser sign-in
    /// for the cloud accounts (`on_link` gets the page to open if the
    /// browser did not), the login for a server. Checked by making the sync
    /// folder `folder`. Anything half made is removed on failure.
    pub fn connect(
        &self,
        provider: Provider,
        server: &Server,
        folder: &str,
        cancel: &Cancel,
        on_link: &mut dyn FnMut(&str),
    ) -> Result<(), RcloneError> {
        let _ = self.disconnect();
        let mut args: Vec<String> = vec!["config".into(), "create".into(), REMOTE.into()];
        args.extend(create_args(provider, server).into_iter().map(str::to_owned));
        if provider.is_server() {
            args.push("--obscure".into());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut on_line = |line: &str| {
            if let Some(url) = line.split("go to the following link:").nth(1) {
                on_link(url.trim());
            }
        };
        self.run(&args, SIGN_IN_TIME, Some(cancel), Some(&mut on_line)).inspect_err(|_| {
            let _ = self.disconnect();
        })?;
        // The folder proves the account works (a wrong password shows here).
        // An account refusing requests for a minute does not undo the
        // sign-in: the first upload makes the folder.
        let dir = remote_path(&format!("{REMOTE}:"), folder);
        let mut mkdir = vec!["mkdir", dir.as_str()];
        mkdir.extend(NET_FLAGS);
        match self.run(&mkdir, COPY_TIME, Some(cancel), None) {
            Ok(_) => Ok(()),
            Err(RcloneError::Failed(e)) if is_rate_limited(&e) => Ok(()),
            Err(e) => {
                let _ = self.disconnect();
                Err(e)
            }
        }
    }

    /// Removes [`REMOTE`] (and its token) from this app's settings.
    pub fn disconnect(&self) -> Result<(), RcloneError> {
        self.run(&["config", "delete", REMOTE], Duration::from_secs(30), None, None).map(drop)
    }

    /// Copies `<root>/devices/*/state.json` into `mirror` (exactly: files gone
    /// there go here too). A root not made yet is an empty one.
    pub fn pull(&self, root: &str, mirror: &Path) -> Result<(), RcloneError> {
        let dest = mirror.join(crate::format::DEVICES_DIR);
        let src = remote_path(root, crate::format::DEVICES_DIR);
        let dest_s = dest.to_string_lossy();
        let mut args = vec!["sync", src.as_str(), dest_s.as_ref(), "--include", "/*/state.json"];
        args.extend(NET_FLAGS);
        match self.run(&args, COPY_TIME, None, None) {
            Err(RcloneError::NotFound) => {
                let _ = fs::remove_dir_all(&dest);
                Ok(())
            }
            other => other.map(drop),
        }
    }

    /// Uploads this PC's file as `<root>/devices/<device>/state.json`.
    pub fn push(&self, file: &Path, root: &str, device: &str) -> Result<(), RcloneError> {
        let dest = remote_path(root, &format!("{}/{device}/{}", crate::format::DEVICES_DIR, crate::format::STATE_FILE));
        let file = file.to_string_lossy();
        self.copy_file(file.as_ref(), &dest, None)
    }

    /// Copies one file (`copyto`), either way.
    pub fn copy_file(&self, from: &str, to: &str, cancel: Option<&Cancel>) -> Result<(), RcloneError> {
        let mut args = vec!["copyto", from, to];
        args.extend(NET_FLAGS);
        self.run(&args, COPY_TIME, cancel, None).map(drop)
    }

    /// A long transfer (`copy`, `sync`...): `args` after the command name's
    /// own flags; `on_percent` gets how far it is (`0..=100`).
    pub fn transfer(&self, args: &[&str], cancel: &Cancel, on_percent: &mut dyn FnMut(u8)) -> Result<(), RcloneError> {
        let mut all: Vec<&str> = args.to_vec();
        all.extend(NET_FLAGS);
        // Few at a time; folders keep their dates (setting them is one more
        // request per folder).
        all.extend([
            "--transfers",
            "4",
            "--checkers",
            "4",
            "--no-update-dir-modtime",
            "--stats",
            "2s",
            "--stats-one-line",
            "--stats-log-level",
            "NOTICE",
        ]);
        let mut on_line = |line: &str| {
            if let Some(p) = percent_of(line) {
                on_percent(p);
            }
        };
        self.run(&all, TRANSFER_TIME, Some(cancel), Some(&mut on_line)).map(drop)
    }

    /// The names in `path` (`lsf`): folders (ending in `/`) or files. A
    /// folder not made yet is an empty one.
    pub fn list(&self, path: &str, dirs: bool) -> Result<Vec<String>, RcloneError> {
        let kind = if dirs { "--dirs-only" } else { "--files-only" };
        let mut args = vec!["lsf", path, kind];
        args.extend(NET_FLAGS);
        match self.run(&args, COPY_TIME, None, None) {
            Ok(out) => Ok(out.lines().map(|l| l.trim_end_matches('/').to_owned()).filter(|l| !l.is_empty()).collect()),
            Err(RcloneError::NotFound) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// The files under `path` matching `include` (an rclone filter), with
    /// their sizes, `/`-separated paths relative to it (`lsjson -R`, one
    /// listing). A folder not made yet is an empty one.
    pub fn list_files(&self, path: &str, include: &str) -> Result<Vec<(String, u64)>, RcloneError> {
        let mut args = vec!["lsjson", path, "-R", "--files-only", "--no-mimetype", "--no-modtime", "--fast-list", "--include", include];
        args.extend(NET_FLAGS);
        let out = match self.run(&args, COPY_TIME, None, None) {
            Ok(out) => out,
            Err(RcloneError::NotFound) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let list: Vec<serde_json::Value> = serde_json::from_str(&out).map_err(|e| RcloneError::Failed(e.to_string()))?;
        Ok(list.iter().filter_map(|f| Some((f["Path"].as_str()?.to_owned(), f["Size"].as_u64().unwrap_or(0)))).collect())
    }

    pub fn delete_file(&self, path: &str) -> Result<(), RcloneError> {
        let mut args = vec!["deletefile", path];
        args.extend(NET_FLAGS);
        self.run(&args, COPY_TIME, None, None).map(drop)
    }
}

/// `rclone config create <name>` arguments after the name.
fn create_args(provider: Provider, s: &Server) -> Vec<&str> {
    match provider {
        // Only the files rclone made: the rest of the Drive stays out of reach.
        Provider::Drive => {
            let mut args = vec!["drive", "scope", "drive.file"];
            if !s.client_id.trim().is_empty() {
                args.extend(["client_id", s.client_id.trim(), "client_secret", s.client_secret.trim()]);
            }
            args
        }
        Provider::Dropbox => vec!["dropbox"],
        Provider::OneDrive => vec!["onedrive"],
        Provider::Box => vec!["box"],
        Provider::Ftp | Provider::Smb => {
            let kind = if provider == Provider::Ftp { "ftp" } else { "smb" };
            let mut args = vec![kind, "host", s.host.trim(), "user", s.user.trim(), "pass", s.password.as_str()];
            if !s.port.trim().is_empty() {
                args.extend(["port", s.port.trim()]);
            }
            args
        }
        Provider::WebDav => {
            let vendor = if s.vendor.is_empty() { "other" } else { s.vendor.as_str() };
            vec!["webdav", "url", s.host.trim(), "vendor", vendor, "user", s.user.trim(), "pass", s.password.as_str()]
        }
        Provider::Folder | Provider::Custom => Vec::new(),
    }
}

/// `root` + `/` + `rel`, for `remote:` and `remote:path` alike.
pub fn remote_path(root: &str, rel: &str) -> String {
    let root = root.trim();
    let rel = rel.trim_matches('/');
    if root.ends_with(':') || root.ends_with('/') {
        format!("{root}{rel}")
    } else {
        format!("{root}/{rel}")
    }
}

/// The percentage of a `--stats-one-line` line
/// (`… 15.938 MiB / 60.000 MiB, 27%, 5.312 MiB/s, ETA 8s`).
fn percent_of(line: &str) -> Option<u8> {
    line.split(", ").find_map(|part| part.trim().strip_suffix('%')?.parse::<u8>().ok()).map(|p| p.min(100))
}

/// Whether rclone's error is the account refusing requests for a while
/// (too many a minute): worth trying again later.
pub fn is_rate_limited(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    e.contains("ratelimit")
        || e.contains("rate limit")
        || e.contains("quota exceeded")
        || e.contains("too many requests")
        || e.contains("too_many_requests")
        || e.contains("error 429")
}

/// The last thing rclone said went wrong, without its date.
fn last_error(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let pick = lines.iter().rev().find(|l| l.contains("ERROR") || l.contains("CRITICAL") || l.starts_with("Error")).or(lines.last());
    let line = pick.copied().unwrap_or("rclone failed");
    let line = line.split_once(" ERROR : ").or_else(|| line.split_once(" CRITICAL: ")).map_or(line, |(_, rest)| rest);
    line.chars().take(300).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcloneError {
    Cancelled,
    /// The folder is not in the account (yet).
    NotFound,
    /// What rclone said (English).
    Failed(String),
}

impl std::fmt::Display for RcloneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RcloneError::Cancelled => write!(f, "cancelled"),
            RcloneError::NotFound => write!(f, "folder not found"),
            RcloneError::Failed(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_join_like_rclone_expects() {
        assert_eq!(remote_path("heronite:", "Heronite"), "heronite:Heronite");
        assert_eq!(remote_path("heronite:Heronite", "devices"), "heronite:Heronite/devices");
        assert_eq!(remote_path("mine:games/", "/devices/"), "mine:games/devices");
    }

    #[test]
    fn the_release_list_gives_the_hash_of_the_file() {
        let listed = "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA1\n\n\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  rclone-v1.75.2-linux-amd64.zip\n\
            ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff  rclone-v1.75.2-windows-amd64.zip\n";
        assert_eq!(
            expected_sha256(listed, "rclone-v1.75.2-linux-amd64.zip").as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert_eq!(expected_sha256(listed, "rclone-v1.75.2-linux-arm64.zip"), None);
    }

    #[test]
    fn progress_is_read_from_the_stats_line() {
        assert_eq!(percent_of("2026/11/01 19:00:00 NOTICE:   15.938 MiB / 60.000 MiB, 27%, 5.312 MiB/s, ETA 8s"), Some(27));
        assert_eq!(percent_of("Transferred: 0 B / 0 B, -, 0 B/s, ETA -"), None);
        assert_eq!(percent_of("2026/11/01 19:00:00 INFO  : a.webp: Copied (new)"), None);
    }

    #[test]
    fn a_full_request_quota_is_told_apart() {
        assert!(is_rate_limited(
            "googleapi: Error 403: Quota exceeded for quota metric 'Queries' and limit 'Previous quota: Requests per minute'"
        ));
        assert!(is_rate_limited(
            "googleapi: Error 403: User Rate Limit Exceeded. Rate of requests for user exceed configured project quota"
        ));
        assert!(is_rate_limited("too_many_requests/"));
        assert!(!is_rate_limited("couldn't find root directory ID: googleapi: Error 404: File not found"));
    }

    #[test]
    fn errors_keep_what_rclone_said() {
        let stderr = "2026/11/01 18:37:49 NOTICE: something\n2026/11/01 18:37:50 ERROR : Failed to create file system: login denied\n";
        assert_eq!(last_error(stderr), "Failed to create file system: login denied");
        assert_eq!(last_error("Error: unknown flag\n"), "Error: unknown flag");
    }

    #[test]
    fn servers_get_their_login_and_cloud_accounts_only_their_type() {
        let s = Server { host: "nas.local".into(), user: "rafa".into(), password: "x".into(), ..Default::default() };
        assert_eq!(create_args(Provider::Smb, &s), ["smb", "host", "nas.local", "user", "rafa", "pass", "x"]);
        assert_eq!(create_args(Provider::Drive, &s), ["drive", "scope", "drive.file"]);
        let own = Server { client_id: "123.apps.googleusercontent.com".into(), client_secret: "s".into(), ..Default::default() };
        assert_eq!(
            create_args(Provider::Drive, &own),
            ["drive", "scope", "drive.file", "client_id", "123.apps.googleusercontent.com", "client_secret", "s"]
        );
        assert_eq!(s.shown(), "rafa@nas.local");
        let w = Server { host: "https://cloud.me/remote.php/dav/files/rafa/".into(), ..Default::default() };
        assert_eq!(w.shown(), "cloud.me/remote.php/dav/files/rafa");
    }
}
