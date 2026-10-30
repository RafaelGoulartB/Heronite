//! Running the `gogdl` CLI: `<bin> --auth-config-path <auth.json> <command> ...`
//! with `GOGDL_CONFIG_PATH` pointing at the data folder. Output is captured,
//! or streamed line by line for long operations that can be cancelled.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use heronite_data::proc::Run;
use heronite_data::DataPaths;
use heronite_epic::cmd::Output;
use heronite_i18n::tr;
use heronite_launch::net::SystemNet;
use heronite_launch::store_tools::GOGDL;
use heronite_launch::{PlanContext, SystemProbe};

pub use heronite_epic::cmd::Cancel;

/// Folder gogdl itself creates under `GOGDL_CONFIG_PATH`; the name is fixed by
/// the tool (its manifests and support files live below it).
const TOOL_DIR: &str = "heroic_gogdl";

/// The gogdl binary and the files it works on.
#[derive(Debug, Clone)]
pub struct Gogdl {
    pub bin: PathBuf,
    /// `<data>/gog_store/auth.json` (`--auth-config-path`).
    pub auth_file: PathBuf,
    /// `<data>/gogdlConfig` (`GOGDL_CONFIG_PATH`).
    pub config_dir: PathBuf,
}

impl Gogdl {
    /// Finds the binary (`altGogdlBin`, `<data>/tools/bin/gogdl`, `$PATH`;
    /// `HERONITE_GOGDL_BIN` overrides all).
    pub fn locate(paths: &DataPaths) -> Result<Self> {
        let found = GOGDL.find(&PlanContext::load(paths), &SystemProbe).ok_or_else(|| anyhow!(tr!("gogdl was not found")))?;
        Ok(Self::with_bin(found, paths))
    }

    /// [`Self::locate`], downloading gogdl into the data folder when none is
    /// found. Blocking, network.
    pub fn locate_or_install(paths: &DataPaths) -> Result<Self> {
        let bin = GOGDL
            .ensure(paths, &SystemProbe, &SystemNet, &|| false)
            .map_err(|e| anyhow!(tr!("Could not get gogdl, which Heronite uses for GOG: {error}", error = e)))?;
        Ok(Self::with_bin(bin, paths))
    }

    pub fn with_bin(bin: impl Into<PathBuf>, paths: &DataPaths) -> Self {
        Self { bin: bin.into(), auth_file: paths.gog_auth(), config_dir: paths.gogdl_config_dir() }
    }

    /// gogdl's manifest of an installed game (written when a download ends).
    pub fn manifest_file(&self, app_name: &str) -> PathBuf {
        manifest_file(&self.config_dir, app_name)
    }

    /// Folder with the game's support files (installer scripts).
    pub fn support_dir(&self, app_name: &str) -> PathBuf {
        support_dir(&self.config_dir, app_name)
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("--auth-config-path").arg(&self.auth_file).args(args);
        if let Some(dir) = self.bin.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.env("GOGDL_CONFIG_PATH", &self.config_dir);
        cmd
    }

    /// Runs to completion and returns the output. Blocking.
    pub fn run<S: AsRef<str>>(&self, args: &[S]) -> Result<Output> {
        self.run_streaming(args, &Cancel::new(), |_| {})
    }

