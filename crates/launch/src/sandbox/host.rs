//! The machine's side of the sandbox: finding bubblewrap and whether it can
//! make sandboxes here, finding the sandbox of a running game and joining it.

use std::collections::HashMap;
use std::process::{Command as Process, Stdio};
use std::sync::Mutex;

use crate::plan::Command;
use crate::procs::{Procs, SystemProcs};

/// First bubblewrap with `--overlay-src` / `--tmp-overlay` (Winetricks'
/// downloads, see `mounts`).
pub const OVERLAY_VERSION: (u32, u32, u32) = (0, 10, 0);
/// First bubblewrap with `--perms`.
const MIN_VERSION: (u32, u32, u32) = (0, 5, 0);

/// bubblewrap on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bwrap {
    pub path: String,
    pub version: (u32, u32, u32),
    /// Why a test sandbox failed (user namespaces turned off...), if it did.
    pub error: Option<String>,
    /// `--tmp-overlay` works (see [`OVERLAY_VERSION`]; the kernel must also
    /// allow overlays in user namespaces, from 5.11).
    pub overlay: bool,
}

impl Bwrap {
    pub fn supported(&self) -> bool {
        self.version >= MIN_VERSION
    }

    /// Whether throwaway layers work (see [`Bwrap::overlay`]).
    pub fn overlay(&self) -> bool {
        self.overlay && self.version >= OVERLAY_VERSION
    }

    pub fn version_text(&self) -> String {
        let (a, b, c) = self.version;
        format!("{a}.{b}.{c}")
    }
}

/// bubblewrap, looked at until it works (it runs it up to three times: a
/// few milliseconds), then kept for this run of the app. `None`: not
/// installed. Not installed, too old or failing is looked at again on the
/// next call, so installing it or allowing user namespaces needs no restart.
pub fn detect() -> Option<Bwrap> {
    static CACHE: Mutex<Option<Bwrap>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.is_none() {
        let found = detect_now();
        if found.as_ref().is_some_and(|b| b.supported() && b.error.is_none()) {
            *cache = found;
        } else {
            return found;
        }
    }
    cache.clone()
}

fn detect_now() -> Option<Bwrap> {
    let path = crate::probe::Probe::which(&crate::probe::SystemProbe, "bwrap")?;
    let out = Process::new(&path).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let version = parse_version(&String::from_utf8_lossy(&out.stdout)).unwrap_or((0, 0, 0));
    let mut b = Bwrap { path, version, error: None, overlay: false };
    if b.supported() {
        b.error = trial(&b, false);
        b.overlay = b.error.is_none() && version >= OVERLAY_VERSION && trial(&b, true).is_none();
    }
    Some(b)
}

/// "bubblewrap 0.11.0" -> (0, 11, 0).
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let word = text.split_whitespace().find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = word.split('.').map(|p| p.trim_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().unwrap_or(0));
    Some((parts.next()?, parts.next().unwrap_or(0), parts.next().unwrap_or(0)))
}

/// Runs `true` in a sandbox shaped like a game's (user and pid namespaces;
/// `overlay`: a throwaway layer like Winetricks'). The error, if any. The
/// user namespace is required: a setuid bubblewrap without one would leave
/// the sandbox's processes looking like this app's, and the sandbox would
/// not be found (see [`find_sandbox`]).
fn trial(b: &Bwrap, overlay: bool) -> Option<String> {
    let mut args: Vec<&str> = vec!["--unshare-user", "--unshare-pid", "--unshare-net", "--ro-bind", "/", "/", "--proc", "/proc"];
    args.extend(["--dev", "/dev", "--tmpfs", "/tmp"]);
    if overlay {
        args.extend(["--overlay-src", "/usr", "--tmp-overlay", "/tmp/usr"]);
    }
    args.push("true");
    match Process::new(&b.path).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).output() {
        Ok(o) if o.status.success() => None,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            Some(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string())
        }
        Err(e) => Some(e.to_string()),
    }
}

