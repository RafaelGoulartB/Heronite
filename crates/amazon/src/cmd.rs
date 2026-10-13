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
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

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

        /// A fake `nile`: a shell script that records its arguments and the
        /// config path, then runs `body`.
        pub fn nile(&self, body: &str) -> Nile {
            let bin = self.root().join("bin").join("nile");
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            let script = format!(
                "#!/bin/sh\necho \"$@\" >> \"{log}\"\necho \"CFG=$NILE_CONFIG_PATH\" >> \"{log}\"\n{body}\n",
                log = self.root().join("calls.log").display()
            );
            std::fs::write(&bin, script).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            Nile::with_bin(bin, &self.paths)
        }

        /// Arguments of every call to the fake nile.
        pub fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.root().join("calls.log"))
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.starts_with("CFG="))
                .map(str::to_string)
                .collect()
        }

        pub fn call_log(&self) -> String {
            std::fs::read_to_string(self.root().join("calls.log")).unwrap_or_default()
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
        let nile = env.nile("echo out; echo err >&2; exit 3");
        let out = nile.run(&["library", "sync"]).unwrap();
        assert_eq!(out.code, Some(3));
        assert!(!out.success());
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
        assert_eq!(env.calls(), ["library sync"]);
        assert!(env.call_log().contains(&format!("CFG={}", env.paths.nile_config_dir().display())));
        assert_eq!(nile.data_dir(), env.paths.nile_config_dir().join("nile"));
    }

    #[test]
    fn streams_lines_split_on_carriage_returns() {
        let env = Env::new();
        let nile = env.nile("printf 'a\\rb\\nc' >&2");
        let mut lines = Vec::new();
        let out = nile.run_streaming(&["x"], &Cancel::new(), |l| lines.push(l.to_string())).unwrap();
        assert!(out.success());
        assert_eq!(lines, ["a", "b", "c"]);
    }

    #[test]
    fn cancel_stops_the_process_group() {
        let env = Env::new();
        let nile = env.nile("echo started; sleep 60 & wait");
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
