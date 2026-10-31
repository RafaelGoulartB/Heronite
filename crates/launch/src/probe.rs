//! Everything plan building needs to know about the machine, behind a trait
//! so plans stay pure functions and tests never touch the real system.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

pub trait Probe: Send + Sync {
    fn exists(&self, path: &str) -> bool;
    /// First `n` bytes of a regular file; `None` if it is not a readable file.
    fn read_head(&self, path: &str, n: usize) -> Option<Vec<u8>>;
    /// Absolute path of `name` on `$PATH` (Heroic's `searchForExecutableOnPath`).
    fn which(&self, name: &str) -> Option<String>;
    fn env_var(&self, key: &str) -> Option<String>;
    fn home(&self) -> String;
    /// Whether the file at `path` has an execute bit (defaults to `exists`).
    fn is_executable(&self, path: &str) -> bool {
        self.exists(path)
    }
    /// Names of the entries of a folder (nothing when it cannot be read).
    fn list_dir(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
    /// Whether the gamescope at `bin` predates 3.12 (`--help` lacks `--filter`).
    /// Only consulted when the game enables gamescope.
    fn gamescope_is_old(&self, _bin: &str) -> bool {
        false
    }
    /// bubblewrap, for games in the sandbox (see [`crate::sandbox`]).
    fn bwrap(&self) -> Option<crate::sandbox::Bwrap> {
        None
    }
}

/// The real machine. Read-only.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn exists(&self, path: &str) -> bool {
        !path.is_empty() && Path::new(path).exists()
    }

    fn read_head(&self, path: &str, n: usize) -> Option<Vec<u8>> {
        let p = Path::new(path);
        if !p.is_file() {
            return None;
        }
        let mut buf = vec![0u8; n];
        let mut f = std::fs::File::open(p).ok()?;
        let mut read = 0;
        while read < n {
            match f.read(&mut buf[read..]) {
                Ok(0) => break,
                Ok(k) => read += k,
                Err(_) => return None,
            }
        }
        buf.truncate(read);
        Some(buf)
    }

    fn which(&self, name: &str) -> Option<String> {
        if name.contains('/') || name.contains(std::path::MAIN_SEPARATOR) {
            return is_executable_file(Path::new(name)).then(|| name.to_string());
        }
        let path = std::env::var_os("PATH")?;
        // Windows finds `steam` as `steam.exe`.
        let names: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
        std::env::split_paths(&path)
            .flat_map(|dir| names.iter().map(move |ext| dir.join(format!("{name}{ext}"))))
            .find(|p| is_executable_file(p))
            .map(|p| p.to_string_lossy().into_owned())
    }

    fn env_var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn home(&self) -> String {
        dirs_home()
    }

    fn is_executable(&self, path: &str) -> bool {
        is_executable_file(Path::new(path))
    }

    fn list_dir(&self, path: &str) -> Vec<String> {
        std::fs::read_dir(path).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
    }

    fn gamescope_is_old(&self, bin: &str) -> bool {
        // Same heuristic as Heroic (and Lutris): >= 3.12 documents `-F, --filter`.
        match std::process::Command::new(bin).arg("--help").output() {
            Ok(out) => {
                let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                !text.contains("-F, --filter")
            }
            Err(_) => false,
        }
    }

    fn bwrap(&self) -> Option<crate::sandbox::Bwrap> {
        crate::sandbox::detect()
    }
}

/// The folders the Vulkan loader reads implicit layer manifests from
/// (`$XDG_CONFIG_HOME`, `/etc`, `$XDG_DATA_HOME`, `$XDG_DATA_DIRS`), each
/// ending in `vulkan/implicit_layer.d`.
pub fn implicit_layer_dirs(probe: &dyn Probe) -> Vec<String> {
    let home = probe.home();
    let var = |k: &str| probe.env_var(k).filter(|v| !v.is_empty());
    let mut dirs: Vec<String> = vec![
        var("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config")),
        "/etc/xdg".into(),
        "/etc".into(),
        var("XDG_DATA_HOME").unwrap_or_else(|| format!("{home}/.local/share")),
    ];
    // An AppImage may replace XDG_DATA_DIRS: the system folders are always looked at.
    let data_dirs = var("XDG_DATA_DIRS").unwrap_or_default();
    dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(str::to_string));
    dirs.extend(["/usr/local/share".to_string(), "/usr/share".to_string()]);
    dirs.into_iter().map(|d| format!("{}/vulkan/implicit_layer.d", d.trim_end_matches('/'))).collect()
}

