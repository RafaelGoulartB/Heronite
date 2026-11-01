//! "Start with the system": an XDG autostart entry
//! (`~/.config/autostart/com.heronite.rs.desktop`) on Linux, a value of the
//! user's `Run` key (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`)
//! on Windows, that follows the `startWithSystem` global setting. The entry
//! passes `--autostart`: only that start honours "Start minimized"
//! (`startInTray`).
//!
//! The toggle writes or removes the entry, and [`sync`] runs again at every
//! start: on, it rewrites it (the executable may have moved, e.g. a new
//! AppImage); off, it removes one left behind. The entry is one per user, so
//! only the real data folder drives it: a run against a copy
//! (`HERONITE_DATA_DIR`) must neither point it at its build nor remove it.
//! `HERONITE_AUTOSTART_DIR` redirects the folder, and
//! `HERONITE_AUTOSTART_KEY` the registry key under `HKEY_CURRENT_USER` (both
//! lift that limit).
//!
//! Blocking (file or registry writes): call it off the UI thread.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;
use heronite_shortcuts::desktop::quote_exec_arg;

const FILE_NAME: &str = "com.heronite.rs.desktop";

/// The `Run` key of the user and the value that starts Heronite.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Heronite";

/// Where the entry lives.
#[derive(Debug, PartialEq)]
enum Place {
    /// An XDG autostart folder.
    Folder(PathBuf),
    /// A key under `HKEY_CURRENT_USER` (Windows).
    RunKey(String),
}

fn place(data_paths: &DataPaths) -> Option<Place> {
    if let Some(dir) = crate::env::value("HERONITE_AUTOSTART_DIR") {
        return Some(Place::Folder(PathBuf::from(dir)));
    }
    if let Some(key) = crate::env::value("HERONITE_AUTOSTART_KEY") {
        return Some(Place::RunKey(key));
    }
    if !data_paths.is_default() || !heronite_platform::supported(heronite_platform::Feature::Autostart) {
        return None;
    }
    if cfg!(windows) {
        Some(Place::RunKey(RUN_KEY.to_string()))
    } else if heronite_platform::flatpak::active() {
        // The session reads the host's folder, not the sandbox's.
        heronite_platform::flatpak::host_xdg("XDG_CONFIG_HOME", ".config").map(|d| Place::Folder(d.join("autostart")))
    } else {
        dirs::config_dir().map(|d| Place::Folder(d.join("autostart")))
    }
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

/// The `Run` value: the quoted executable and `--autostart`.
#[cfg(windows)]
fn run_command(exe: &Path) -> String {
    format!("\"{}\" --autostart", exe.display())
}

/// Writes (`on`) or removes the `Run` value under `key`. Rewrites only when
/// it changed.
#[cfg(windows)]
fn apply_in_key(key: &str, exe: &Path, on: bool) -> std::io::Result<()> {
    use heronite_platform::registry::{delete_value, set_string, string, Hive};
    if !on {
        return delete_value(Hive::CurrentUser, key, RUN_VALUE);
    }
    let command = run_command(exe);
    if string(Hive::CurrentUser, key, RUN_VALUE).as_deref() == Some(command.as_str()) {
        return Ok(());
    }
    set_string(Hive::CurrentUser, key, RUN_VALUE, &command)
}

/// Makes the autostart entry match `on` (nothing for a copy of the data folder).
pub fn sync(data_paths: &DataPaths, on: bool) {
    let exe = super::shortcuts::own_exe();
    match place(data_paths) {
        None => {}
        Some(Place::Folder(dir)) => {
            if let Err(e) = apply_in(&dir, &exe, on) {
                log::warn!("autostart entry in {}: {e}", dir.display());
            }
        }
        #[cfg(windows)]
        Some(Place::RunKey(key)) => {
            if let Err(e) = apply_in_key(&key, &exe, on) {
                log::warn!(r"autostart value in HKEY_CURRENT_USER\{key}: {e}");
            }
        }
        #[cfg(not(windows))]
        Some(Place::RunKey(_)) => {}
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

    #[test]
    fn a_copy_of_the_data_folder_leaves_the_entry_alone() {
        if crate::env::value("HERONITE_AUTOSTART_DIR").is_some() || crate::env::value("HERONITE_AUTOSTART_KEY").is_some() {
            return;
        }
        let data = tempfile::tempdir().unwrap();
        assert_eq!(place(&DataPaths::at(data.path())), None);
    }

    #[cfg(windows)]
    #[test]
    fn writes_and_removes_the_run_value() {
        use heronite_platform::registry::{string, Hive};
        let key = format!(r"Software\Heronite\Tests\autostart-{}", std::process::id());
        let exe = Path::new(r"C:\Program Files\Heronite\heronite-rs.exe");
        apply_in_key(&key, exe, true).unwrap();
        assert_eq!(
            string(Hive::CurrentUser, &key, RUN_VALUE).as_deref(),
            Some(r#""C:\Program Files\Heronite\heronite-rs.exe" --autostart"#)
        );
        apply_in_key(&key, exe, false).unwrap();
        assert_eq!(string(Hive::CurrentUser, &key, RUN_VALUE), None);
        // Already gone: still fine.
        apply_in_key(&key, exe, false).unwrap();
        let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\{key}"), "/f"]).output();
    }
}
