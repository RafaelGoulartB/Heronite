//! Running the `legendary` CLI the way Heroic's `callRunner` does:
//! `LEGENDARY_CONFIG_PATH` points at the data folder's legendary config, arguments are
//! passed as-is and output (stdout and stderr) is captured, or streamed line by
//! line for long operations that can be cancelled.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use heronite_data::proc::Run;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::{legendary::find_legendary, PlanContext, SystemProbe};

/// The legendary binary and the config folder it works on.
#[derive(Debug, Clone)]
pub struct Legendary {
    pub bin: PathBuf,
    /// `<data>/legendaryConfig/legendary`.
    pub config_dir: PathBuf,
}

/// Result of a finished run.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    /// The run was stopped through its [`Cancel`].
    pub aborted: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.aborted
    }

    /// `ERROR:` / `CRITICAL:` lines of legendary's log, for messages.
    pub fn error_lines(&self) -> String {
        let all = format!("{}\n{}", self.stderr, self.stdout);
        let lines: Vec<&str> = all.lines().filter(|l| l.contains("ERROR") || l.contains("CRITICAL") || l.contains("Traceback")).collect();
        lines.join("\n")
    }
}

pub use heronite_data::proc::Cancel;

impl Legendary {
    /// Finds the binary (`altLegendaryBin`, `<data>/tools/bin/legendary`,
    /// `$PATH`; `HERONITE_LEGENDARY_BIN` overrides all).
    pub fn locate(data_paths: &DataPaths) -> Result<Self> {
        let ctx = PlanContext::load(data_paths);
        let bin = find_legendary(&ctx, &SystemProbe).ok_or_else(|| anyhow!(tr!("The legendary binary was not found")))?;
        Ok(Self { bin: PathBuf::from(bin), config_dir: data_paths.legendary_config_dir() })
    }

    /// [`Self::locate`], downloading legendary into the data folder first when
    /// none is found (a new setup). Blocking, network.
    pub fn locate_or_install(data_paths: &DataPaths) -> Result<Self> {
        let bin = heronite_launch::legendary_install::ensure(data_paths, &SystemProbe, &heronite_launch::net::SystemNet, &|| false)
            .map_err(|e| anyhow!(tr!("Could not get legendary, which Heronite uses for Epic Games: {error}", error = e)))?;
        Ok(Self { bin, config_dir: data_paths.legendary_config_dir() })
    }

    pub fn with_bin(bin: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into(), config_dir: config_dir.into() }
    }

    fn command(&self, args: &[String], env: &[(String, String)]) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        if let Some(dir) = self.bin.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.env("LEGENDARY_CONFIG_PATH", &self.config_dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }

    /// Runs to completion and returns the output. Blocking.
    pub fn run<S: AsRef<str>>(&self, args: &[S], env: &[(String, String)]) -> Result<Output> {
        self.run_streaming(args, env, &Cancel::new(), |_| {})
    }

    /// Like [`run`](Self::run) but calls `on_line` for every output line (both
    /// streams, split on `\n` and `\r`) while the command runs, and stops it
    /// when `cancel` is requested.
    pub fn run_streaming<S: AsRef<str>>(
        &self,
        args: &[S],
        env: &[(String, String)],
        cancel: &Cancel,
        on_line: impl FnMut(&str),
    ) -> Result<Output> {
        let args: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        log::info!("legendary {}", redact(&args).join(" "));
        let mut cmd = self.command(&args, env);
        // SIGTERM only legendary (it saves its resume data); the workers it
        // left behind are killed with the group afterwards.
        let mut on_line = on_line;
        let out = Run::new()
            .cancel(cancel)
            .term_main_only()
            .line_capture()
            .on_line(&mut on_line)
            .run(&mut cmd)
            .with_context(|| format!("starting {}", self.bin.display()))?;
        Ok(Output { stdout: out.stdout_text(), stderr: out.stderr_text(), code: out.code, aborted: out.aborted })
    }
}

