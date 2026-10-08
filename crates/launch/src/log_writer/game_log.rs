//! [`GameLog`]: one log file of one run, written like Heroic's `LogWriter`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{format_line, old_path, Level};

/// Last line of a finished log.
pub const END_OF_LOG: &str = "============= End of log =============";

/// The launcher's own lines stop at this size. (The game's output goes
/// straight into the file and is not limited.)
pub const MAX_LOG_BYTES: u64 = 512 * 1024 * 1024;

struct State {
    file: Option<File>,
    /// The first write archives the previous log; true once done.
    started: bool,
    written: u64,
    truncated: bool,
    closed: bool,
    failed: bool,
}

/// One log file of one run (Heroic's `LogWriter`), safe to share between the
/// launch thread and the output pump.
pub struct GameLog {
    path: PathBuf,
    /// Lines that are not forced are dropped (`disableLogs` / no `verboseLogs`).
    disabled: bool,
    state: Mutex<State>,
}

impl GameLog {
    pub fn new(path: PathBuf, disabled: bool) -> Arc<Self> {
        Arc::new(Self {
            path,
            disabled,
            state: Mutex::new(State { file: None, started: false, written: 0, truncated: false, closed: false, failed: false }),
        })
    }

    /// A log that writes nothing (tests, callers without a place for logs).
    pub fn null() -> Arc<Self> {
        Self::new(PathBuf::new(), true)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn debug(&self, message: &str) {
        self.line(Level::Debug, message, false)
    }
    pub fn info(&self, message: &str) {
        self.line(Level::Info, message, false)
    }
    pub fn warning(&self, message: &str) {
        self.line(Level::Warning, message, false)
    }
    pub fn error(&self, message: &str) {
        self.line(Level::Error, message, false)
    }
    /// Written even when logs are disabled (Heroic's `forceLog`).
    pub fn forced_warning(&self, message: &str) {
        self.line(Level::Warning, message, true)
    }

    fn line(&self, level: Level, message: &str, force: bool) {
        if message.trim().is_empty() {
            return;
        }
        let mut text = format_line(chrono::Local::now().time(), level, "", message);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.write(text.as_bytes(), force);
    }

    /// Where a game's stdout / stderr should go: a descriptor of the log file
    /// itself (shared `O_APPEND` description, so launcher and game lines
    /// interleave without overwriting each other). `Stdio::null()` when the
    /// log is disabled, closed or unwritable. Creates the file (archiving the
    /// previous one) if nothing was written yet.
    pub fn child_stdio(&self) -> std::process::Stdio {
        use std::process::Stdio;
        if self.path.as_os_str().is_empty() || self.disabled {
            return Stdio::null();
        }
        let mut st = self.state.lock().unwrap();
        if st.closed || st.failed {
            return Stdio::null();
        }
        if st.file.is_none() && !st.started {
            st.started = true;
            match self.open_new() {
                Ok(f) => st.file = Some(f),
                Err(e) => {
                    st.failed = true;
                    log::warn!("cannot write the log {}: {e}", self.path.display());
                    return Stdio::null();
                }
            }
        }
        match st.file.as_ref().map(File::try_clone) {
            Some(Ok(f)) => Stdio::from(f),
            _ => Stdio::null(),
        }
    }

    /// Raw output of the game (stdout / stderr chunks), stored as it comes.
    /// (Only for output that reaches the launcher; games write to the file
    /// directly, see [`GameLog::child_stdio`].)
    pub fn raw(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.write(bytes, false);
        }
    }

    /// Writes the final line and stops accepting writes.
    pub fn close(&self) {
        let has_file = {
            let st = self.state.lock().unwrap();
            if st.closed {
                return;
            }
            st.file.is_some()
        };
        // A log that exists (something was written) always gets its end.
        self.write(format!("{END_OF_LOG}\n").as_bytes(), has_file);
        let mut st = self.state.lock().unwrap();
        st.closed = true;
        st.file = None;
    }

    fn write(&self, data: &[u8], force: bool) {
        if self.path.as_os_str().is_empty() || (self.disabled && !force) {
            return;
        }
        let mut st = self.state.lock().unwrap();
        if st.closed || st.failed {
            return;
        }
        if st.file.is_none() && !st.started {
            st.started = true;
            match self.open_new() {
                Ok(f) => st.file = Some(f),
                Err(e) => {
                    st.failed = true;
                    log::warn!("cannot write the log {}: {e}", self.path.display());
                    return;
                }
            }
        }
        let st = &mut *st;
        let Some(file) = st.file.as_mut() else { return };
        if st.written + data.len() as u64 > MAX_LOG_BYTES {
            if !st.truncated {
                st.truncated = true;
                let note = format!("\n[log truncated: the size limit of {} MB was reached; the rest of the output is not stored]\n", MAX_LOG_BYTES / 1024 / 1024);
                let _ = file.write_all(note.as_bytes());
            }
            return;
        }
        if file.write_all(data).is_err() {
            st.failed = true;
            return;
        }
        st.written += data.len() as u64;
    }

    /// Renames the previous log to `.old` and creates the new file.
    fn open_new(&self) -> std::io::Result<File> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if self.path.exists() {
            std::fs::rename(&self.path, old_path(&self.path))?;
        }
        OpenOptions::new().create(true).append(true).open(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_writer::write_disabled_notice;

    #[test]
    fn a_new_run_archives_the_previous_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("games/g_sideload/launch.log");
        let first = GameLog::new(path.clone(), false);
        first.info("first run");
        first.raw(b"game says hi\n");
        first.close();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("first run") && text.contains("game says hi\n"));
        assert!(text.ends_with(&format!("{END_OF_LOG}\n")), "{text}");
        assert!(!old_path(&path).exists());

        let second = GameLog::new(path.clone(), false);
        second.info("second run");
        second.close();
        assert!(std::fs::read_to_string(old_path(&path)).unwrap().contains("first run"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("second run") && !text.contains("first run"));
        // Nothing is archived before the first write.
        let third = GameLog::new(path.clone(), false);
        assert!(std::fs::read_to_string(&path).unwrap().contains("second run"));
        drop(third);
    }

    #[test]
    fn disabled_logs_only_take_forced_lines_and_then_still_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("launch.log");
        let log = GameLog::new(path.clone(), true);
        log.info("hidden");
        log.raw(b"hidden output");
        assert!(!path.exists(), "no file and no rotation when nothing is written");
        write_disabled_notice(&log);
        log.info("still hidden");
        log.close();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[WARNING]: IMPORTANT: Logs are disabled"), "{text}");
        assert!(!text.contains("hidden output") && !text.contains("still hidden"));
        assert!(text.contains(END_OF_LOG));

        // Fully disabled and nothing forced: the file is never created.
        let path2 = dir.path().join("other.log");
        let quiet = GameLog::new(path2.clone(), true);
        quiet.info("x");
        quiet.close();
        assert!(!path2.exists());
    }

    #[test]
    fn writes_after_close_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("l.log");
        let log = GameLog::new(path.clone(), false);
        log.info("a");
        log.close();
        log.raw(b"late output");
        log.close();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("late output"));
        assert_eq!(text.matches(END_OF_LOG).count(), 1);
    }
}
