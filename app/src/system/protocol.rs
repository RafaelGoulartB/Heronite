//! `heronite://` links (games added to Steam, shortcuts): Windows starts
//! this executable with the link when they are opened. Linux registers the
//! scheme with the app's `.desktop` file (`scripts/install.sh`); Windows has
//! no installer yet, so the scheme is registered for the user here, in
//! `HKCU\Software\Classes\heronite`, at every start (the executable may
//! have moved).
//!
//! Like the autostart entry, only the real data folder drives it: a run
//! against a copy (`HERONITE_DATA_DIR`, a development build) must not take
//! the links over. Nor does a portable copy, which leaves nothing in the
//! registry on its own.
//!
//! Blocking (registry writes): call it off the UI thread.

use heronite_data::DataPaths;

/// Registers `heronite://` for this executable (Windows; nothing elsewhere,
/// nor for a copy of the data folder).
pub fn sync(data_paths: &DataPaths) {
    #[cfg(windows)]
    if data_paths.is_default() && heronite_platform::dirs::portable_root().is_none() {
        let exe = super::shortcuts::own_exe();
        if let Err(e) = windows::register(windows::CLASSES_KEY, &exe) {
            log::warn!("registering heronite:// links: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = data_paths;
}

#[cfg(windows)]
mod windows {
    use std::path::Path;

    use heronite_platform::registry::{set_string, string, Hive};

    /// The scheme's key under `HKEY_CURRENT_USER`.
    pub(super) const CLASSES_KEY: &str = r"Software\Classes\heronite";

    /// Writes the scheme under `key` (`HKEY_CURRENT_USER`), only what differs.
    pub(super) fn register(key: &str, exe: &Path) -> std::io::Result<()> {
        let exe = exe.display();
        let open = format!(r"{key}\shell\open\command");
        let values = [
            (key.to_string(), "", "URL:Heronite".to_string()),
            (key.to_string(), "URL Protocol", String::new()),
            (format!(r"{key}\DefaultIcon"), "", format!("\"{exe}\",0")),
            (open, "", format!("\"{exe}\" \"%1\"")),
        ];
        for (key, name, value) in values {
            if string(Hive::CurrentUser, &key, name).as_deref() != Some(value.as_str()) {
                set_string(Hive::CurrentUser, &key, name, &value)?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_scheme_opens_this_executable_with_the_link() {
            let key = format!(r"Software\Heronite\Tests\protocol-{}", std::process::id());
            let exe = Path::new(r"C:\Program Files\Heronite\heronite-rs.exe");
            register(&key, exe).unwrap();
            // Again: nothing changes.
            register(&key, exe).unwrap();
            let read = |sub: &str, name: &str| string(Hive::CurrentUser, &format!("{key}{sub}"), name);
            assert_eq!(read("", "").as_deref(), Some("URL:Heronite"));
            assert_eq!(read("", "URL Protocol").as_deref(), Some(""));
            assert_eq!(read(r"\shell\open\command", "").as_deref(), Some(r#""C:\Program Files\Heronite\heronite-rs.exe" "%1""#));
            assert_eq!(read(r"\DefaultIcon", "").as_deref(), Some(r#""C:\Program Files\Heronite\heronite-rs.exe",0"#));
            let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\{key}"), "/f"]).output();
        }
    }
}
