//! Cuts the end off a saved replay: a save from the panel holds the time the
//! panel was open, and the panel is in the picture. ffmpeg copies the
//! streams (nothing is encoded again) on a thread with idle priority, so the
//! game never waits for it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;

/// Whether ffmpeg and ffprobe are there to cut with.
pub fn available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|n| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(n).is_file())))
}

/// Cuts the last `seconds` off `path` in the background (the file keeps its
/// name and date). Failures are only logged: the replay stays as saved.
pub fn cut_end_later(path: PathBuf, seconds: f64) {
    let spawned = std::thread::Builder::new().name("replay-trim".into()).spawn(move || {
        heronite_data::proc::idle_priority();
        match cut_end(&path, seconds) {
            Ok(()) => log::info!("replay: cut {seconds:.1} s off the end of {}", path.display()),
            Err(e) => log::warn!("replay: could not cut the end of {}: {e}", path.display()),
        }
    });
    if let Err(e) = spawned {
        log::warn!("replay: could not start cutting {e}");
    }
}

fn cut_end(path: &Path, seconds: f64) -> Result<(), String> {
    let duration = duration(path)?;
    let keep = duration - seconds;
    if keep < 1. {
        return Err(format!("only {duration:.1} s long"));
    }
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("bad file name")?;
    let tmp = path.with_file_name(format!(".{name}.cut.mp4"));
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-i"]).arg(path);
    cmd.args(["-t", &format!("{keep:.3}"), "-map", "0", "-c", "copy", "-movflags", "+faststart"]).arg(&tmp);
    let out = Run::new().timeout(Duration::from_secs(120)).run(&mut cmd).map_err(|e| e.to_string())?;
    if !out.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(out.stderr_text().trim().to_string());
    }
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    if let Some(t) = modified {
        let _ = std::fs::File::options().write(true).open(path).and_then(|f| f.set_modified(t));
    }
    Ok(())
}

/// Length in seconds (ffprobe).
fn duration(path: &Path) -> Result<f64, String> {
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"]).arg(path);
    let out = Run::new().timeout(Duration::from_secs(20)).run(&mut cmd).map_err(|e| e.to_string())?;
    out.stdout_text().trim().parse::<f64>().map_err(|_| format!("ffprobe: {}", out.stderr_text().trim()))
}
