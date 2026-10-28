//! Panic policy: the release build unwinds (so a decoder panic can be caught),
//! but the app still closes on any panic that is not explicitly contained.

use std::any::Any;
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, PanicHookInfo};

thread_local! {
    /// How many [`contained`] scopes are open on this thread.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Closes one scope when dropped, so the depth is restored on unwind too.
struct Scope;

impl Scope {
    fn enter() -> Self {
        DEPTH.with(|d| d.set(d.get() + 1));
        Scope
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// Runs `f` and catches its panic. While `f` runs on this thread, a panic is
/// "contained": the hook from [`install_hook`] lets it unwind instead of
/// aborting. Use it only around code that is expected to fail on hostile
/// input (picture decoders), never around app logic.
pub fn contained<T>(f: impl FnOnce() -> T) -> std::thread::Result<T> {
    let _scope = Scope::enter();
    std::panic::catch_unwind(AssertUnwindSafe(f))
}

/// True while the current thread is inside [`contained`]. Threads spawned from
/// it start uncontained.
pub fn is_contained() -> bool {
    DEPTH.with(|d| d.get() > 0)
}

/// The text of a caught panic payload (`panic!("...")` and `panic!("{x}")`).
pub fn message(payload: &(dyn Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default()
}

/// Installs the app's panic hook: `log` first, then the previous hook (message
/// and backtrace), then abort unless the panic is [`contained`].
pub fn install_hook(log: impl Fn(&PanicHookInfo<'_>) + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(info);
        previous(info);
        // Without this, a worker thread would die alone and leave its shared
        // state half-updated (a queue stuck "Running", poisoned locks).
        if !is_contained() {
            std::process::abort();
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_false_outside_and_true_inside() {
        assert!(!is_contained());
        contained(|| assert!(is_contained())).unwrap();
        assert!(!is_contained());
    }

    #[test]
    fn nesting_keeps_the_outer_scope_contained() {
        contained(|| {
            contained(|| assert!(is_contained())).unwrap();
            assert!(is_contained(), "leaving the inner scope must not end the outer one");
        })
        .unwrap();
        assert!(!is_contained());
    }

    #[test]
    fn a_caught_panic_restores_the_state() {
        let r = contained(|| -> () { panic!("boom") });
        assert_eq!(message(&*r.unwrap_err()), "boom");
        assert!(!is_contained());

        // A panic in the inner scope leaves the outer one contained.
        contained(|| {
            assert!(contained(|| -> () { panic!("inner") }).is_err());
            assert!(is_contained());
        })
        .unwrap();
        assert!(!is_contained());
    }

    #[test]
    fn returns_the_value() {
        assert_eq!(contained(|| 7).unwrap(), 7);
    }

    #[test]
    fn message_reads_str_and_string_payloads() {
        let n = 3;
        assert_eq!(message(&*contained(|| -> () { panic!("a {n}") }).unwrap_err()), "a 3");
        assert_eq!(message(&*contained(|| -> () { panic!("plain") }).unwrap_err()), "plain");
        assert_eq!(message(&*contained(|| -> () { std::panic::panic_any(5_u8) }).unwrap_err()), "");
    }

    #[test]
    fn a_spawned_thread_starts_uncontained() {
        contained(|| {
            assert!(!std::thread::spawn(is_contained).join().unwrap());
        })
        .unwrap();
    }

    const CHILD_ENV: &str = "HERONITE_PANICS_TEST_CHILD";

    /// Runs only in the child process of `the_hook_*`: installs the hook and panics.
    #[test]
    fn child_scenario() {
        let Ok(scenario) = std::env::var(CHILD_ENV) else { return };
        // SAFETY: plain syscall in a throwaway child; no core dump of the deliberate abort.
        unsafe {
            let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            libc::setrlimit(libc::RLIMIT_CORE, &none);
        }
        install_hook(|_| println!("hook-logged"));
        match scenario.as_str() {
            "contained" => {
                assert!(contained(|| -> () { panic!("decoder") }).is_err());
                println!("child-survived");
            }
            _ => panic!("app code"),
        }
    }

    fn run_child(scenario: &str) -> (std::process::ExitStatus, String) {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panics::tests::child_scenario", "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, scenario)
            .output()
            .unwrap();
        (out.status, String::from_utf8_lossy(&out.stdout).into_owned())
    }

    #[test]
    fn the_hook_aborts_on_a_panic_outside_contained() {
        use std::os::unix::process::ExitStatusExt;
        let (status, stdout) = run_child("uncontained");
        assert_eq!(status.signal(), Some(libc::SIGABRT), "{status:?}");
        assert!(stdout.contains("hook-logged"), "{stdout}");
    }

    #[test]
    fn the_hook_lets_a_contained_panic_unwind() {
        let (status, stdout) = run_child("contained");
        assert!(status.success(), "{status:?}");
        assert!(stdout.contains("hook-logged") && stdout.contains("child-survived"), "{stdout}");
    }
}
