//! Processes: whether one lives, stopping it, process groups and detached
//! starts.
//!
//! A *group* is a process and everything it starts: a Unix process group
//! (its id is the leader's pid) or, on Windows, a job object the child is put
//! in once it runs ([`track_group`]). Either way the group is named by the
//! first process's pid, so callers keep passing pids around.
//!
//! Windows has no SIGTERM: [`Signal::Term`] ends the process there as
//! [`Signal::Kill`] does.

use std::process::{Child, Command, ExitStatus};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

#[cfg(windows)]
pub use windows::{command_line, exe_path, in_tracked_group, snapshot, start_time, ProcessEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Asks the process to stop (SIGTERM).
    Term,
    /// Ends it at once (SIGKILL).
    Kill,
}

/// Whether `pid` names a live process (one we may not signal counts).
pub fn is_alive(pid: u32) -> bool {
    pid > 0 && sys::is_alive(pid)
}

/// Sends `sig` to the process `pid`. Never to pid 0 or 1.
pub fn signal(pid: u32, sig: Signal) {
    if pid > 1 {
        sys::signal(pid, sig);
    }
}

/// The child `cmd` starts leads a group of its own (see the module docs).
/// On Windows the group starts with [`track_group`], called on the child.
///
/// [`new_group`], [`new_helper_group`] and [`detach`] each set all of a
/// command's creation flags on Windows: give a command only one of them.
pub fn new_group(cmd: &mut Command) -> &mut Command {
    sys::new_group(cmd)
}

/// [`new_group`] for a helper program (a command-line tool the app runs and
/// reads): on Windows it also gets no console window, which Windows would
/// open for every such tool a windowed app starts.
pub fn new_helper_group(cmd: &mut Command) -> &mut Command {
    sys::new_helper_group(cmd)
}

/// Starts following the group of `child` (a job object on Windows). Call it
/// right after the child started from a [`new_group`] command; a no-op on
/// Unix, where the kernel keeps the group.
pub fn track_group(child: &Child) {
    sys::track_group(child);
}

/// Whether any process of the group `leader` started is left.
pub fn group_alive(leader: u32) -> bool {
    leader > 1 && sys::group_alive(leader)
}

/// Sends `sig` to every process of the group of `leader`.
pub fn signal_group(leader: u32, sig: Signal) {
    if leader > 1 {
        sys::signal_group(leader, sig);
    }
}

/// The child `cmd` starts is on its own: a new session on Unix (`setsid`),
/// no console and no job of ours on Windows. For programs that must outlive
/// the app (the updated app, a store client).
pub fn detach(cmd: &mut Command) -> &mut Command {
    sys::detach(cmd)
}

/// The signal that ended a process (`None` when it exited, and always on
/// Windows).
pub fn exit_signal(status: &ExitStatus) -> Option<i32> {
    sys::exit_signal(status)
}

/// Milliseconds of a clock that only moves forward and that the next run of
/// the app reads the same during this boot (`CLOCK_MONOTONIC` on Linux, the
/// tick count on Windows). Unlike an `Instant`, it can be written down.
pub fn monotonic_ms() -> u64 {
    sys::monotonic_ms()
}