/// The first process of the sandbox `root` runs in: the first one below it
/// (or itself) in another user namespace than this app. `root` is a game
/// started through wrappers (gamescope...) and `bwrap`. One pass over the
/// processes, then the user namespaces of `root`'s own.
pub fn find_sandbox(procs: &dyn Procs, root: u32) -> Option<u32> {
    let own = procs.user_ns(std::process::id())?;
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, parent) in procs.parents() {
        children.entry(parent).or_default().push(pid);
    }
    // Breadth first: the sandbox's first process before the ones it started.
    let mut level = vec![root];
    for _ in 0..64 {
        level.sort_unstable();
        if let Some(p) = level.iter().find(|p| procs.user_ns(**p).is_some_and(|ns| ns != own)) {
            return Some(*p);
        }
        level = level.iter().flat_map(|p| children.get(p).into_iter().flatten().copied()).collect();
        if level.is_empty() {
            break;
        }
    }
    None
}

/// The first process of a sandbox running programs in the Wine prefix
/// `prefix`, if one runs. Only processes in another user namespace count, so
/// what it finds is always some sandbox, never this app's own namespaces.
/// Reads `/proc`; not run by tests.
pub fn running_in_prefix(prefix: &str) -> Option<u32> {
    let procs = SystemProcs;
    let own = procs.user_ns(std::process::id())?;
    let inside = |pid: u32| procs.user_ns(pid).is_some_and(|ns| ns != own);
    let mark = format!("{}=1", super::ENV_INSIDE);
    let parents = procs.parents();
    let mut pids: Vec<u32> = parents.keys().copied().collect();
    pids.sort_unstable();
    for pid in pids {
        if !inside(pid) {
            continue;
        }
        let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else { continue };
        let vars: Vec<&[u8]> = environ.split(|b| *b == 0).collect();
        let marked = vars.contains(&mark.as_bytes());
        let here = vars
            .iter()
            .filter_map(|v| v.strip_prefix(b"WINEPREFIX="))
            .any(|v| crate::procs::same_prefix(&String::from_utf8_lossy(v), prefix));
        if !(marked && here) {
            continue;
        }
        // Up to the sandbox's first process (a program may run a sandbox
        // of its own inside it).
        let mut top = pid;
        for _ in 0..64 {
            match parents.get(&top) {
                Some(&p) if inside(p) => top = p,
                _ => return Some(top),
            }
        }
    }
    None
}

