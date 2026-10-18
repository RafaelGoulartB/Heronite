//! Running the helper programs of an install (archivers, Wine, shell
//! commands): guarded (they die with the app), stopped on cancel, their
//! output passed on line by line.

use std::process::Command;

use heronite_data::proc::Run;
use heronite_i18n::tr;

use crate::engine::CANCELLED;

/// What a run reports to and asks of the install.
pub struct Io<'a> {
    pub cancel: &'a dyn Fn() -> bool,
    pub log: &'a dyn Fn(&str),
}

/// Runs `cmd` to the end; its exit code. `Err` when it could not start,
/// was cancelled or was killed by a signal.
pub fn run(cmd: &mut Command, io: &Io) -> Result<i32, String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut on_line = |line: &str| (io.log)(line);
    let out = Run::new()
        .poll(io.cancel)
        .on_line(&mut on_line)
        .line_capture()
        .run(cmd)
        .map_err(|e| tr!("Could not start {program}: {error}", program, error = e))?;
    if out.aborted {
        return Err(CANCELLED.into());
    }
    out.code.ok_or_else(|| tr!("{program} was stopped by a signal", program))
}

/// [`run`] that fails unless the program exits with 0 (or `accepted`).
pub fn run_ok(cmd: &mut Command, io: &Io, accepted: Option<i32>) -> Result<(), String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let code = run(cmd, io)?;
    if code == 0 || Some(code) == accepted {
        Ok(())
    } else {
        Err(tr!("{program} exited with code {code}", program = crate::steps::short(&program), code))
    }
}

/// The first of `names` found on `$PATH`.
pub fn which_any(names: &[&str]) -> Option<String> {
    use heronite_launch::Probe;
    names.iter().find_map(|n| heronite_launch::SystemProbe.which(n))
}
