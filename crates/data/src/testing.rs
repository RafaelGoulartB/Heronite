//! Helpers for tests of other crates (the `testing` feature, turned on by
//! their `[dev-dependencies]`).
//!
//! [`FakeTool`]: a stand-in for legendary, gogdl, nile or any command-line
//! tool, the same on every system. It is a small program
//! (`testing/fake_tool.rs`, built once with `rustc` into the temp folder)
//! copied where the test wants the tool, which replays what the test told it
//! for the arguments it gets and logs its calls:
//!
//! ```ignore
//! let tool = FakeTool::new(dir, "legendary")
//!     .record_env("LEGENDARY_CONFIG_PATH")
//!     .on(&["info"], Reply::new().out(r#"{"game": {}}"#))
//!     .otherwise(Reply::new().err("[cli] ERROR: boom").exit(1));
//! let lg = Legendary::with_bin(tool.bin(), dir.join("cfg"));
//! // ... run the code under test ...
//! assert_eq!(tool.calls(), ["info Calluna --json", "install Calluna"]);
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const SOURCE: &str = include_str!("../testing/fake_tool.rs");

/// The fake tool program, built on first use (once per machine and version
/// of its source: it is kept in the temp folder).
pub fn fake_tool_program() -> &'static Path {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM.get_or_init(|| build_fake_tool().unwrap_or_else(|e| panic!("building the fake tool: {e}")))
}

fn fnv64(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// `rustc`: `$RUSTC`, else the one next to the `cargo` that built this
/// crate, else the one on `PATH`.
fn rustc() -> PathBuf {
    if let Some(r) = std::env::var_os("RUSTC") {
        return r.into();
    }
    let next_to_cargo = option_env!("CARGO").map(|c| Path::new(c).with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX)));
    next_to_cargo.filter(|p| p.is_file()).unwrap_or_else(|| "rustc".into())
}

