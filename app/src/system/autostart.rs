//! "Start with the system": an XDG autostart entry
//! (`~/.config/autostart/com.heronite.rs.desktop`) that follows the
//! `startWithSystem` global setting. The entry passes `--autostart`: only
//! that start honours "Start minimized" (`startInTray`).
//!
//! The toggle writes or removes the entry; while it is on, [`sync`] also runs
//! at every start and rewrites it (the executable may have moved, e.g. a new
//! AppImage). `HERONITE_AUTOSTART_DIR` redirects the folder.
//!
//! Blocking (file writes): call it off the UI thread.

use std::path::{Path, PathBuf};

use heronite_shortcuts::desktop::quote_exec_arg;

const FILE_NAME: &str = "com.heronite.rs.desktop";

fn autostart_dir() -> Option<PathBuf> {
    if let Some(dir) = crate::env::value("HERONITE_AUTOSTART_DIR") {
        return Some(PathBuf::from(dir));
    }
    dirs::config_dir().map(|d| d.join("autostart"))
}

fn entry_text(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Heronite\nComment=Every game you own, on Linux, in one native launcher\nExec={} --autostart\nIcon=com.heronite.rs\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        quote_exec_arg(&exe.to_string_lossy())
    )
}

/// Writes (`on`) or removes the entry in `dir`. Rewrites only when it changed.
fn apply_in(dir: &Path, exe: &Path, on: bool) -> std::io::Result<()> {
    let file = dir.join(FILE_NAME);
    if !on {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        };
    }
    let text = entry_text(exe);
    if std::fs::read_to_string(&file).is_ok_and(|t| t == text) {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(&file, text)
}

/// Makes the autostart entry match `on`.
pub fn sync(on: bool) {
    let Some(dir) = autostart_dir() else { return };
    if let Err(e) = apply_in(&dir, &super::shortcuts::own_exe(), on) {
        log::warn!("autostart entry in {}: {e}", dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_removes_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("autostart");
        let exe = Path::new("/opt/My Apps/heronite-rs");
        apply_in(&dir, exe, true).unwrap();
        let text = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(text.contains("Exec=\"/opt/My Apps/heronite-rs\" --autostart\n"), "{text}");
        apply_in(&dir, exe, false).unwrap();
        assert!(!dir.join(FILE_NAME).exists());
        // Already gone: still fine.
        apply_in(&dir, exe, false).unwrap();
    }
}
