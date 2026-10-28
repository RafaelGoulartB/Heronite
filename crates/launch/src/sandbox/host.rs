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

/// The variables of the session that name a socket the sandbox binds: the
/// ones the sandbox's program got, which can differ from the launcher's (a
/// game under gamescope has gamescope's display). The proxies (see
/// [`super::pass::PROXY`]) are taken from the sandbox too, as it got them
/// only with the network exception.
const SESSION_VARS: [&str; 3] = ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY"];

/// [`SESSION_VARS`] and the proxies as the program of the sandbox of `pid` has them: the
/// first process below `pid` (itself included) whose `environ` (read by
/// `environ`) carries the sandbox's mark, which is the `sh` that waits for
/// the game. bubblewrap's own processes start with none (see `script`).
/// `Some(empty)`: the sandbox has none of them. `None`: not known.
fn session_of(procs: &dyn Procs, pid: u32, environ: &dyn Fn(u32) -> Option<Vec<u8>>) -> Option<Vec<(String, String)>> {
    let mark = format!("{}=1", super::ENV_INSIDE);
    let mut seen = std::collections::HashSet::from([pid]);
    let mut level = vec![pid];
    for _ in 0..16 {
        level.sort_unstable();
        for p in &level {
            let Some(data) = environ(*p) else { continue };
            let vars: Vec<&[u8]> = data.split(|b| *b == 0).collect();
            if !vars.contains(&mark.as_bytes()) {
                continue;
            }
            return Some(
                SESSION_VARS
                    .iter()
                    .chain(&super::pass::PROXY)
                    .filter_map(|name| {
                        let prefix = format!("{name}=");
                        let value = vars.iter().find_map(|v| v.strip_prefix(prefix.as_bytes()))?;
                        let value = String::from_utf8(value.to_vec()).ok().filter(|v| !v.is_empty())?;
                        Some((name.to_string(), value))
                    })
                    .collect(),
            );
        }
        level = level.iter().flat_map(|p| procs.children(*p)).filter(|c| seen.insert(*c)).collect();
        if level.is_empty() {
            break;
        }
    }
    None
}

/// `cmd` run inside the running sandbox whose first process is `pid`, with
/// the same view, network, IPC and processes (`nsenter`, from util-linux).
/// Its environment is the one of the sandbox's program (see [`super::pass`]),
/// with the display the sandbox's game got.
pub fn join(cmd: &Command, pid: u32) -> Command {
    let session = session_of(&SystemProcs, pid, &|p| std::fs::read(format!("/proc/{p}/environ")).ok());
    join_with(cmd, pid, session.as_deref())
}

