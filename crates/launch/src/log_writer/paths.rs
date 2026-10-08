//! Where the log files live.

use std::path::{Path, PathBuf};

use heronite_data::HeroicPaths;

use super::GameLogType;

/// Where the log files live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPaths {
    /// Heroic's logs folder (`heroic.log`, `games/`, `runners/`).
    pub base: PathBuf,
    /// This app's own log (`heronite-rs-<date>.log`).
    pub launcher_dir: PathBuf,
}

/// Heroic's logs folder on this platform (`getBaseLogPath`).
pub fn heroic_default_base() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "windows") {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
        local.join("Heroic").join("logs")
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Logs").join("Heroic Games Launcher")
    } else {
        let state = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local").join("state"));
        state.join("Heroic").join("logs")
    }
}

/// `dirs::config_dir()` (kept here so the crate needs no extra dependency).
fn config_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming"))
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
    }
}

impl LogPaths {
    /// The real locations when `heroic` is the real Heroic config folder;
    /// any other folder (tests, `HERONITE_HEROIC_DIR` copies) keeps its logs
    /// inside itself so nothing leaks into the user's real log folders.
    /// `HERONITE_LOG_DIR` overrides the base explicitly.
    pub fn for_heroic(heroic: &HeroicPaths) -> Self {
        if let Some(dir) = std::env::var_os("HERONITE_LOG_DIR").filter(|v| !v.is_empty()) {
            let base = PathBuf::from(dir);
            return Self { launcher_dir: base.join("launcher"), base };
        }
        let config = config_dir();
        let real_root = config.join("heroic");
        if real_root == heroic.root {
            let launcher = config.join("heronite-rs").join("logs");
            Self { base: heroic_default_base(), launcher_dir: launcher }
        } else {
            let base = heroic.root.join("logs");
            Self { launcher_dir: base.join("launcher"), base }
        }
    }

    /// `games/<appName>_<runner>/<type>.log`.
    pub fn game(&self, app_name: &str, runner: &str, ty: GameLogType) -> PathBuf {
        self.base.join("games").join(format!("{app_name}_{runner}")).join(format!("{}.log", ty.file_stem()))
    }

    /// `runners/<runner>.log`.
    pub fn runner(&self, runner: &str) -> PathBuf {
        self.base.join("runners").join(format!("{runner}.log"))
    }

    /// `heroic.log` (written by Heroic / Heronite).
    pub fn heroic_log(&self) -> PathBuf {
        self.base.join("heroic.log")
    }

    /// Today's launcher log.
    pub fn launcher_log(&self, date: chrono::NaiveDate) -> PathBuf {
        self.launcher_dir.join(format!("heronite-rs-{}.log", date.format("%Y-%m-%d")))
    }
}

/// `file` with `.old` appended (`launch.log` -> `launch.log.old`).
pub fn old_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_heroics_layout() {
        let p = LogPaths { base: PathBuf::from("/s/Heroic/logs"), launcher_dir: PathBuf::from("/c/logs") };
        assert_eq!(p.game("Calluna", "legendary", GameLogType::Launch), PathBuf::from("/s/Heroic/logs/games/Calluna_legendary/launch.log"));
        assert_eq!(p.game("a", "gog", GameLogType::Install), PathBuf::from("/s/Heroic/logs/games/a_gog/install.log"));
        assert_eq!(p.runner("legendary"), PathBuf::from("/s/Heroic/logs/runners/legendary.log"));
        assert_eq!(old_path(&p.heroic_log()), PathBuf::from("/s/Heroic/logs/heroic.log.old"));
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        assert_eq!(p.launcher_log(d), PathBuf::from("/c/logs/heronite-rs-2026-10-08.log"));
    }

    #[test]
    fn other_config_folders_keep_logs_inside_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let p = LogPaths::for_heroic(&HeroicPaths::at(dir.path()));
        assert_eq!(p.base, dir.path().join("logs"));
        assert!(p.launcher_dir.starts_with(dir.path()));
    }
}
