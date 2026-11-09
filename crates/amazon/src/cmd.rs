//! Running the `nile` CLI: `NILE_CONFIG_PATH` points at the data folder's nile
//! config (nile keeps its files in `nile/` under it), arguments are passed
//! as-is and output (stdout and stderr) is captured, or streamed line by line
//! for long operations that can be cancelled.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use heronite_data::proc::Run;
use heronite_data::DataPaths;
use heronite_epic::cmd::Output;
use heronite_i18n::tr;
use heronite_launch::store_tools::NILE;
use heronite_launch::{PlanContext, SystemProbe};

pub use heronite_epic::cmd::Cancel;

/// Time nile gets on Windows between the Ctrl+C and the kill.
const WINDOWS_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// The nile binary and the config folder it works on.
#[derive(Debug, Clone)]
pub struct Nile {
    pub bin: PathBuf,
    /// `NILE_CONFIG_PATH`: `<data>/nile_config`.
    pub config_dir: PathBuf,
}

impl Nile {
    /// Finds the binary (`altNileBin`, `<data>/tools/bin/nile`, `$PATH`;
    /// `HERONITE_NILE_BIN` overrides all).
    pub fn locate(paths: &DataPaths) -> Result<Self> {
        let bin = NILE.find(&PlanContext::load(paths), &SystemProbe).ok_or_else(|| anyhow!(tr!("nile was not found")))?;
        Ok(Self::with_bin(bin, paths))
    }

    /// [`Self::locate`], downloading nile into the data folder first when none
    /// is found (a new setup). Blocking, network.
    pub fn locate_or_install(paths: &DataPaths) -> Result<Self> {
        let bin = NILE
            .ensure(paths, &SystemProbe, &heronite_launch::net::SystemNet, &|| false)
            .map_err(|e| anyhow!(tr!("Could not get nile, which Heronite uses for Amazon Games: {error}", error = e)))?;
        Ok(Self::with_bin(bin, paths))
    }

    pub fn with_bin(bin: impl Into<PathBuf>, paths: &DataPaths) -> Self {
        Self { bin: bin.into(), config_dir: paths.nile_config_dir() }
    }

    /// `<config>/nile`, where nile keeps `library.json`, `installed.json`...
    pub fn data_dir(&self) -> PathBuf {
        self.config_dir.join("nile")
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        // Some builds look for files next to themselves.
        if let Some(dir) = self.bin.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.env("NILE_CONFIG_PATH", &self.config_dir);
        cmd
    }

    /// Runs to completion and returns the output. Blocking.
    pub fn run<S: AsRef<str>>(&self, args: &[S]) -> Result<Output> {
        self.run_streaming(args, &Cancel::new(), |_| {})
    }

    /// Like [`run`](Self::run) but calls `on_line` for every output line (both
    /// streams, split on `\n` and `\r`) while the command runs, and stops it
    /// when `cancel` is requested.
    pub fn run_streaming<S: AsRef<str>>(&self, args: &[S], cancel: &Cancel, on_line: impl FnMut(&str)) -> Result<Output> {
        let args: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        log::info!("nile {}", redact(&args).join(" "));
        let mut cmd = self.command(&args);
        let mut on_line = on_line;
        // SIGTERM only nile; the download workers it left behind are killed
        // with the process group afterwards.
        let run = Run::new().cancel(cancel).term_main_only();
        // On Windows nile's main thread stops at the Ctrl+C but its download
        // threads go on until the kill, and the next run checks the files
        // anyway: a pause need not wait for them.
        let run = if cfg!(windows) { run.grace(WINDOWS_GRACE) } else { run };
        let out = run.line_capture().on_line(&mut on_line).run(&mut cmd).with_context(|| format!("starting {}", self.bin.display()))?;
        Ok(Output { stdout: out.stdout_text(), stderr: out.stderr_text(), code: out.code, aborted: out.aborted })
    }
}

