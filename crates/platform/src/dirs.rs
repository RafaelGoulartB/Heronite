//! Where the app keeps its files.
//!
//! Linux follows the XDG base directories: `~/.local/share/<app>`,
//! `~/.config/<app>` and `~/.cache/<app>`. Windows has one roaming folder
//! for both data and settings, so they get a subfolder each:
//! `%APPDATA%\<app>\data`, `%APPDATA%\<app>\config` and
//! `%LOCALAPPDATA%\<app>\cache`. The data and settings folders must never
//! hold one another: backups walk each of them as a whole.

use std::path::PathBuf;

/// The data folder of `app` (the games, the Collection, the stores' files).
pub fn data(app: &str) -> Option<PathBuf> {
    if cfg!(windows) {
        ::dirs::data_dir().map(|d| d.join(app).join("data"))
    } else {
        ::dirs::data_dir().map(|d| d.join(app))
    }
}

/// The settings and UI state of `app`.
pub fn config(app: &str) -> Option<PathBuf> {
    if cfg!(windows) {
        ::dirs::config_dir().map(|d| d.join(app).join("config"))
    } else {
        ::dirs::config_dir().map(|d| d.join(app))
    }
}

/// Caches of `app`: anything in it can be thrown away.
pub fn cache(app: &str) -> Option<PathBuf> {
    if cfg!(windows) {
        ::dirs::cache_dir().map(|d| d.join(app).join("cache"))
    } else {
        ::dirs::cache_dir().map(|d| d.join(app))
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
}
