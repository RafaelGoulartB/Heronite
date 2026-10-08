//! A scripted [`CommandRunner`] for tests.

use std::sync::Mutex;

use super::runner::{CommandOutput, CommandRunner};

#[derive(Default)]
pub(super) struct Fake {
    pub(super) calls: Mutex<Vec<Vec<String>>>,
    /// (prefix of args joined by space) -> output
    pub(super) replies: Vec<(String, CommandOutput)>,
}

impl CommandRunner for Fake {
    fn run(&self, _binary: &str, args: &[String]) -> Result<CommandOutput, String> {
        self.calls.lock().unwrap().push(args.to_vec());
        let joined = args.join(" ");
        for (prefix, out) in &self.replies {
            if joined.starts_with(prefix.as_str()) {
                return Ok(out.clone());
            }
        }
        Ok(CommandOutput::default())
    }
}

pub(super) fn out(stdout: &str, stderr: &str, code: i32) -> CommandOutput {
    CommandOutput { stdout: stdout.into(), stderr: stderr.into(), code }
}
