//! Running the Ludusavi binary.

use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;
use heronite_i18n::tr;

pub const LUDUSAVI_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// Runs `binary args...`. A non-zero exit is `Ok` with its code (Ludusavi
/// exits 1 when some games have no saves yet still prints a useful JSON);
/// `Err` means the process could not run at all (spawn failure, timeout).
pub trait CommandRunner: Send + Sync {
    fn run(&self, binary: &str, args: &[String]) -> Result<CommandOutput, String>;
}

/// Real process execution with a timeout and, on Linux,
/// `/usr/bin:/usr/local/bin` appended to `PATH`.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    pub timeout: Duration,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self { timeout: LUDUSAVI_TIMEOUT }
    }
}

/// The `PATH` the program runs with: Linux adds the system folders (an
/// AppImage or a desktop session may leave them out); Windows keeps its own
/// (`;`-separated, nothing to add), so `None`.
fn search_path(current: Option<String>) -> Option<String> {
    (!cfg!(windows)).then(|| format!("{}:/usr/bin:/usr/local/bin", current.unwrap_or_default()))
}

impl CommandRunner for SystemRunner {
    fn run(&self, binary: &str, args: &[String]) -> Result<CommandOutput, String> {
        // Guarded helper: own process group, told to stop if the app dies, and
        // the whole group is killed on timeout.
        let mut cmd = Command::new(binary);
        cmd.args(args);
        if let Some(path) = search_path(std::env::var("PATH").ok()) {
            cmd.env("PATH", path);
        }
        let out = Run::new().timeout(self.timeout).grace(Duration::from_secs(2)).run(&mut cmd).map_err(|e| e.to_string())?;
        if out.timed_out {
            return Err(tr!("{binary} timed out after {seconds}s", seconds = self.timeout.as_secs()));
        }
        Ok(CommandOutput { stdout: out.stdout_text(), stderr: out.stderr_text(), code: out.code.unwrap_or(1) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_runner_runs_and_times_out() {
        let r = SystemRunner::default();
        let ok = if cfg!(windows) {
            r.run("cmd", &["/C".into(), "echo hi& echo err 1>&2& exit 3".into()]).unwrap()
        } else {
            r.run("sh", &["-c".into(), "echo hi; echo err >&2; exit 3".into()]).unwrap()
        };
        assert_eq!((ok.stdout.trim(), ok.stderr.trim(), ok.code), ("hi", "err", 3));
        assert!(r.run("/nonexistent/binary", &[]).is_err());
        let fast = SystemRunner { timeout: Duration::from_millis(100) };
        let slow =
            if cfg!(windows) { fast.run("ping", &["-n".into(), "6".into(), "127.0.0.1".into()]) } else { fast.run("sleep", &["5".into()]) };
        assert!(slow.unwrap_err().contains("timed out"));
    }

    #[test]
    fn path_gets_the_system_folders_on_linux_only() {
        if cfg!(windows) {
            assert_eq!(search_path(Some(r"C:\Windows\system32;C:\Tools".into())), None);
        } else {
            assert_eq!(search_path(Some("/opt/bin".into())).as_deref(), Some("/opt/bin:/usr/bin:/usr/local/bin"));
            assert_eq!(search_path(None).as_deref(), Some(":/usr/bin:/usr/local/bin"));
        }
    }
}
