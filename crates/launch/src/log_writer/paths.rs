//! Where the log files live.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;

use super::GameLogType;

/// Where the log files live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPaths {
    /// The logs folder (`games/`, `runners/`).
    pub base: PathBuf,
    /// This app's own log (`heronite-rs-<date>.log`).
    pub launcher_dir: PathBuf,
}

impl LogPaths {
    /// `<data>/logs`, with the launcher's logs in `launcher/`.
    /// `HERONITE_LOG_DIR` overrides the folder.
    pub fn for_data(data_paths: &DataPaths) -> Self {
        let base = std::env::var_os("HERONITE_LOG_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| data_paths.logs_dir());
        Self { launcher_dir: base.join("launcher"), base }
    }

    /// `games/<appName>_<runner>/<type>.log`.
    pub fn game(&self, app_name: &str, runner: &str, ty: GameLogType) -> PathBuf {
        self.base.join("games").join(format!("{app_name}_{runner}")).join(format!("{}.log", ty.file_stem()))
    }

    /// `runners/<runner>.log`.
    pub fn runner(&self, runner: &str) -> PathBuf {
        self.base.join("runners").join(format!("{runner}.log"))
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
        let p = LogPaths { base: PathBuf::from("/d/logs"), launcher_dir: PathBuf::from("/d/logs/launcher") };
        assert_eq!(p.game("Calluna", "legendary", GameLogType::Launch), PathBuf::from("/d/logs/games/Calluna_legendary/launch.log"));
        assert_eq!(p.game("a", "gog", GameLogType::Install), PathBuf::from("/d/logs/games/a_gog/install.log"));
        assert_eq!(p.runner("legendary"), PathBuf::from("/d/logs/runners/legendary.log"));
        assert_eq!(old_path(&p.runner("legendary")), PathBuf::from("/d/logs/runners/legendary.log.old"));
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        assert_eq!(p.launcher_log(d), PathBuf::from("/d/logs/launcher/heronite-rs-2026-10-08.log"));
    }

    #[test]
    fn logs_live_in_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let p = LogPaths::for_data(&DataPaths::at(dir.path()));
        assert_eq!(p.base, dir.path().join("logs"));
        assert_eq!(p.launcher_dir, dir.path().join("logs/launcher"));
    }
}