    /// Like [`run`](Self::run) but calls `on_line` for every output line (both
    /// streams) while the command runs, and stops it when `cancel` is requested.
    pub fn run_streaming<S: AsRef<str>>(&self, args: &[S], cancel: &Cancel, on_line: impl FnMut(&str)) -> Result<Output> {
        let args: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        log::info!("gogdl {}", redact(&args).join(" "));
        // gogdl writes the auth file without creating its folder.
        if let Some(dir) = self.auth_file.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let mut cmd = self.command(&args);
        // SIGTERM only gogdl (it leaves `.gogdl-resume` behind); its workers
        // are killed with the group afterwards.
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

/// [`Gogdl::manifest_file`] for a `GOGDL_CONFIG_PATH`.
pub fn manifest_file(config_dir: &Path, app_name: &str) -> PathBuf {
    config_dir.join(TOOL_DIR).join("manifests").join(app_name)
}

/// [`Gogdl::support_dir`] for a `GOGDL_CONFIG_PATH`.
pub fn support_dir(config_dir: &Path, app_name: &str) -> PathBuf {
    config_dir.join(TOOL_DIR).join("gog-support").join(app_name)
}

/// Arguments for the log with the authorization code hidden.
pub fn redact(args: &[String]) -> Vec<String> {
    let mut out = args.to_vec();
    for (i, a) in args.iter().enumerate() {
        if a == "--code" && i + 1 < out.len() {
            out[i + 1] = "<redacted>".into();
        }
    }
    out
}

/// The last line of `text` that is a JSON object (gogdl prints its logs on
/// stderr, but a few messages land on stdout next to the result).
pub fn last_json_object(text: &str) -> Option<serde_json::Value> {
    text.lines().rev().map(str::trim).filter(|l| l.starts_with('{')).find_map(|l| serde_json::from_str(l).ok())
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    #[cfg(unix)]
    use std::path::Path;

    /// A fake `gogdl`: a shell script that records its arguments and the config
    /// path, then runs `body`. Unix only: it needs `sh`.
    #[cfg(unix)]
    pub fn fake(dir: &Path, paths: &DataPaths, body: &str) -> Gogdl {
        let bin = dir.join("gogdl");
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> \"{log}\"\necho \"CFG=$GOGDL_CONFIG_PATH\" >> \"{log}\"\n{body}\n",
            log = dir.join("calls.log").display()
        );
        std::fs::write(&bin, script).unwrap();
        heronite_platform::fs::make_executable(&bin).unwrap();
        Gogdl::with_bin(bin, paths)
    }

    /// The recorded command lines, without the `--auth-config-path <file>` prefix.
    #[cfg(unix)]
    pub fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with("CFG="))
            .map(|l| {
                let mut parts = l.splitn(3, ' ');
                assert_eq!(parts.next(), Some("--auth-config-path"));
                parts.next();
                parts.next().unwrap_or("").to_string()
            })
            .collect()
    }

    pub fn setup() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        (dir, paths)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    #[test]
    fn passes_the_auth_file_and_config_path() {
        let (dir, paths) = setup();
        let g = fake(dir.path(), &paths, "echo out; echo err >&2; exit 3");
        let out = g.run(&["info", "123"]).unwrap();
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
        assert_eq!(calls(dir.path()), ["info 123"]);
        let log = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
        assert!(log.contains(&format!("--auth-config-path {}", paths.gog_auth().display())), "{log}");
        assert!(log.contains(&format!("CFG={}", paths.gogdl_config_dir().display())), "{log}");
        // gogdl does not create the auth file's folder itself.
        assert!(paths.gog_auth().parent().unwrap().is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn streams_lines_and_cancels() {
        let (dir, paths) = setup();
        let g = fake(dir.path(), &paths, "printf 'a\\rb\\n' >&2; echo started; sleep 60 & wait");
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            c2.request();
        });
        let mut lines = Vec::new();
        let started = Instant::now();
        let out = g.run_streaming(&["download"], &cancel, |l| lines.push(l.to_string())).unwrap();
        t.join().unwrap();
        assert!(out.aborted);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(lines.contains(&"a".to_string()) && lines.contains(&"b".to_string()), "{lines:?}");
    }

    #[test]
    fn missing_binary_is_an_error() {
        let (_dir, paths) = setup();
        assert!(Gogdl::with_bin("/nonexistent/gogdl", &paths).run(&["info"]).is_err());
    }

    #[test]
    fn codes_are_redacted_and_json_lines_found() {
        let args: Vec<String> = ["auth", "--code", "secret"].iter().map(|s| s.to_string()).collect();
        assert_eq!(redact(&args), ["auth", "--code", "<redacted>"]);
        let v = last_json_object("noise\n{\"a\": 1}\nmore\n").unwrap();
        assert_eq!(v["a"], 1);
        assert!(last_json_object("null\n").is_none());
    }

    #[test]
    fn manifest_and_support_paths() {
        let (_dir, paths) = setup();
        let g = Gogdl::with_bin("/x/gogdl", &paths);
        assert!(g.manifest_file("42").ends_with(format!("gogdlConfig/{TOOL_DIR}/manifests/42")));
        assert!(g.support_dir("42").ends_with(format!("gogdlConfig/{TOOL_DIR}/gog-support/42")));
    }
}
