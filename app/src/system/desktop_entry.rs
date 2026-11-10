//! Heronite's menu entry (`app.heronite.Heronite.desktop`) where no package
//! installed one: an AppImage, the portable folder, a build run from its
//! target folder. The desktop portal names an app outside a sandbox by this
//! file (`Registry.Register` answers "App info not found" without it) and
//! refuses global shortcuts to an app it cannot name, so the screenshot,
//! overlay and replay shortcuts need it.
//!
//! [`sync`] runs at every start. Where a package or `scripts/install.sh`
//! installed the entry, it writes nothing. Elsewhere it keeps a hidden one
//! (`NoDisplay`: no menu changes) in the user's applications folder, pointing
//! at this executable, and removes it once a real one exists: the user's
//! folder wins over the system's, so a hidden entry left there would hide the
//! package's.
//!
//! Like the autostart entry, only the real data folder drives it (a run
//! against a copy must not point it at its build); `HERONITE_APPLICATIONS_DIR`
//! redirects the folder and lifts that limit. A Flatpak exports its own.
//!
//! Blocking (file writes): call it off the UI thread.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;
use heronite_shortcuts::desktop::quote_exec_arg;

/// The app id: the entry's name, what the window and the portal are told.
const APP_ID: &str = "app.heronite.Heronite";

/// Marks an entry [`sync`] wrote: only those are rewritten or removed.
const GENERATED: &str = "X-Heronite-Generated=true";

fn file_name() -> String {
    format!("{APP_ID}.desktop")
}

fn entry_text(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Heronite\nComment=Every game you own, on Linux, in one native launcher\nExec={} %U\nIcon={APP_ID}\nTerminal=false\nNoDisplay=true\nStartupWMClass={APP_ID}\n{GENERATED}\n",
        quote_exec_arg(&exe.to_string_lossy())
    )
}

/// What [`apply_in`] did.
#[derive(Debug, PartialEq, Eq)]
enum Change {
    None,
    Written,
    Removed,
}

/// Makes the entry in `user_dir` (an `applications` folder) right, given
/// whether an `installed` one exists elsewhere. An entry somebody else put in
/// `user_dir` is never touched.
fn apply_in(user_dir: &Path, installed: bool, exe: &Path) -> std::io::Result<Change> {
    let file = user_dir.join(file_name());
    let current = match std::fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let ours = current.as_deref().is_some_and(|t| t.lines().any(|l| l.trim() == GENERATED));
    if current.is_some() && !ours {
        return Ok(Change::None);
    }
    if installed {
        if ours {
            std::fs::remove_file(&file)?;
            return Ok(Change::Removed);
        }
        return Ok(Change::None);
    }
    let text = entry_text(exe);
    if current.as_deref() == Some(text.as_str()) {
        return Ok(Change::None);
    }
    std::fs::create_dir_all(user_dir)?;
    std::fs::write(&file, text)?;
    Ok(Change::Written)
}

/// The `applications` folders of `$XDG_DATA_DIRS` (the spec's default when
/// unset or empty).
fn system_dirs(xdg_data_dirs: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let dirs = xdg_data_dirs.filter(|d| !d.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::env::split_paths(&dirs).filter(|d| d.is_absolute()).map(|d| d.join("applications")).collect()
}

/// Keeps a hidden menu entry for this executable while no installed one
/// exists (nothing for a copy of the data folder, nor in a Flatpak).
pub fn sync(data_paths: &DataPaths) {
    if heronite_platform::flatpak::active() {
        return;
    }
    let user_dir = match crate::env::value("HERONITE_APPLICATIONS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None if data_paths.is_default() => match dirs::data_dir() {
            Some(dir) => dir.join("applications"),
            None => return,
        },
        None => return,
    };
    let name = file_name();
    let installed = system_dirs(std::env::var_os("XDG_DATA_DIRS")).iter().any(|d| *d != user_dir && d.join(&name).is_file());
    match apply_in(&user_dir, installed, &super::shortcuts::own_exe()) {
        Ok(Change::None) => {}
        Ok(Change::Written) => log::info!("wrote a hidden menu entry in {} (the desktop portal names the app by it)", user_dir.display()),
        Ok(Change::Removed) => log::info!("removed the hidden menu entry in {}: an installed one exists", user_dir.display()),
        Err(e) => log::warn!("menu entry in {}: {e}", user_dir.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_entry_is_written_where_none_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("applications");
        let exe = Path::new("/home/me/My Apps/Heronite.AppImage");
        assert_eq!(apply_in(&apps, false, exe).unwrap(), Change::Written);
        let text = std::fs::read_to_string(apps.join("app.heronite.Heronite.desktop")).unwrap();
        assert!(text.contains("Exec=\"/home/me/My Apps/Heronite.AppImage\" %U\n"), "{text}");
        assert!(text.contains("NoDisplay=true\n") && text.contains("X-Heronite-Generated=true\n"), "{text}");
        // Nothing to do the second time.
        assert_eq!(apply_in(&apps, false, exe).unwrap(), Change::None);
        // The AppImage moved.
        assert_eq!(apply_in(&apps, false, Path::new("/opt/Heronite.AppImage")).unwrap(), Change::Written);
        assert!(std::fs::read_to_string(apps.join(file_name())).unwrap().contains("Exec=/opt/Heronite.AppImage %U\n"));
    }

    #[test]
    fn an_installed_entry_replaces_the_hidden_one() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/usr/bin/heronite-rs");
        // Installed from the start: nothing is written.
        assert_eq!(apply_in(dir.path(), true, exe).unwrap(), Change::None);
        assert!(!dir.path().join(file_name()).exists());
        // A package arrives after the hidden entry was written.
        assert_eq!(apply_in(dir.path(), false, exe).unwrap(), Change::Written);
        assert_eq!(apply_in(dir.path(), true, exe).unwrap(), Change::Removed);
        assert!(!dir.path().join(file_name()).exists());
    }

    #[test]
    fn an_entry_somebody_else_wrote_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(file_name());
        let theirs = "[Desktop Entry]\nType=Application\nName=Heronite\nExec=/home/me/.local/bin/heronite-rs %U\n";
        std::fs::write(&file, theirs).unwrap();
        for installed in [false, true] {
            assert_eq!(apply_in(dir.path(), installed, Path::new("/elsewhere/heronite-rs")).unwrap(), Change::None);
            assert_eq!(std::fs::read_to_string(&file).unwrap(), theirs);
        }
    }

    #[test]
    fn system_folders_follow_xdg_data_dirs() {
        assert_eq!(
            system_dirs(Some("/a/share:relative:/b/share".into())),
            [PathBuf::from("/a/share/applications"), PathBuf::from("/b/share/applications")]
        );
        let default = [PathBuf::from("/usr/local/share/applications"), PathBuf::from("/usr/share/applications")];
        assert_eq!(system_dirs(None), default);
        assert_eq!(system_dirs(Some("".into())), default);
    }

    #[test]
    fn a_copy_of_the_data_folder_writes_nothing() {
        if crate::env::value("HERONITE_APPLICATIONS_DIR").is_some() {
            return;
        }
        let data = tempfile::tempdir().unwrap();
        sync(&DataPaths::at(data.path()));
        assert!(std::fs::read_dir(data.path()).unwrap().next().is_none());
    }
}
