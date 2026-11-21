//! What a store's command-line tool left when it finished (nile for Amazon
//! Games; Epic and GOG need none), and the [`Cancel`] every long
//! operation of the stores takes.

/// Result of a finished run.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    /// The run was stopped through its [`Cancel`].
    pub aborted: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.aborted
    }

    /// `ERROR:` / `CRITICAL:` lines of the tool's log, for messages.
    pub fn error_lines(&self) -> String {
        let all = format!("{}\n{}", self.stderr, self.stdout);
        let lines: Vec<&str> = all.lines().filter(|l| l.contains("ERROR") || l.contains("CRITICAL") || l.contains("Traceback")).collect();
        lines.join("\n")
    }
}

pub use heronite_data::proc::Cancel;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_picked_from_both_streams() {
        let out = Output { stdout: "ok\n[cli] ERROR: no".into(), stderr: "CRITICAL: worse\nfine".into(), code: Some(1), aborted: false };
        assert!(!out.success());
        assert_eq!(out.error_lines(), "CRITICAL: worse\n[cli] ERROR: no");
        assert!(Output { code: Some(0), ..Default::default() }.success());
        assert!(!Output { code: Some(0), aborted: true, ..Default::default() }.success());
    }
}
