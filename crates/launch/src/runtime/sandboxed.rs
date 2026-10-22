//! A process that starts a sandbox (see [`crate::sandbox`]) lasts as long as
//! the sandbox does. A SIGTERM ends `bwrap` itself, but a program that ignores
//! it keeps the sandbox (its pid namespace) going without `bwrap`; killing the
//! sandbox's first process ends every program in it.

use std::io;
use std::sync::Arc;
use std::time::Instant;

use super::spawner::{ExitInfo, ProcessHandle};
use crate::procs::{Procs, Sig};

pub(super) struct Sandboxed {
    inner: Box<dyn ProcessHandle>,
    procs: Arc<dyn Procs>,
    /// The sandbox's first process, once seen (it is only found below the
    /// process started while that one still runs).
    first: Option<u32>,
    /// Looked for until then (see [`crate::sandbox::FOUND_WITHIN`]): each look
    /// reads every process.
    look_until: Instant,
    exit: Option<ExitInfo>,
}

impl Sandboxed {
    pub(super) fn new(inner: Box<dyn ProcessHandle>, procs: Arc<dyn Procs>) -> Self {
        Self { inner, procs, first: None, look_until: Instant::now() + crate::sandbox::FOUND_WITHIN, exit: None }
    }
}

impl ProcessHandle for Sandboxed {
    fn pid(&self) -> u32 {
        self.inner.pid()
    }

    fn poll(&mut self) -> io::Result<Option<ExitInfo>> {
        if self.exit.is_none() {
            if self.first.is_none() && Instant::now() < self.look_until {
                self.first = crate::sandbox::find_sandbox(&*self.procs, self.inner.pid());
            }
            self.exit = self.inner.poll()?;
        }
        match (self.exit, self.first) {
            (Some(_), Some(first)) if self.procs.is_alive(first) => Ok(None),
            (exit, _) => Ok(exit),
        }
    }

    fn terminate(&mut self) {
        self.inner.terminate();
    }

    fn kill(&mut self) {
        self.inner.kill();
        if let Some(first) = self.first {
            self.procs.signal(first, Sig::Kill);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::FakeProcs;
    use std::sync::Mutex;

    struct Handle(Arc<Mutex<Option<ExitInfo>>>);

    impl ProcessHandle for Handle {
        fn pid(&self) -> u32 {
            10
        }
        fn poll(&mut self) -> io::Result<Option<ExitInfo>> {
            Ok(*self.0.lock().unwrap())
        }
        fn terminate(&mut self) {}
        fn kill(&mut self) {}
    }

    #[test]
    fn the_session_lasts_until_the_sandbox_is_gone_and_a_kill_ends_it() {
        let procs = Arc::new(FakeProcs::new());
        procs.start(10, "bwrap", None);
        procs.start(12, "bwrap", Some(10));
        procs.set_user_ns(12, "user:[2]");
        let exit = Arc::new(Mutex::new(None));
        let mut h = Sandboxed::new(Box::new(Handle(exit.clone())), procs.clone());
        assert_eq!(h.poll().unwrap(), None);
        // bwrap ended (a SIGTERM), a program inside ignored it.
        *exit.lock().unwrap() = Some(ExitInfo { code: None, signal: Some(15) });
        procs.exit(10);
        assert_eq!(h.poll().unwrap(), None, "the sandbox still runs");
        h.kill();
        assert!(procs.signals().contains(&(12, Sig::Kill)));
        procs.exit(12);
        assert_eq!(h.poll().unwrap(), Some(ExitInfo { code: None, signal: Some(15) }));
    }

    #[test]
    fn the_sandbox_is_looked_for_only_for_a_while() {
        let procs = Arc::new(FakeProcs::new());
        procs.start(10, "bwrap", None);
        let exit = Arc::new(Mutex::new(None));
        let mut h = Sandboxed::new(Box::new(Handle(exit.clone())), procs.clone());
        // Not found in time (a sandbox that never showed): no more looking.
        h.look_until = Instant::now();
        procs.start(12, "bwrap", Some(10));
        procs.set_user_ns(12, "user:[2]");
        assert_eq!(h.poll().unwrap(), None);
        assert_eq!(h.first, None);
        *exit.lock().unwrap() = Some(ExitInfo { code: Some(0), signal: None });
        assert_eq!(h.poll().unwrap(), Some(ExitInfo { code: Some(0), signal: None }));
    }
}
