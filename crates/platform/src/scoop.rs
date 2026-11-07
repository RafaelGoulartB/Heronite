//! Installed by Scoop on Windows (`packaging/scoop/heronite.json`).
//!
//! Scoop unpacks every version of an app into its own folder,
//! `<root>\apps\<app>\<version>\`, and points the junction
//! `<root>\apps\<app>\current` at the installed one; its commands are in
//! `<root>\shims`. The bucket's manifest also writes [`FORMAT`] into
//! [`MARKER`] next to the program, and takes the portable zip's
//! `portable.txt` out, so the copy keeps its data in the user's folders like
//! an installed one.
//!
//! Scoop owns these files: the app only tells that a new version is out
//! (`scoop update heronite` installs it), and whatever must start it later
//! (shortcuts, the autostart entry, `heronite://` links) goes through
//! `current` ([`current`]), which follows every update.

use std::path::{Path, PathBuf};

/// The file next to the program that says how it was installed (the
/// Windows setup writes `windows-installer` into the same file).
pub const MARKER: &str = "package-format";
/// Its content for a copy installed by Scoop.
pub const FORMAT: &str = "scoop";

/// `<root>\apps\<app>` when `exe` sits in a version folder of a Scoop app:
/// the folders are named that way and `<root>\shims` exists.
fn app_dir(exe: &Path) -> Option<&Path> {
    let app = exe.parent()?.parent()?;
    let apps = app.parent()?;
    let root = apps.parent()?;
    let named_apps = apps.file_name()?.to_str()?.eq_ignore_ascii_case("apps");
    (named_apps && root.join("shims").is_dir()).then_some(app)
}

/// `exe` was installed by Scoop: its marker says so, or Scoop's folders are
/// around it (a manifest from another bucket that writes no marker).
pub fn installed(exe: &Path) -> bool {
    let marker = exe.parent().map(|dir| dir.join(MARKER));
    let marked = marker.and_then(|m| std::fs::read_to_string(m).ok()).is_some_and(|text| text.trim() == FORMAT);
    marked || app_dir(exe).is_some()
}

/// The same program through `current`, for what starts the app later: a
/// version folder is deleted by `scoop cleanup`, `current` always points at
/// the installed version. `None` outside Scoop, or when `exe` already goes
/// through `current`.
pub fn current(exe: &Path) -> Option<PathBuf> {
    let app = app_dir(exe)?;
    let version = exe.parent()?.file_name()?.to_str()?;
    if version.eq_ignore_ascii_case("current") {
        return None;
    }
    let path = app.join("current").join(exe.file_name()?);
    path.is_file().then_some(path)
}

/// The program a shim in `<root>\shims` starts, from the `.shim` file Scoop
/// writes beside it (`path = "C:\...\retroarch.exe"`, then maybe `args = ...`;
/// older versions leave the path unquoted). `None` when it names none.
pub fn shim_target(shim_file: &str) -> Option<PathBuf> {
    shim_file.trim_start_matches('\u{feff}').lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        let value = value.trim().trim_matches('"');
        (key.trim().eq_ignore_ascii_case("path") && !value.is_empty()).then(|| PathBuf::from(value))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Scoop root with Heronite 1.2.0 in it, `current` a plain folder
    /// (junctions are Windows-only). Returns the root and the 1.2.0 program.
    fn scoop_root() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("shims")).unwrap();
        let app = tmp.path().join("apps").join("heronite");
        for version in ["1.2.0", "current"] {
            std::fs::create_dir_all(app.join(version)).unwrap();
            std::fs::write(app.join(version).join("heronite.exe"), "").unwrap();
        }
        (tmp, app.join("1.2.0").join("heronite.exe"))
    }

    #[test]
    fn scoops_folders_mark_a_copy_even_without_the_marker() {
        let (tmp, exe) = scoop_root();
        assert!(installed(&exe));
        assert!(installed(&tmp.path().join("apps/heronite/current/heronite.exe")));
        // The same folders without Scoop's shims: not Scoop.
        std::fs::remove_dir(tmp.path().join("shims")).unwrap();
        assert!(!installed(&exe));
    }

    #[test]
    fn the_marker_alone_is_enough() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("Heronite").join("heronite.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert!(!installed(&exe));
        std::fs::write(exe.with_file_name(MARKER), "windows-installer\r\n").unwrap();
        assert!(!installed(&exe), "the setup's copy");
        std::fs::write(exe.with_file_name(MARKER), "scoop\r\n").unwrap();
        assert!(installed(&exe));
    }

    #[test]
    fn shims_name_their_program() {
        let quoted = "\u{feff}path = \"C:\\Users\\a b\\scoop\\apps\\retroarch\\current\\retroarch.exe\"\r\nargs = --verbose\r\n";
        assert_eq!(shim_target(quoted), Some(PathBuf::from(r"C:\Users\a b\scoop\apps\retroarch\current\retroarch.exe")));
        assert_eq!(shim_target("path = C:\\scoop\\apps\\x\\current\\x.exe\n"), Some(PathBuf::from(r"C:\scoop\apps\x\current\x.exe")));
        assert_eq!(shim_target("args = -v\npath = \"\"\n"), None);
        assert_eq!(shim_target(""), None);
    }

    #[test]
    fn shortcuts_go_through_current() {
        let (tmp, exe) = scoop_root();
        let current_exe = tmp.path().join("apps").join("heronite").join("current").join("heronite.exe");
        assert_eq!(current(&exe), Some(current_exe.clone()));
        assert_eq!(current(&current_exe), None, "already there");
        let elsewhere = tmp.path().join("Games").join("heronite.exe");
        assert_eq!(current(&elsewhere), None);
    }
}
