//! Where the app keeps its files.
//!
//! Linux follows the XDG base directories: `~/.local/share/<app>`,
//! `~/.config/<app>` and `~/.cache/<app>`. Windows has one roaming folder
//! for both data and settings, so they get a subfolder each:
//! `%APPDATA%\<app>\data`, `%APPDATA%\<app>\config` and
//! `%LOCALAPPDATA%\<app>\cache`. The data and settings folders must never
//! hold one another: backups walk each of them as a whole.
//!
//! A portable copy on Windows (a [`PORTABLE_MARKER`] next to the program)
//! keeps all three in its own folder instead: `data`, `config` and `cache`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The file next to the program that makes a copy portable (Windows). The
/// portable zip has one; deleting it moves the copy to the user's folders.
pub const PORTABLE_MARKER: &str = "portable.txt";

/// The subfolder of a Windows copy (portable or installed) that holds the
/// programs the app starts itself, like the stores' login window, so that
/// the program the user starts is the only one at the top.
pub const HELPERS_DIR: &str = "helpers";

/// The folder of this portable copy: the program's own folder when it holds
/// a [`PORTABLE_MARKER`], or the folder above for a program in its
/// [`HELPERS_DIR`] (Windows only; `None` elsewhere and for installed
/// copies).
pub fn portable_root() -> Option<PathBuf> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        if !cfg!(windows) {
            return None;
        }
        let exe = std::env::current_exe().ok()?;
        portable_for(exe.parent()?)
    })
    .clone()
}

/// The portable copy a program in `dir` belongs to: `dir` itself, or its
/// parent when `dir` is the copy's [`HELPERS_DIR`].
fn portable_for(dir: &Path) -> Option<PathBuf> {
    let helpers = || dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(HELPERS_DIR));
    portable_in(dir).or_else(|| helpers().then(|| dir.parent()).flatten().and_then(portable_in))
}

/// `dir` when it holds a [`PORTABLE_MARKER`].
fn portable_in(dir: &Path) -> Option<PathBuf> {
    dir.join(PORTABLE_MARKER).is_file().then(|| dir.to_path_buf())
}

/// The data folder of `app` (the games, the Collection, the stores' files).
pub fn data(app: &str) -> Option<PathBuf> {
    if let Some(root) = portable_root() {
        return Some(root.join("data"));
    }
    if cfg!(windows) {
        ::dirs::data_dir().map(|d| d.join(app).join("data"))
    } else {
        ::dirs::data_dir().map(|d| d.join(app))
    }
}

/// The settings and UI state of `app`.
pub fn config(app: &str) -> Option<PathBuf> {
    if let Some(root) = portable_root() {
        return Some(root.join("config"));
    }
    if cfg!(windows) {
        ::dirs::config_dir().map(|d| d.join(app).join("config"))
    } else {
        ::dirs::config_dir().map(|d| d.join(app))
    }
}

/// Caches of `app`: anything in it can be thrown away.
pub fn cache(app: &str) -> Option<PathBuf> {
    if let Some(root) = portable_root() {
        return Some(root.join("cache"));
    }
    if cfg!(windows) {
        ::dirs::cache_dir().map(|d| d.join(app).join("cache"))
    } else {
        ::dirs::cache_dir().map(|d| d.join(app))
    }
}

/// Where captures go until the user picks a folder: `Screenshots` in a
/// portable copy (it travels with it), else `<title>` in the user's
/// Pictures folder (`$XDG_PICTURES_DIR` on Linux, the Pictures library on
/// Windows, wherever OneDrive moved it).
pub fn screenshots(title: &str) -> Option<PathBuf> {
    if let Some(root) = portable_root() {
        return Some(root.join("Screenshots"));
    }
    let pictures = ::dirs::picture_dir().or_else(|| home().map(|h| h.join("Pictures")))?;
    Some(pictures.join(title))
}

/// A folder of the temp folder that only this user uses, for `name`:
/// `/tmp/<name>-<uid>` on Unix (the temp folder is shared there), the user's
/// own `%TEMP%\<name>` on Windows.
pub fn user_temp(name: &str) -> PathBuf {
    #[cfg(unix)]
    {
        // SAFETY: getuid has no preconditions.
        std::env::temp_dir().join(format!("{name}-{}", unsafe { libc::getuid() }))
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join(name)
    }
}

/// The user's home folder (`C:\Users\<name>` on Windows).
pub fn home() -> Option<PathBuf> {
    ::dirs::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_and_settings_never_hold_one_another() {
        let (data, config) = (data("heronite").unwrap(), config("heronite").unwrap());
        assert!(!data.starts_with(&config) && !config.starts_with(&data), "{data:?} {config:?}");
        let cache = cache("heronite").unwrap();
        assert!(!cache.starts_with(&data) && !cache.starts_with(&config));
    }

    #[test]
    fn screenshots_go_to_the_pictures_folder() {
        let shots = screenshots("Heronite").unwrap();
        assert!(shots.ends_with("Heronite"), "{shots:?}");
        assert!(!shots.starts_with(data("heronite").unwrap()) && !shots.starts_with(config("heronite").unwrap()));
    }

    #[test]
    fn a_marker_next_to_the_program_makes_it_portable() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(portable_in(d.path()), None);
        std::fs::write(d.path().join(PORTABLE_MARKER), "").unwrap();
        assert_eq!(portable_in(d.path()).as_deref(), Some(d.path()));
        // The test program has none next to it.
        assert_eq!(portable_root(), None);
    }

    #[test]
    fn a_helper_belongs_to_the_copy_above_it() {
        let d = tempfile::tempdir().unwrap();
        let helpers = d.path().join(HELPERS_DIR);
        let other = d.path().join("other");
        std::fs::create_dir(&helpers).unwrap();
        std::fs::create_dir(&other).unwrap();
        assert_eq!(portable_for(&helpers), None);
        std::fs::write(d.path().join(PORTABLE_MARKER), "").unwrap();
        assert_eq!(portable_for(d.path()).as_deref(), Some(d.path()));
        assert_eq!(portable_for(&helpers).as_deref(), Some(d.path()));
        // Only the helpers folder: another subfolder is not part of the copy.
        assert_eq!(portable_for(&other), None);
    }
}