/// Whether `path` exists inside the running sandbox of `pid`. Not run by tests.
pub fn visible_in(pid: u32, path: &str) -> bool {
    let target = pid.to_string();
    let args = ["--target", &target, "--user", "--mount", "--preserve-credentials", "--", "test", "-e", path];
    Process::new("nsenter").args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// `cmd` run inside the running sandbox whose first process is `pid`, with
/// the same view, network and processes (`nsenter`, from util-linux). Its
/// environment is the one of the sandbox's program (see [`super::pass`]).
pub fn join(cmd: &Command, pid: u32) -> Command {
    let mut args: Vec<String> = vec!["--target".into(), pid.to_string()];
    args.extend(["--user", "--mount", "--pid", "--net", "--uts", "--preserve-credentials"].map(String::from));
    // `--wd` would name a folder outside (`--wdns` is too recent): `cd` inside.
    let cwd = cmd.cwd.as_deref().filter(|d| !d.is_empty()).unwrap_or("/");
    let plan = cmd.env.keys().map(String::as_str);
    let script = format!(
        "cd -- \"$0\" || exit 126; {}set -- \"$@\" {}=1; {}exec env -i \"$@\"",
        super::pass::collect(plan, &[], r#""$y=$z""#),
        super::ENV_INSIDE,
        super::pass::ROTATE,
    );
    args.extend(["--", "/bin/sh", "-c", &script].map(String::from));
    args.push(cwd.to_string());
    args.push(cmd.program.clone());
    args.extend(cmd.args.iter().cloned());
    Command { program: "nsenter".into(), args, env: cmd.env.clone(), cwd: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::FakeProcs;

    #[test]
    fn versions_are_read() {
        assert_eq!(parse_version("bubblewrap 0.11.0\n"), Some((0, 11, 0)));
        assert_eq!(parse_version("bubblewrap 0.6.1"), Some((0, 6, 1)));
        assert_eq!(parse_version("nothing"), None);
        let b = |v| Bwrap { path: "/usr/bin/bwrap".into(), version: v, error: None, overlay: true };
        assert!(!b((0, 4, 1)).supported());
        assert!(b((0, 6, 1)).supported() && !b((0, 6, 1)).overlay());
        assert!(b((0, 10, 0)).overlay());
    }

    #[test]
    fn the_sandbox_is_the_first_process_in_another_user_namespace() {
        let procs = FakeProcs::new();
        procs.start(10, "gamescope", None);
        procs.start(11, "bwrap", Some(10));
        procs.start(12, "bwrap", Some(11));
        procs.start(13, "sh", Some(12));
        procs.set_user_ns(12, "user:[2]");
        procs.set_user_ns(13, "user:[2]");
        assert_eq!(find_sandbox(&procs, 10), Some(12));
        assert_eq!(find_sandbox(&procs, 11), Some(12));
        procs.start(20, "game", None);
        assert_eq!(find_sandbox(&procs, 20), None, "not sandboxed");
    }

    #[test]
    fn joining_runs_the_command_through_nsenter_with_a_clean_environment() {
        let mut cmd =
            Command { program: "/p/GE/proton".into(), args: vec!["runinprefix".into(), "/t/trainer.exe".into()], ..Default::default() };
        cmd.cwd = Some("/t".into());
        cmd.env.insert("PROTON_VERB".into(), "runinprefix".into());
        let j = join(&cmd, 4242);
        assert_eq!(j.program, "nsenter");
        assert_eq!(j.args[..8], ["--target", "4242", "--user", "--mount", "--pid", "--net", "--uts", "--preserve-credentials"]);
        assert_eq!(j.args[8..11], ["--", "/bin/sh", "-c"]);
        let script = &j.args[11];
        assert!(script.starts_with(r#"cd -- "$0""#) && script.ends_with(r#"exec env -i "$@""#), "{script}");
        assert!(script.contains("p=' PROTON_VERB '"), "the command's own variables always pass");
        assert!(script.contains("HERONITE_SANDBOX=1"));
        assert_eq!(j.args[12..], ["/t", "/p/GE/proton", "runinprefix", "/t/trainer.exe"]);
        assert_eq!(j.env["PROTON_VERB"], "runinprefix");
        let out = Process::new("/bin/sh").arg("-n").arg("-c").arg(script).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    /// The program the script starts gets the plan's variables and the
    /// session's, not the launcher's secrets.
    #[test]
    fn a_joined_program_gets_no_secret_of_the_launcher() {
        let mut cmd = Command { program: "/usr/bin/env".into(), cwd: Some("/".into()), ..Default::default() };
        cmd.env.insert("MY_OPTION".into(), "1".into());
        let j = join(&cmd, 1);
        // What nsenter runs, without nsenter.
        let out = Process::new(&j.args[9])
            .args(&j.args[10..])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("DISPLAY", ":0")
            .env("MY_OPTION", "1")
            .env("GH_TOKEN", "ghp_secret")
            .env("SSH_AUTH_SOCK", "/s")
            .output()
            .unwrap();
        let env = String::from_utf8_lossy(&out.stdout);
        let vars: Vec<&str> = env.lines().collect();
        assert!(vars.contains(&"DISPLAY=:0") && vars.contains(&"MY_OPTION=1") && vars.contains(&"HERONITE_SANDBOX=1"), "{vars:?}");
        assert!(!env.contains("GH_TOKEN") && !env.contains("SSH_AUTH_SOCK"), "{vars:?}");
    }

    /// With the real bubblewrap, when it works here: a sandboxed process is
    /// found below the one that started it, and by its prefix.
    #[test]
    fn a_real_sandbox_is_found() {
        let Some(b) = detect().filter(|b| b.supported() && b.error.is_none()) else {
            eprintln!("no working bubblewrap here: skipped");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().to_string_lossy().into_owned();
        let mut child = Process::new(&b.path)
            .args(["--unshare-user", "--unshare-pid", "--ro-bind", "/", "/", "--proc", "/proc", "--dev", "/dev"])
            .args(["--setenv", super::super::ENV_INSIDE, "1", "--setenv", "WINEPREFIX", &prefix, "sleep", "30"])
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let procs = SystemProcs;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut found = None;
        while found.is_none() && std::time::Instant::now() < deadline {
            found = find_sandbox(&procs, child.id()).zip(running_in_prefix(&prefix));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        let (first, by_prefix) = found.expect("the sandbox is found");
        assert_ne!(procs.user_ns(first), procs.user_ns(std::process::id()));
        assert_eq!(first, by_prefix, "both find its first process");
        assert_eq!(running_in_prefix("/nowhere"), None);
    }
}
