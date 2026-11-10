//! What one call of a script may use. Going over any of them ends the call
//! (the source answers "not found" for that game, or "unreachable" when the
//! network failed) and counts as a failure of the extension.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Wall-clock time of one call of an exported function, the requests
    /// and the waits for the rate included.
    pub time: Duration,
    /// Memory of the script's engine (all its objects), for the whole lookup.
    pub memory: usize,
    /// Stack of the script's engine (deep recursion ends the call).
    pub stack: usize,
    /// Requests one call may send (each redirect counts).
    pub requests: u32,
    /// Time one request may take (never more than what is left of `time`).
    pub request_timeout: Duration,
    /// Size of one answer's body.
    pub response_bytes: u64,
    /// Size of all the answers of one call.
    pub total_bytes: u64,
    /// Lines `ctx.log` writes per call; more are dropped.
    pub log_lines: u32,
    /// Failures in a row (errors and limits, not an unreachable network)
    /// after which the extension is not asked until switched on again or
    /// updated.
    pub failures_in_a_row: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            time: Duration::from_secs(10),
            memory: 32 * 1024 * 1024,
            stack: 1024 * 1024,
            requests: 12,
            request_timeout: Duration::from_secs(8),
            response_bytes: 2 * 1024 * 1024,
            total_bytes: 6 * 1024 * 1024,
            log_lines: 20,
            failures_in_a_row: 5,
        }
    }
}