/// Arguments for the log with the sign-in secrets hidden.
pub fn redact(args: &[String]) -> Vec<String> {
    let mut out = args.to_vec();
    for (i, a) in args.iter().enumerate() {
        if matches!(a.as_str(), "--code" | "--code-verifier") && i + 1 < out.len() {
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

    /// A temp data folder with a fake nile that can be pointed at it.
    pub struct Env {
        pub dir: tempfile::TempDir,
        pub paths: DataPaths,
    }

    impl Env {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::at(dir.path().join("data"));
            Self { dir, paths }
        }

        pub fn root(&self) -> &Path {
            self.dir.path()
        }

        /// A fake `nile` in `<root>/bin` that records the config path it
        /// gets (`heronite_data::testing::FakeTool`, the same on every
        /// system): tell it what to answer with `.on(...)` /
        /// `.otherwise(...)`, then run it through [`Env::nile_of`].
        /// `${NILE_CONFIG_PATH}` in what it writes is the folder it got.
        pub fn fake_tool(&self) -> FakeTool {
            FakeTool::new(&self.root().join("bin"), "nile").record_env("NILE_CONFIG_PATH")
        }

        /// A [`Nile`] running `tool` on this data folder.
        pub fn nile_of(&self, tool: &FakeTool) -> Nile {
            Nile::with_bin(tool.bin(), &self.paths)
        }

        /// A fake `nile` (the [`Env::fake_tool`]) answering every call with
        /// `reply`. A new one replaces the last; the calls of all of them are
        /// in [`Env::calls`].
        pub fn nile(&self, reply: Reply) -> Nile {
            let tool = self.fake_tool().otherwise(reply);
            self.nile_of(&tool)
        }

        /// Arguments of every call to the fake nile.
        pub fn calls(&self) -> Vec<String> {
            let log = self.root().join("bin").join("nile.calls");
            std::fs::read_to_string(log).unwrap_or_default().lines().filter(|l| !l.starts_with("env ")).map(str::to_string).collect()
        }

        /// A fake `nile`: a shell script that runs `body`. Unix only: it
        /// needs `sh` (for a tool that leaves processes behind).
        #[cfg(unix)]
        pub fn nile_sh(&self, body: &str) -> Nile {
            let bin = self.root().join("sh").join("nile");
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
            heronite_platform::fs::make_executable(&bin).unwrap();
            Nile::with_bin(bin, &self.paths)
        }

        /// `<config>/nile`.
        pub fn nile_dir(&self) -> PathBuf {
            self.paths.nile_config_dir().join("nile")
        }

        /// Writes one of nile's files (`library.json`, `installed.json`...).
        pub fn write_nile_file(&self, name: &str, content: &str) {
            let path = self.nile_dir().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn captures_both_streams_and_passes_the_config_path() {
        let env = Env::new();
        let tool = env.fake_tool().otherwise(Reply::new().out("out").err("err").exit(3));
        let nile = env.nile_of(&tool);
        let out = nile.run(&["library", "sync"]).unwrap();
        assert_eq!(out.code, Some(3));
        assert!(!out.success());
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
        assert_eq!(tool.calls(), ["library sync"]);
        assert_eq!(tool.env_seen("NILE_CONFIG_PATH"), [env.paths.nile_config_dir().display().to_string()]);
        assert_eq!(nile.data_dir(), env.paths.nile_config_dir().join("nile"));
    }

    #[test]
    fn streams_lines_split_on_carriage_returns() {
        let env = Env::new();
        let tool = env.fake_tool().otherwise(Reply::new().err_raw("a\rb\nc"));
        let nile = env.nile_of(&tool);
        let mut lines = Vec::new();
        let out = nile.run_streaming(&["x"], &Cancel::new(), |l| lines.push(l.to_string())).unwrap();
        assert!(out.success());
        assert_eq!(lines, ["a", "b", "c"]);
    }

    #[test]
    fn cancel_stops_a_running_tool() {
        let env = Env::new();
        let tool = env.fake_tool().on(&["install"], Reply::new().out("started").sleep(Duration::from_secs(60)));
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let started = Instant::now();
        let out = env.nile_of(&tool).run_streaming(&["install"], &cancel, |_| c2.request()).unwrap();
        assert!(out.aborted);
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn cancel_stops_the_process_group() {
        let env = Env::new();
        let nile = env.nile_sh("echo started; sleep 60 & wait");
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c2.request();
        });
        let started = Instant::now();
        let out = nile.run_streaming(&["install"], &cancel, |_| {}).unwrap();
        t.join().unwrap();
        assert!(out.aborted);
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn missing_binary_is_an_error() {
        let env = Env::new();
        let nile = Nile::with_bin("/nonexistent/nile", &env.paths);
        assert!(nile.run(&["library"]).is_err());
    }

    #[test]
    fn sign_in_secrets_are_redacted_in_logs() {
        let args: Vec<String> =
            ["register", "--code", "secret", "--code-verifier", "v", "--serial", "s"].iter().map(|s| s.to_string()).collect();
        assert_eq!(redact(&args), ["register", "--code", "<redacted>", "--code-verifier", "<redacted>", "--serial", "s"]);
    }
}
