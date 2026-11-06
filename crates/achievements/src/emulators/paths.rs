//! Config discovery is separate from editing: portable Windows emulators
//! live beside their registered executable, not necessarily under the home.

use std::path::{Path, PathBuf};

use heronite_launch::emulators::EmulatorsFile;
use heronite_launch::probe::Probe;

use super::Emulator;

#[cfg(any(not(windows), test))]
mod linux;
#[cfg(any(windows, test))]
mod windows;

#[derive(Clone, Default)]
pub struct ConfigPaths {
    paths: Vec<(Emulator, Vec<PathBuf>)>,
    pub(super) retroarch_programs: Vec<PathBuf>,
    /// Configs that could not be found, and why. The others are still
    /// written; each of these is reported as a failure beside them.
    skipped: Vec<(Emulator, String)>,
}

impl ConfigPaths {
    /// Reload registered installations so adding a portable emulator does not
    /// require restarting Heronite. Malformed registrations are not ignored.
    pub fn load(home: &Path, paths: &heronite_data::DataPaths) -> Result<Self, String> {
        let registered = if cfg!(windows) {
            heronite_data::json_store::read(&paths.emulators()).map_err(|e| e.to_string())?
        } else {
            EmulatorsFile::default()
        };
        Self::detect(home, &registered, &heronite_launch::probe::SystemProbe)
    }

    /// Uses native Linux locations, or on Windows the registered RetroArch
    /// executables (including portable copies), PATH and APPDATA.
    pub fn detect(home: &Path, registered: &EmulatorsFile, probe: &dyn Probe) -> Result<Self, String> {
        #[cfg(windows)]
        {
            let _ = home;
            Ok(Self::windows(registered, probe))
        }
        #[cfg(not(windows))]
        {
            let _ = (registered, probe);
            Ok(Self::linux(home))
        }
    }

    pub(super) fn candidates(&self, emulator: Emulator) -> Vec<PathBuf> {
        self.paths.iter().find(|(kind, _)| *kind == emulator).map(|(_, paths)| paths.clone()).unwrap_or_default()
    }

    /// Why configs of `emulator` were left out.
    pub(super) fn skipped(&self, emulator: Emulator) -> impl Iterator<Item = &str> {
        self.skipped.iter().filter(move |(kind, _)| *kind == emulator).map(|(_, why)| why.as_str())
    }
}

/// What two spellings of one file share: Windows ignores case and takes
/// either separator, Linux does not.
pub(super) fn same_file(path: &Path) -> String {
    fold(path, cfg!(windows))
}

fn fold(path: &Path, windows: bool) -> String {
    let text = path.to_string_lossy();
    if windows {
        text.replace('/', "\\").to_lowercase()
    } else {
        text.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_spellings_of_one_file_are_one() {
        let windows = |p: &str| fold(Path::new(p), true);
        assert_eq!(windows(r"C:\RetroArch\RetroArch.exe"), windows("c:/retroarch/retroarch.EXE"));
        assert_ne!(fold(Path::new("/a/RetroArch"), false), fold(Path::new("/a/retroarch"), false));
    }
}
