//! Display capture with `grim` (wlroots compositors, Hyprland included).

use std::process::{Command, Stdio};

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// PNG bytes of one output (`grim -o <name> -`) or of everything (`grim -`).
pub fn capture_png(output: Option<&str>) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("grim");
    if let Some(name) = output {
        cmd.args(["-o", name]);
    }
    cmd.arg("-").stdin(Stdio::null()).stderr(Stdio::piped());
    let out = cmd.output().map_err(|e| format!("grim: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("grim failed ({}): {}", out.status, err.trim()));
    }
    if out.stdout.is_empty() {
        return Err("grim returned an empty image".into());
    }
    if !out.stdout.starts_with(&PNG_MAGIC) {
        return Err("grim did not return a PNG".into());
    }
    Ok(out.stdout)
}