fn build_fake_tool() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("heronite-fake-tool-{:016x}", fnv64(SOURCE)));
    let exe = dir.join(format!("fake-tool{}", std::env::consts::EXE_SUFFIX));
    if exe.is_file() {
        return Ok(exe);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Another test binary may build it at the same time: each in its own files.
    let pid = std::process::id();
    let src = dir.join(format!("fake_tool_{pid}.rs"));
    let out = dir.join(format!("fake-tool-{pid}{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&src, SOURCE).map_err(|e| format!("{}: {e}", src.display()))?;
    let output = Command::new(rustc())
        .args(["--edition", "2021", "--crate-name", "fake_tool", "-C", "opt-level=1", "-C", "debuginfo=0", "-o"])
        .arg(&out)
        .arg(&src)
        .output()
        .map_err(|e| format!("running rustc: {e}"))?;
    let _ = std::fs::remove_file(&src);
    if !output.status.success() {
        return Err(format!("rustc failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    if std::fs::rename(&out, &exe).is_err() && !exe.is_file() {
        return Err(format!("could not move {} to {}", out.display(), exe.display()));
    }
    let _ = std::fs::remove_file(&out);
    Ok(exe)
}

/// What the fake tool does for one kind of call, in order.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    actions: Vec<(&'static str, String)>,
}

impl Reply {
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes `line` and a line feed to stdout.
    pub fn out(self, line: &str) -> Self {
        self.out_raw(&format!("{line}\n"))
    }

    /// Writes `text` to stdout as it is (`\r` progress lines, no line feed).
    pub fn out_raw(mut self, text: &str) -> Self {
        self.actions.push(("out", escape(text)));
        self
    }

    /// Writes `line` and a line feed to stderr.
    pub fn err(self, line: &str) -> Self {
        self.err_raw(&format!("{line}\n"))
    }

    pub fn err_raw(mut self, text: &str) -> Self {
        self.actions.push(("err", escape(text)));
        self
    }

    /// Writes a file (its folders created). `${NAME}` in `path` or `text` is
    /// the tool's environment variable `NAME`, as it got it.
    pub fn write_file(mut self, path: impl AsRef<Path>, text: &str) -> Self {
        self.actions.push(("write", format!("{}\t{}", escape(&path.as_ref().to_string_lossy()), escape(text))));
        self
    }

    pub fn sleep(mut self, time: Duration) -> Self {
        self.actions.push(("sleep", time.as_millis().to_string()));
        self
    }

    /// Ends with `code` (the default is 0, after the last action).
    pub fn exit(mut self, code: i32) -> Self {
        self.actions.push(("exit", code.to_string()));
        self
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// A fake command-line tool in a test's folder (see the module docs).
#[derive(Debug)]
pub struct FakeTool {
    bin: PathBuf,
    env: Vec<String>,
    rules: Vec<(Vec<String>, Reply)>,
}

impl FakeTool {
    /// `<dir>/<name>` (`<name>.exe` on Windows), answering every call with
    /// nothing and exit code 0 until told otherwise. Its calls are logged
    /// in `<dir>/<name>.calls`.
    pub fn new(dir: &Path, name: &str) -> Self {
        let bin = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::create_dir_all(dir).unwrap();
        std::fs::copy(fake_tool_program(), &bin).unwrap_or_else(|e| panic!("copying the fake tool to {}: {e}", bin.display()));
        let tool = Self { bin, env: Vec::new(), rules: Vec::new() };
        tool.write_script();
        tool
    }

    /// The program to run.
    pub fn bin(&self) -> &Path {
        &self.bin
    }

    /// Also logs the value of the variable `name` the tool got, with each
    /// call ([`FakeTool::env_seen`]).
    pub fn record_env(mut self, name: &str) -> Self {
        self.env.push(name.to_string());
        self.write_script();
        self
    }

    /// Answers calls whose arguments include all of `args` (`"x*"`: one
    /// that starts with `x`) with `reply`. The first rule added that
    /// matches wins.
    pub fn on(mut self, args: &[&str], reply: Reply) -> Self {
        self.rules.push((args.iter().map(|a| a.to_string()).collect(), reply));
        self.write_script();
        self
    }

    /// Answers the calls no [`FakeTool::on`] rule took.
    pub fn otherwise(self, reply: Reply) -> Self {
        self.on(&[], reply)
    }

    fn log_file(&self) -> PathBuf {
        self.bin.with_extension("calls")
    }

    fn write_script(&self) {
        let mut script = format!("log {}\n", escape(&self.log_file().to_string_lossy()));
        for name in &self.env {
            script.push_str(&format!("env {name}\n"));
        }
        for (args, reply) in &self.rules {
            let needles: Vec<String> = args.iter().map(|a| escape(a)).collect();
            script.push_str(&format!("rule {}\n", needles.join("\t")));
            for (verb, value) in &reply.actions {
                script.push_str(&format!("{verb} {value}\n"));
            }
        }
        let file = self.bin.with_extension("fake");
        std::fs::write(&file, script).unwrap_or_else(|e| panic!("writing {}: {e}", file.display()));
    }

    fn log_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.log_file()).unwrap_or_default().lines().map(str::to_string).collect()
    }

    /// Each call's arguments, joined by spaces, in order.
    pub fn calls(&self) -> Vec<String> {
        self.log_lines().into_iter().filter(|l| !l.starts_with("env ")).collect()
    }

    /// The values of `name` the calls got ([`FakeTool::record_env`]).
    pub fn env_seen(&self, name: &str) -> Vec<String> {
        let prefix = format!("env {name}=");
        self.log_lines().into_iter().filter_map(|l| l.strip_prefix(&prefix).map(str::to_string)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(tool: &FakeTool, args: &[&str]) -> std::process::Output {
        let mut cmd = Command::new(tool.bin());
        cmd.args(args).env("FAKE_CFG", "/cfg dir");
        crate::proc::spawn(cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()))
            .unwrap()
            .wait_with_output()
            .unwrap()
    }

    #[test]
    fn replays_the_reply_of_the_first_matching_rule_and_logs_the_calls() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out").join("installed.json");
        let tool = FakeTool::new(dir.path(), "legendary")
            .record_env("FAKE_CFG")
            .on(&["info", "--json"], Reply::new().out(r#"{"a": "b\\c"}"#).err_raw("x\ry\tz"))
            .on(&["--code=*"], Reply::new().write_file(&file, "cfg=${FAKE_CFG}\n").exit(3))
            .otherwise(Reply::new().err("[cli] ERROR: boom").exit(1));
        assert!(tool.bin().ends_with(format!("legendary{}", std::env::consts::EXE_SUFFIX)));

        let out = run(&tool, &["info", "Calluna", "--json"]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "{\"a\": \"b\\\\c\"}\n");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "x\ry\tz");

        let out = run(&tool, &["auth", "--code=secret value"]);
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "cfg=/cfg dir\n");

        let out = run(&tool, &["list"]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(String::from_utf8_lossy(&out.stderr), "[cli] ERROR: boom\n");

        assert_eq!(tool.calls(), ["info Calluna --json", "auth --code=secret value", "list"]);
        assert_eq!(tool.env_seen("FAKE_CFG"), ["/cfg dir"; 3]);
    }

    #[test]
    fn with_no_rule_it_succeeds_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let tool = FakeTool::new(dir.path(), "gogdl");
        let out = run(&tool, &["--version"]);
        assert_eq!((out.status.code(), out.stdout.len()), (Some(0), 0));
        // A copy without its script (a download being checked) too.
        let copy = dir.path().join(format!("gogdl.part{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(fake_tool_program(), &copy).unwrap();
        assert!(Command::new(&copy).arg("--version").status().unwrap().success());
    }
}
