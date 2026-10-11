//! The app's own log (`heronite-rs-<date>.log`).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use super::{format_line, old_path, Level, LogPaths};

/// A launcher log file stops growing at this size; the next record moves it
/// to `.old` and starts again.
pub const MAX_LAUNCHER_LOG_BYTES: u64 = 32 * 1024 * 1024;
/// Daily launcher logs kept (the newest ones).
pub const KEEP_LAUNCHER_LOGS: usize = 7;

struct LauncherState {
    date: chrono::NaiveDate,
    file: Option<File>,
    written: u64,
}

/// Writes the app's log records (`log` crate) to
/// `heronite-rs-<date>.log`, one file per day, newest [`KEEP_LAUNCHER_LOGS`]
/// kept. Several instances may append to the same day's file.
pub struct LauncherLogger {
    paths: LogPaths,
    disabled: bool,
    state: Mutex<LauncherState>,
}

impl LauncherLogger {
    /// Opens today's log and prunes old ones. `disabled` (the global
    /// `disableLogs`) keeps everything but the warning out of the file.
    pub fn open(paths: LogPaths, disabled: bool) -> Self {
        let this = Self {
            paths,
            disabled,
            state: Mutex::new(LauncherState { date: chrono::Local::now().date_naive(), file: None, written: 0 }),
        };
        {
            let mut st = this.state.lock().unwrap();
            this.rotate(&mut st);
        }
        if disabled {
            this.write_line(Level::Warning, "", "IMPORTANT: Logs are disabled. Enable logs before reporting any issue", true);
        }
        this
    }

    fn rotate(&self, st: &mut LauncherState) {
        let path = self.paths.launcher_log(st.date);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() >= MAX_LAUNCHER_LOG_BYTES {
                let _ = std::fs::rename(&path, old_path(&path));
            }
        }
        st.written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        st.file = OpenOptions::new().create(true).append(true).open(&path).ok();
        prune_launcher_logs(&self.paths.launcher_dir, KEEP_LAUNCHER_LOGS);
    }

    /// Appends one record (`target` is the `log` target; `heronite_` is
    /// dropped from it for the prefix column).
    pub fn record(&self, level: Level, target: &str, message: &str) {
        let prefix = target.strip_prefix("heronite_").unwrap_or(target);
        self.write_line(level, prefix, message, false);
    }

    /// A free-form line without a prefix (session markers).
    pub fn note(&self, message: &str) {
        self.write_line(Level::Info, "", message, true);
    }

    fn write_line(&self, level: Level, prefix: &str, message: &str, force: bool) {
        if self.disabled && !force {
            return;
        }
        let now = chrono::Local::now();
        let mut text = format_line(now.time(), level, prefix, message);
        if text.is_empty() {
            return;
        }
        text.push('\n');
        let mut st = self.state.lock().unwrap();
        if st.date != now.date_naive() || st.written >= MAX_LAUNCHER_LOG_BYTES {
            st.date = now.date_naive();
            self.rotate(&mut st);
        }
        let st = &mut *st;
        if let Some(f) = st.file.as_mut() {
            if f.write_all(text.as_bytes()).is_ok() {
                st.written += text.len() as u64;
            }
        }
    }
}

/// Deletes all but the newest `keep` `heronite-rs-*.log` files (and their
/// `.old`).
pub fn prune_launcher_logs(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with("heronite-rs-") && n.ends_with(".log"))
        .collect();
    names.sort();
    names.reverse();
    for name in names.into_iter().skip(keep) {
        let path = dir.join(&name);
        let _ = std::fs::remove_file(old_path(&path));
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::DataPaths;

    #[test]
    fn launcher_log_appends_prunes_and_respects_disable() {
        let dir = tempfile::tempdir().unwrap();
        let paths = LogPaths { base: dir.path().join("base"), launcher_dir: dir.path().join("l") };
        std::fs::create_dir_all(&paths.launcher_dir).unwrap();
        for d in 1..=9 {
            std::fs::write(paths.launcher_dir.join(format!("heronite-rs-2020-01-0{d}.log")), "x").unwrap();
        }
        std::fs::write(paths.launcher_dir.join("keep-me.txt"), "x").unwrap();
        let logger = LauncherLogger::open(paths.clone(), false);
        logger.record(Level::Info, "heronite_launch::runtime", "hello");
        logger.record(Level::Error, "wgpu", "bad");
        let today = paths.launcher_log(chrono::Local::now().date_naive());
        let text = std::fs::read_to_string(&today).unwrap();
        assert!(text.contains("[INFO]:    [launch::runtime]: hello\n"), "{text}");
        assert!(text.contains("[ERROR]:   [wgpu]:") && text.contains("bad"));
        let kept = crate::log_reader::list_logs(&paths, &DataPaths::at(dir.path().join("h"))).len();
        assert_eq!(kept, KEEP_LAUNCHER_LOGS);
        assert!(paths.launcher_dir.join("keep-me.txt").exists());
        assert!(!paths.launcher_dir.join("heronite-rs-2020-01-01.log").exists());

        let paths2 = LogPaths { base: dir.path().join("b2"), launcher_dir: dir.path().join("l2") };
        let off = LauncherLogger::open(paths2.clone(), true);
        off.record(Level::Info, "heronite_x", "secret chatter");
        let text = std::fs::read_to_string(paths2.launcher_log(chrono::Local::now().date_naive())).unwrap();
        assert!(text.contains("Logs are disabled") && !text.contains("chatter"));
    }
}