/// Arguments for the log with the authorization code hidden.
pub fn redact(args: &[String]) -> Vec<String> {
    let mut out = args.to_vec();
    for (i, a) in args.iter().enumerate() {
        if (a == "--code" || a == "--token" || a == "--sid") && i + 1 < out.len() {
            out[i + 1] = "<redacted>".into();
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::path::Path;

    pub use heronite_data::testing::{FakeTool, Reply};

    /// A fake `legendary` in `dir` that records the config path it gets
    /// (`heronite_data::testing::FakeTool`, the same on every system): tell
    /// it what to answer with `.on(...)` / `.otherwise(...)`, then run it
    /// through [`legendary_of`].
    pub fn fake_tool(dir: &Path) -> FakeTool {
        FakeTool::new(dir, "legendary").record_env("LEGENDARY_CONFIG_PATH")
    }

    /// A [`Legendary`] running `tool`, with `<dir>/cfg` as its config folder.
    pub fn legendary_of(tool: &FakeTool, dir: &Path) -> Legendary {
        Legendary::with_bin(tool.bin(), dir.join("cfg"))
    }

    /// A fake `legendary`: a shell script that records its arguments and the
    /// config path, then runs `body`. Unix only: it needs `sh`.
    #[cfg(unix)]
    pub fn fake(dir: &Path, body: &str) -> Legendary {
        let bin = dir.join("legendary");
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> \"{log}\"\necho \"CFG=$LEGENDARY_CONFIG_PATH\" >> \"{log}\"\n{body}\n",
            log = dir.join("calls.log").display()
        );
        std::fs::write(&bin, script).unwrap();
        heronite_platform::fs::make_executable(&bin).unwrap();
        Legendary::with_bin(bin, dir.join("cfg"))
    }

    #[cfg(unix)]
    pub fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with("CFG="))
            .map(str::to_string)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn captures_both_streams_and_passes_the_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(dir.path()).otherwise(Reply::new().out("out").err("err").exit(3));
        let out = legendary_of(&tool, dir.path()).run(&["list", "--third-party"], &[]).unwrap();
        assert_eq!(out.code, Some(3));
        assert!(!out.success());
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
        assert_eq!(tool.calls(), ["list --third-party"]);
        assert_eq!(tool.env_seen("LEGENDARY_CONFIG_PATH"), [dir.path().join("cfg").display().to_string()]);
    }

    #[test]
    fn streams_lines_split_on_carriage_returns() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(dir.path()).otherwise(Reply::new().err_raw("a\rb\nc"));
        let mut lines = Vec::new();
        let out = legendary_of(&tool, dir.path()).run_streaming(&["x"], &[], &Cancel::new(), |l| lines.push(l.to_string())).unwrap();
        assert!(out.success());
        assert_eq!(lines, ["a", "b", "c"]);
    }

    #[test]
    fn cancel_stops_a_running_tool() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(dir.path()).otherwise(Reply::new().out("started").sleep(Duration::from_secs(60)));
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let started = Instant::now();
        let out = legendary_of(&tool, dir.path()).run_streaming(&["install"], &[], &cancel, |_| c2.request()).unwrap();
        assert!(out.aborted);
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn cancel_stops_the_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let lg = fake(dir.path(), "echo started; sleep 60 & wait");
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c2.request();
        });
        let started = Instant::now();
        let out = lg.run_streaming(&["install"], &[], &cancel, |_| {}).unwrap();
        t.join().unwrap();
        assert!(out.aborted);
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn missing_binary_is_an_error() {
        let lg = Legendary::with_bin("/nonexistent/legendary", "/tmp/x");
        assert!(lg.run(&["list"], &[]).is_err());
    }

    #[test]
    fn codes_are_redacted_in_logs() {
        let args: Vec<String> = ["auth", "--code", "secret"].iter().map(|s| s.to_string()).collect();
        assert_eq!(redact(&args), ["auth", "--code", "<redacted>"]);
    }
}