pub fn dirs_home() -> String {
    heronite_platform::dirs::home().map(|h| h.to_string_lossy().into_owned()).filter(|h| !h.is_empty()).unwrap_or_else(|| "/".to_string())
}

fn is_executable_file(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| heronite_platform::fs::is_executable(p, &m))
}

/// In-memory machine for tests (also handy for the app's own tests).
#[derive(Debug, Default, Clone)]
pub struct FakeProbe {
    pub paths: HashSet<String>,
    pub heads: HashMap<String, Vec<u8>>,
    pub bins: HashMap<String, String>,
    pub env: HashMap<String, String>,
    pub home: String,
    pub old_gamescope: bool,
    /// Existing files without an execute bit.
    pub not_executable: HashSet<String>,
    pub bwrap: Option<crate::sandbox::Bwrap>,
}

impl FakeProbe {
    pub fn new(home: &str) -> Self {
        Self { home: home.to_string(), ..Default::default() }
    }
    pub fn with_path(mut self, p: &str) -> Self {
        self.paths.insert(p.to_string());
        self
    }
    pub fn with_file(mut self, p: &str, head: &[u8]) -> Self {
        self.paths.insert(p.to_string());
        self.heads.insert(p.to_string(), head.to_vec());
        self
    }
    pub fn with_bin(mut self, name: &str, path: &str) -> Self {
        self.paths.insert(path.to_string());
        self.bins.insert(name.to_string(), path.to_string());
        self
    }
    pub fn with_non_executable(mut self, p: &str) -> Self {
        self.paths.insert(p.to_string());
        self.not_executable.insert(p.to_string());
        self
    }
    pub fn with_env(mut self, k: &str, v: &str) -> Self {
        self.env.insert(k.to_string(), v.to_string());
        self
    }
    /// A working bubblewrap at `/usr/bin/bwrap` of `version`.
    pub fn with_bwrap(mut self, version: (u32, u32, u32)) -> Self {
        self.bwrap = Some(crate::sandbox::Bwrap { path: "/usr/bin/bwrap".into(), version, error: None, overlay: true });
        self
    }
}

impl Probe for FakeProbe {
    fn exists(&self, path: &str) -> bool {
        self.paths.contains(path)
    }
    fn read_head(&self, path: &str, n: usize) -> Option<Vec<u8>> {
        self.heads.get(path).map(|h| h[..h.len().min(n)].to_vec())
    }
    fn which(&self, name: &str) -> Option<String> {
        self.bins.get(name).cloned()
    }
    fn env_var(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }
    fn home(&self) -> String {
        self.home.clone()
    }
    fn is_executable(&self, path: &str) -> bool {
        self.paths.contains(path) && !self.not_executable.contains(path)
    }
    fn list_dir(&self, path: &str) -> Vec<String> {
        // `\` separates too on Windows.
        let sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
        let base = path.trim_end_matches(sep);
        let mut out: Vec<String> = self
            .paths
            .iter()
            .filter_map(|p| p.strip_prefix(base)?.strip_prefix(sep))
            .map(|rest| rest.split(sep).next().unwrap_or(rest).to_string())
            .collect();
        out.sort();
        out.dedup();
        out
    }
    fn gamescope_is_old(&self, _bin: &str) -> bool {
        self.old_gamescope
    }
    fn bwrap(&self) -> Option<crate::sandbox::Bwrap> {
        self.bwrap.clone()
    }
}
