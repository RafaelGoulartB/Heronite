//! `process::ask_to_stop` against real console programs: this test's own
//! program, started again in the roles below. No libtest harness
//! (`harness = false`): `main` must call `init_stop_helper` first, as the
//! app's does, for Windows to send Ctrl+C.

fn main() {
    heronite_platform::process::init_stop_helper();
    #[cfg(windows)]
    windows::main();
}

#[cfg(windows)]
mod windows {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Child, ChildStdout, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use heronite_platform::process::{self, Signal};
    use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT};

    static GOT_CTRL_C: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn on_ctrl(kind: u32) -> i32 {
        if kind == CTRL_C_EVENT {
            GOT_CTRL_C.store(true, Ordering::SeqCst);
            1
        } else {
            0
        }
    }

    fn ready() {
        println!("ready");
        std::io::stdout().flush().unwrap();
    }

    pub fn main() {
        match std::env::args().nth(1).as_deref() {
            // A tool that saves its work on Ctrl+C (legendary, gogdl).
            Some("--wait-for-ctrl-c") => {
                // SAFETY: a handler that only sets a flag.
                unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
                ready();
                let end = Instant::now() + Duration::from_secs(30);
                while !GOT_CTRL_C.load(Ordering::SeqCst) && Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(10));
                }
                println!("stopping cleanly");
                std::process::exit(42);
            }
            // A tool that goes on after Ctrl+C.
            Some("--ignore-ctrl-c") => {
                // SAFETY: turns Ctrl+C off for this process only.
                unsafe { SetConsoleCtrlHandler(None, 1) };
                ready();
                std::thread::sleep(Duration::from_secs(60));
            }
            // A program sharing its console with what it stops (a debug
            // build in a terminal); in a hidden console of its own, so a
            // Ctrl+C sent by mistake stays in there.
            Some("--host") => std::process::exit(host()),
            _ => checks(),
        }
    }

    fn me() -> Command {
        Command::new(std::env::current_exe().unwrap())
    }

    /// Starts this program as a helper in `role` and waits for its "ready".
    fn helper(role: &str) -> (Child, BufReader<ChildStdout>) {
        let mut cmd = me();
        cmd.arg(role).stdin(Stdio::null()).stdout(Stdio::piped());
        process::new_helper_group(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        process::track_group(&child);
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ready");
        (child, out)
    }

    fn wait(child: &mut Child, within: Duration) -> Option<i32> {
        let end = Instant::now() + within;
        while Instant::now() < end {
            if let Some(status) = child.try_wait().unwrap() {
                return status.code();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    fn checks() {
        // Asked, a helper stops by itself and finishes its work.
        let (mut child, mut out) = helper("--wait-for-ctrl-c");
        let asked = Instant::now();
        process::ask_to_stop(child.id(), true);
        assert_eq!(wait(&mut child, Duration::from_secs(10)), Some(42), "the helper got Ctrl+C and ended by itself");
        let mut rest = String::new();
        out.read_to_string(&mut rest).unwrap();
        assert_eq!(rest.trim(), "stopping cleanly");
        println!("stop_helper: Ctrl+C reached the helper in {:?}", asked.elapsed());

        // One that goes on is still there; the kill after the grace ends it.
        let (mut child, _out) = helper("--ignore-ctrl-c");
        process::ask_to_stop(child.id(), true);
        assert_eq!(wait(&mut child, Duration::from_secs(2)), None, "Ctrl+C does not end a helper that ignores it");
        process::signal_group(child.id(), Signal::Kill);
        assert_eq!(wait(&mut child, Duration::from_secs(5)), Some(1), "ended by the kill");

        // Never a Ctrl+C on a console the app shares.
        let mut cmd = me();
        cmd.arg("--host").stdin(Stdio::null()).stdout(Stdio::null());
        process::new_helper_group(&mut cmd);
        let mut host = cmd.spawn().unwrap();
        assert_eq!(wait(&mut host, Duration::from_secs(20)), Some(0), "see --host");

        if let Some(nile) = std::env::var_os("HERONITE_STOP_HELPER_NILE") {
            real_python_tool(nile.as_ref());
        }
        println!("stop_helper: ok");
    }

    /// A real Python tool (PyInstaller's one-file build: a launcher process
    /// and Python under it): `nile auth --login` waits for Enter; Ctrl+C
    /// must reach Python as a `KeyboardInterrupt`. Opt-in:
    /// `HERONITE_STOP_HELPER_NILE=<nile.exe>`.
    fn real_python_tool(nile: &std::path::Path) {
        let config = std::env::temp_dir().join(format!("heronite-stop-helper-{}", std::process::id()));
        let mut cmd = Command::new(nile);
        cmd.args(["auth", "--login"]).env("NILE_CONFIG_PATH", &config).env("PYTHONUNBUFFERED", "1");
        // Kept open and never written to: input() waits.
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        process::new_helper_group(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        process::track_group(&child);
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert!(line.starts_with("Login URL"), "{line}");
        std::thread::sleep(Duration::from_millis(500));
        process::ask_to_stop(child.id(), true);
        // Python reads a pipe without being interrupted: it sees the Ctrl+C
        // when the read returns. (Missed, nile would open the login page.)
        std::thread::sleep(Duration::from_secs(1));
        let mut stdin = child.stdin.take().unwrap();
        let _ = stdin.write_all(b"\n");
        drop(stdin);
        let code = wait(&mut child, Duration::from_secs(10));
        // Python under the launcher may hold the pipes: the whole group goes.
        process::signal_group(child.id(), Signal::Kill);
        let mut err = String::new();
        child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
        let _ = std::fs::remove_dir_all(&config);
        // A kill would leave no traceback.
        assert!(err.contains("KeyboardInterrupt"), "nile ended with {code:?} without a KeyboardInterrupt: {err}");
        assert!(code.is_some(), "nile ended by itself");
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default();
        println!("stop_helper: nile got a KeyboardInterrupt ({last}) and ended with {code:?}");
    }

    /// Stops a program on its own console: it must be ended (exit code 1
    /// of the kill), not sent Ctrl+C, and this program must live on.
    fn host() -> i32 {
        let mut cmd = me();
        // No flags: it shares this program's console.
        cmd.arg("--wait-for-ctrl-c").stdin(Stdio::null()).stdout(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        process::track_group(&child);
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        process::ask_to_stop(child.id(), true);
        let code = wait(&mut child, Duration::from_secs(10));
        // Give a stray Ctrl+C time to reach this program too.
        std::thread::sleep(Duration::from_millis(300));
        if code == Some(1) {
            0
        } else {
            eprintln!("the shared-console program ended with {code:?}");
            1
        }
    }
}
