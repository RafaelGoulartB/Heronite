//! Whether a build of a game runs on this system, and how: natively, through
//! Wine, or not at all. One rule for every kind of game (games added by
//! hand, emulators, the stores' builds):
//!
//! - a build made for this system runs as it is;
//! - a Windows build runs through Wine where Wine is
//!   (`heronite_platform::Feature::Wine`: Linux);
//! - anything else does not run here (a Linux build on Windows, a macOS
//!   build anywhere).

use heronite_i18n::tr;

use crate::native::Platform;

/// The system a build of a game is made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Os {
    Windows,
    Linux,
    Mac,
}

impl Os {
    /// The system Heronite runs on.
    pub const HOST: Os = if cfg!(windows) { Os::Windows } else { Os::Linux };

    /// A store's name for a platform, any case: Epic's `Windows` / `Mac`,
    /// GOG's `windows` / `linux` / `osx`, Amazon's `Windows`.
    pub fn from_name(name: &str) -> Option<Os> {
        match name.trim().to_ascii_lowercase().as_str() {
            "windows" | "win" | "win32" | "win64" => Some(Os::Windows),
            "linux" => Some(Os::Linux),
            "mac" | "macos" | "osx" | "darwin" => Some(Os::Mac),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Os::Windows => "Windows",
            Os::Linux => "Linux",
            Os::Mac => "macOS",
        }
    }
}

impl From<Platform> for Os {
    fn from(p: Platform) -> Os {
        match p {
            Platform::Windows => Os::Windows,
            Platform::Linux => Os::Linux,
        }
    }
}

/// How a build runs here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    /// As it is: it is made for this system.
    Native,
    /// Through Wine or Proton.
    Wine,
    /// Not on this system ([`unavailable_reason`] says why).
    Unavailable,
}

/// How a build made for `build` runs on this system.
pub fn run_mode(build: Os) -> RunMode {
    run_mode_on(Os::HOST, heronite_platform::supported(heronite_platform::Feature::Wine), build)
}

fn run_mode_on(host: Os, wine: bool, build: Os) -> RunMode {
    if build == host {
        RunMode::Native
    } else if build == Os::Windows && wine {
        RunMode::Wine
    } else {
        RunMode::Unavailable
    }
}

/// Why a build made for `build` does not run here, for the user.
pub fn unavailable_reason(build: Os) -> String {
    match build {
        Os::Linux if Os::HOST == Os::Windows => tr!("Linux programs do not run on Windows").to_string(),
        Os::Windows => tr!("Windows programs do not run on {system} without Wine", system = Os::HOST.name()),
        _ => tr!("{platform} programs do not run on {system}", platform = build.name(), system = Os::HOST.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_system_runs_its_own_builds_and_linux_runs_windows_ones_through_wine() {
        use Os::*;
        use RunMode::*;
        // Windows: no Wine.
        assert_eq!([Windows, Linux, Mac].map(|b| run_mode_on(Windows, false, b)), [Native, Unavailable, Unavailable]);
        // Linux, with Wine and (a debug build's `HERONITE_FEATURES=-wine`) without.
        assert_eq!([Windows, Linux, Mac].map(|b| run_mode_on(Linux, true, b)), [Wine, Native, Unavailable]);
        assert_eq!([Windows, Linux, Mac].map(|b| run_mode_on(Linux, false, b)), [Unavailable, Native, Unavailable]);
        // This build.
        assert_eq!(run_mode(Os::HOST), Native);
        assert_eq!(run_mode(Mac), Unavailable);
        assert_eq!(run_mode(Windows), if cfg!(windows) { Native } else { Wine });
        assert_eq!(run_mode(Linux), if cfg!(windows) { Unavailable } else { Native });
    }

    #[test]
    fn store_platform_names() {
        assert_eq!(Os::from_name("Windows"), Some(Os::Windows));
        assert_eq!(Os::from_name("windows"), Some(Os::Windows));
        assert_eq!(Os::from_name("linux"), Some(Os::Linux));
        assert_eq!(Os::from_name("osx"), Some(Os::Mac));
        assert_eq!(Os::from_name("Mac"), Some(Os::Mac));
        assert_eq!(Os::from_name("amiga"), None);
    }

    #[test]
    fn reasons_name_the_systems() {
        assert!(unavailable_reason(Os::Mac).contains("macOS"));
        if cfg!(windows) {
            assert_eq!(unavailable_reason(Os::Linux), "Linux programs do not run on Windows");
        }
    }
}
