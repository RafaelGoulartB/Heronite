//! A spawner that records instead of running, for tests (also usable from the app's tests).

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::{ExitInfo, ProcessHandle, Spawner};
use crate::plan::Command;

/// Records everything instead of running it. Handles finish when told to.
#[derive(Default)]
pub struct FakeSpawner {
    state: Arc<Mutex<FakeState>>,
}

#[derive(Default)]
struct FakeState {
    spawned: Vec<Command>,
    uris: Vec<String>,
    patterns: Vec<String>,
    handles: Vec<Arc<Mutex<FakeHandleState>>>,
    fail_spawn: Option<String>,
    fail_uri: Option<String>,
    next_pid: u32,
}

#[derive(Default)]
pub struct FakeHandleState {
    pub exit: Option<ExitInfo>,
    pub terminated: bool,
    pub killed: bool,
}

struct FakeHandle {
    pid: u32,
    state: Arc<Mutex<FakeHandleState>>,
    /// Exits by itself when terminated, like a well-behaved process.
    exit_on_terminate: bool,
}

impl ProcessHandle for FakeHandle {
    fn pid(&self) -> u32 {
        self.pid
    }
    fn poll(&mut self) -> io::Result<Option<ExitInfo>> {
        Ok(self.state.lock().unwrap().exit)
    }
    fn terminate(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.terminated = true;
        if self.exit_on_terminate {
            s.exit = Some(ExitInfo { code: None, signal: Some(15) });
        }
    }
    fn kill(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.killed = true;
        s.exit = Some(ExitInfo { code: None, signal: Some(9) });
    }
}

impl FakeSpawner {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn spawned(&self) -> Vec<Command> {
        self.state.lock().unwrap().spawned.clone()
    }
    pub fn uris(&self) -> Vec<String> {
        self.state.lock().unwrap().uris.clone()
    }
    pub fn patterns(&self) -> Vec<String> {
        self.state.lock().unwrap().patterns.clone()
    }
    pub fn handle(&self, index: usize) -> Option<Arc<Mutex<FakeHandleState>>> {
        self.state.lock().unwrap().handles.get(index).cloned()
    }
    /// Makes the `index`-th spawned process exit.
    pub fn finish(&self, index: usize, code: i32) {
        if let Some(h) = self.handle(index) {
            h.lock().unwrap().exit = Some(ExitInfo { code: Some(code), signal: None });
        }
    }
    pub fn fail_next_spawn(&self, message: &str) {
        self.state.lock().unwrap().fail_spawn = Some(message.to_string());
    }
    pub fn fail_next_uri(&self, message: &str) {
        self.state.lock().unwrap().fail_uri = Some(message.to_string());
    }
}

impl Spawner for FakeSpawner {
    fn spawn(&self, cmd: &Command, _log: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        let mut s = self.state.lock().unwrap();
        if let Some(msg) = s.fail_spawn.take() {
            return Err(io::Error::new(io::ErrorKind::NotFound, msg));
        }
        s.next_pid += 1;
        let pid = 40_000 + s.next_pid;
        s.spawned.push(cmd.clone());
        let st = Arc::new(Mutex::new(FakeHandleState::default()));
        s.handles.push(st.clone());
        Ok(Box::new(FakeHandle { pid, state: st, exit_on_terminate: true }))
    }
    fn open_uri(&self, uri: &str) -> io::Result<()> {
        let mut s = self.state.lock().unwrap();
        if let Some(msg) = s.fail_uri.take() {
            return Err(io::Error::new(io::ErrorKind::NotFound, msg));
        }
        s.uris.push(uri.to_string());
        Ok(())
    }
    fn kill_pattern(&self, pattern: &str) {
        self.state.lock().unwrap().patterns.push(pattern.to_string());
    }
    fn make_executable(&self, _path: &str) -> io::Result<()> {
        Ok(())
    }
    fn create_dir_all(&self, _path: &str) -> io::Result<()> {
        Ok(())
    }
}