/// [`join`] with the sandbox's [`session_of`]. `None`: the launcher's own
/// display variables pass, as the rest of the allowlist does, and no proxy.
fn join_with(cmd: &Command, pid: u32, session: Option<&[(String, String)]>) -> Command {
    let mut args: Vec<String> = vec!["--target".into(), pid.to_string()];
    args.extend(["--user", "--mount", "--pid", "--net", "--uts", "--ipc", "--preserve-credentials"].map(String::from));
    // `--wd` would name a folder outside (`--wdns` is too recent): `cd` inside.
    let cwd = cmd.cwd.as_deref().filter(|d| !d.is_empty()).unwrap_or("/");
    let plan = cmd.env.keys().map(String::as_str);
    // The command's own variables win over the session's.
    let own = |name: &str| cmd.env.contains_key(name);
    let skip: Vec<&str> = if session.is_some() { SESSION_VARS.into_iter().filter(|n| !own(n)).collect() } else { Vec::new() };
    let words: String = session
        .into_iter()
        .flatten()
        .filter(|(k, _)| !own(k))
        .map(|(k, v)| format!(" {}", super::script::q(&format!("{k}={v}"))))
        .collect();
    // The sandbox's IPC namespace has no shared memory with the X server
    // (MIT-SHM), as in `script`.
    let script = format!(
        "cd -- \"$0\" || exit 126; {}set -- \"$@\" {}=1 _X11_NO_MITSHM=1 QT_X11_NO_MITSHM=1{words}; {}exec env -i \"$@\"",
        super::pass::collect(plan, &skip, false, r#""$y=$z""#),
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
        assert_eq!(j.args[..9], ["--target", "4242", "--user", "--mount", "--pid", "--net", "--uts", "--ipc", "--preserve-credentials"]);
        assert_eq!(j.args[9..12], ["--", "/bin/sh", "-c"]);
        let script = &j.args[12];
        assert!(script.starts_with(r#"cd -- "$0""#) && script.ends_with(r#"exec env -i "$@""#), "{script}");
        assert!(script.contains("p=' PROTON_VERB '"), "the command's own variables always pass");
        assert!(script.contains("HERONITE_SANDBOX=1"));
        // The sandbox has its own IPC namespace, so shared memory goes through the socket.
        assert!(script.contains("_X11_NO_MITSHM=1") && script.contains("QT_X11_NO_MITSHM=1"), "{script}");
        assert_eq!(j.args[13..], ["/t", "/p/GE/proton", "runinprefix", "/t/trainer.exe"]);
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
        let out = Process::new(&j.args[10])
            .args(&j.args[11..])
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

    /// What `join_with` starts, run without `nsenter` (as the launcher's
    /// environment `env`): the program's environment.
    fn joined_env(session: Option<&[(String, String)]>, env: &[(&str, &str)]) -> Vec<String> {
        let cmd = Command { program: "/usr/bin/env".into(), cwd: Some("/".into()), ..Default::default() };
        let j = join_with(&cmd, 1, session);
        let at = j.args.iter().position(|a| a == "--").unwrap() + 1;
        let out = Process::new(&j.args[at])
            .args(&j.args[at + 1..])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// The sandbox binds the socket of the display its game started on
    /// (gamescope's), not the launcher's.
    #[test]
    fn a_joined_program_gets_the_display_of_the_sandbox_not_the_launchers() {
        let launcher = [("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0"), ("XAUTHORITY", "/run/user/1000/xauth_a")];
        let session = pairs(&[("DISPLAY", ":1"), ("WAYLAND_DISPLAY", "gamescope-0")]);
        let vars = joined_env(Some(&session), &launcher);
        assert!(vars.contains(&"DISPLAY=:1".to_string()) && vars.contains(&"WAYLAND_DISPLAY=gamescope-0".to_string()), "{vars:?}");
        assert!(!vars.iter().any(|v| v.contains(":0") || v.contains("wayland-0")), "{vars:?}");
        // The sandbox had no display (headless): neither does the program.
        let vars = joined_env(Some(&[]), &launcher);
        assert!(!vars.iter().any(|v| v.starts_with("DISPLAY=") || v.starts_with("WAYLAND_DISPLAY=")), "{vars:?}");
        // Unknown (its environment cannot be read): the launcher's.
        let vars = joined_env(None, &launcher);
        assert!(vars.contains(&"DISPLAY=:0".to_string()) && vars.contains(&"WAYLAND_DISPLAY=wayland-0".to_string()), "{vars:?}");
        // A value with quotes stays a value.
        let tricky = pairs(&[("DISPLAY", "a b';touch /tmp/x;'")]);
        assert!(joined_env(Some(&tricky), &launcher).contains(&"DISPLAY=a b';touch /tmp/x;'".to_string()));
    }

    #[test]
    fn the_display_comes_from_the_first_process_of_the_sandbox_with_its_environment() {
        let procs = FakeProcs::new();
        procs.start(10, "bwrap", None);
        procs.start(11, "bwrap", Some(10));
        procs.start(12, "sh", Some(11));
        procs.start(13, "game", Some(12));
        let environ = |pid: u32| match pid {
            // bwrap itself starts with none; the game's own can differ.
            10 | 11 => Some(Vec::new()),
            12 => Some(b"PATH=/usr/bin\0HERONITE_SANDBOX=1\0DISPLAY=:1\0WAYLAND_DISPLAY=gamescope-0\0XAUTHORITY=/x\0".to_vec()),
            13 => Some(b"HERONITE_SANDBOX=1\0DISPLAY=:9\0".to_vec()),
            _ => None,
        };
        let session = session_of(&procs, 10, &environ).expect("found");
        assert_eq!(session, pairs(&[("DISPLAY", ":1"), ("WAYLAND_DISPLAY", "gamescope-0"), ("XAUTHORITY", "/x")]));
        // Headless: found, with no display.
        let headless = |pid: u32| (pid == 12).then(|| b"HERONITE_SANDBOX=1\0".to_vec());
        assert_eq!(session_of(&procs, 10, &headless), Some(Vec::new()));
        // Nothing readable or marked: unknown.
        assert_eq!(session_of(&procs, 10, &|_| None), None);
        assert_eq!(session_of(&procs, 10, &|_| Some(b"DISPLAY=:5\0".to_vec())), None);
    }

    /// The sandbox's game got the proxy of the launcher only with the network
    /// exception; a program joined to it gets what its game got, no more.
    #[test]
    fn a_joined_program_gets_the_proxy_its_game_got_and_no_other() {
        let procs = FakeProcs::new();
        procs.start(10, "bwrap", None);
        procs.start(11, "sh", Some(10));
        let with = |pid: u32| (pid == 11).then(|| b"HERONITE_SANDBOX=1\0DISPLAY=:1\0https_proxy=http://u:p@h\0".to_vec());
        let session = session_of(&procs, 10, &with).unwrap();
        assert_eq!(session, pairs(&[("DISPLAY", ":1"), ("https_proxy", "http://u:p@h")]));
        let launcher = [("https_proxy", "http://u:p@h"), ("http_proxy", "http://other")];
        let vars = joined_env(Some(&session), &launcher);
        assert!(vars.contains(&"https_proxy=http://u:p@h".to_string()), "{vars:?}");
        // The sandbox has no network (its game got no proxy): neither does the program.
        let none = session_of(&procs, 10, &|pid| (pid == 11).then(|| b"HERONITE_SANDBOX=1\0".to_vec())).unwrap();
        let vars = joined_env(Some(&none), &launcher);
        assert!(!vars.iter().any(|v| v.contains("proxy")), "{vars:?}");
        let vars = joined_env(None, &launcher);
        assert!(!vars.iter().any(|v| v.contains("proxy")), "unknown sandbox: {vars:?}");
    }

    /// With the real bubblewrap and `nsenter`, when they work here: a program
    /// joined to a running sandbox is in its IPC namespace and has the
    /// display its game got.
    #[test]
    fn a_real_joined_program_is_in_the_ipc_namespace_and_has_the_games_display() {
        let Some(b) = detect().filter(|b| b.supported() && b.error.is_none()) else {
            eprintln!("no working bubblewrap here: skipped");
            return;
        };
        let p =
            super::super::script::prelude(&b, &crate::sandbox::Sandbox { enabled: true, ..Default::default() }, &Default::default(), None);
        let mut child = Process::new(&p[0])
            .args(&p[1..])
            .args(["sleep", "30"])
            .env("DISPLAY", ":7")
            .env("WAYLAND_DISPLAY", "gamescope-0")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let procs = SystemProcs;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (mut sandbox, mut session) = (None, None);
        while session.is_none() && std::time::Instant::now() < deadline {
            sandbox = find_sandbox(&procs, child.id());
            session = sandbox.and_then(|s| session_of(&procs, s, &|p| std::fs::read(format!("/proc/{p}/environ")).ok()));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let result = sandbox.map(|pid| {
            let cmd = Command {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "readlink /proc/self/ns/ipc; env".into()],
                cwd: Some("/".into()),
                ..Default::default()
            };
            let j = join(&cmd, pid);
            let out = Process::new(&j.program).args(&j.args).env("DISPLAY", ":0").env("WAYLAND_DISPLAY", "wayland-0").output();
            (out, std::fs::read_link(format!("/proc/{pid}/ns/ipc")).ok(), std::fs::read_link("/proc/self/ns/ipc").ok())
        });
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(session, Some(pairs(&[("DISPLAY", ":7"), ("WAYLAND_DISPLAY", "gamescope-0")])));
        let (out, theirs, ours) = result.unwrap();
        let out = out.unwrap();
        if !out.status.success() {
            eprintln!("nsenter cannot join here ({}): skipped", String::from_utf8_lossy(&out.stderr).trim());
            return;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        assert_ne!(theirs, ours);
        assert_eq!(lines[0], theirs.unwrap().to_string_lossy());
        for want in ["DISPLAY=:7", "WAYLAND_DISPLAY=gamescope-0", "_X11_NO_MITSHM=1", "QT_X11_NO_MITSHM=1"] {
            assert!(lines.contains(&want), "{want} missing: {lines:?}");
        }
        assert!(!text.contains("DISPLAY=:0") && !text.contains("wayland-0"), "{lines:?}");
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
