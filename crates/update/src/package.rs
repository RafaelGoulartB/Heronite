//! System packages (.deb, .rpm, Arch) made by `scripts/packages.sh`. Each one
//! installs `share/heronite/package-format` beside `bin/`, holding its
//! format. Files owned by a package manager must not be replaced behind its
//! back: the app downloads the release's file of the same format, checks its
//! signature like the AppImage's, and hands it to the package manager
//! through `pkexec`.
//!
//! The AUR's `heronite-bin` also writes the marker (`aur`), but it conflicts
//! with our Arch package: the app only tells that a new version is out, and
//! the user's AUR helper installs it.

use heronite_i18n::tr;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::install::InstallError;
use crate::release::ARCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    Deb,
    Rpm,
    Arch,
    /// `heronite-bin` from the AUR: updated by the user's AUR helper.
    Aur,
}

impl Package {
    /// The marker, relative to the prefix (`/usr`).
    pub const MARKER: &'static str = "share/heronite/package-format";

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "deb" => Some(Package::Deb),
            "rpm" => Some(Package::Rpm),
            "arch" => Some(Package::Arch),
            "aur" => Some(Package::Aur),
            _ => None,
        }
    }

    /// The package `exe` (`<prefix>/bin/heronite-rs`) belongs to, if any.
    pub fn detect(exe: &Path) -> Option<Self> {
        let prefix = exe.parent()?.parent()?;
        Self::parse(&std::fs::read_to_string(prefix.join(Self::MARKER)).ok()?)
    }

    /// `.deb`, `.rpm`, `Arch`: "installed from a … package".
    pub fn label(self) -> &'static str {
        match self {
            Package::Deb => ".deb",
            Package::Rpm => ".rpm",
            Package::Arch => "Arch",
            Package::Aur => "AUR",
        }
    }

    /// The command a user types to install a downloaded `file` (an absolute
    /// path: apt and dnf need a `/` to read a file instead of a name). The
    /// AUR has no file: its helper updates the package.
    pub fn install_command(self, file: &str) -> String {
        match self {
            Package::Deb => format!("sudo apt install {file}"),
            Package::Rpm => format!("sudo dnf install {file}"),
            Package::Arch => format!("sudo pacman -U {file}"),
            Package::Aur => "yay -Syu".into(),
        }
    }

    /// End of this format's file name in a release (nfpm's default names:
    /// `heronite_0.2.0-1_amd64.deb`, `heronite-0.2.0-1.x86_64.rpm`,
    /// `heronite-0.2.0-1-x86_64.pkg.tar.zst`). `None` for the AUR: the app
    /// never downloads its package.
    pub fn file_suffix(self) -> Option<String> {
        match self {
            Package::Deb => Some(format!("_{}.deb", if ARCH == "aarch64" { "arm64" } else { "amd64" })),
            Package::Rpm => Some(format!(".{ARCH}.rpm")),
            Package::Arch => Some(format!("-{ARCH}.pkg.tar.zst")),
            Package::Aur => None,
        }
    }
}

/// Finds `name` on `PATH`.
fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

impl Package {
    /// The package manager call (run as root) that installs `file`; `None`
    /// when the manager is not on `PATH` (and for the AUR, which the app does
    /// not install). Dependencies come from the configured repositories.
    pub fn install_args(self, file: &Path) -> Option<Vec<OsString>> {
        let file = file.as_os_str().to_owned();
        let args = |tool: PathBuf, rest: &[&str]| {
            let mut v = vec![tool.into_os_string()];
            v.extend(rest.iter().map(OsString::from));
            v.push(file.clone());
            v
        };
        match self {
            Package::Deb => which("apt-get").map(|t| args(t, &["install", "-y"])),
            Package::Rpm => which("dnf")
                .map(|t| args(t, &["install", "-y"]))
                .or_else(|| which("zypper").map(|t| args(t, &["--non-interactive", "install", "--allow-unsigned-rpm"]))),
            Package::Arch => which("pacman").map(|t| args(t, &["-U", "--noconfirm"])),
            Package::Aur => None,
        }
    }

    /// Installs the downloaded `file` through `pkexec` (the desktop's
    /// password prompt). Blocks until the package manager is done. On
    /// failure the caller offers [`Package::install_command`].
    pub fn install(self, file: &Path) -> Result<(), InstallError> {
        let pkexec = which("pkexec").ok_or_else(|| InstallError::Failed(tr!("pkexec is not installed").into()))?;
        let args = self.install_args(file).ok_or_else(|| InstallError::Failed(tr!("No package manager found").into()))?;
        let out = Command::new(pkexec)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| InstallError::Failed(tr!("Could not run pkexec: {error}", error = e)))?;
        match out.status.code() {
            Some(0) => Ok(()),
            // The password prompt was dismissed.
            Some(126) => Err(InstallError::Cancelled),
            Some(127) => Err(InstallError::Failed(tr!("Not authorized (is a polkit agent running?)").into())),
            _ => {
                let text = String::from_utf8_lossy(&out.stderr);
                let text = if text.trim().is_empty() { String::from_utf8_lossy(&out.stdout) } else { text };
                let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
                let tail = tail[tail.len().saturating_sub(3)..].join(" ");
                Err(InstallError::Failed(tr!("The package manager failed: {tail}", tail)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Package;

    #[test]
    fn detects_the_marker_beside_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("usr/bin/heronite-rs");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(Package::detect(&exe), None);
        let marker = tmp.path().join("usr").join(Package::MARKER);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "rpm\n").unwrap();
        assert_eq!(Package::detect(&exe), Some(Package::Rpm));
        std::fs::write(&marker, "aur\n").unwrap();
        assert_eq!(Package::detect(&exe), Some(Package::Aur));
        std::fs::write(&marker, "snap").unwrap();
        assert_eq!(Package::detect(&exe), None);
        assert_eq!(Package::Deb.install_command("/tmp/heronite_0.2.0_amd64.deb"), "sudo apt install /tmp/heronite_0.2.0_amd64.deb");
        if super::ARCH == "x86_64" {
            assert_eq!(Package::Deb.file_suffix().as_deref(), Some("_amd64.deb"));
            assert_eq!(Package::Arch.file_suffix().as_deref(), Some("-x86_64.pkg.tar.zst"));
        }
    }

    #[test]
    fn the_aur_package_is_never_downloaded_nor_installed() {
        assert_eq!(Package::parse("aur"), Some(Package::Aur));
        assert_eq!(Package::Aur.label(), "AUR");
        assert_eq!(Package::Aur.file_suffix(), None);
        assert_eq!(Package::Aur.install_args(std::path::Path::new("/tmp/heronite.pkg.tar.zst")), None);
    }
}
