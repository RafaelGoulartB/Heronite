//! Whether Sunshine (or Apollo) looks installed on this computer. Only used
//! to word the message when nothing answers at the address: "not installed"
//! and "not running" call for different steps.

use std::path::{Path, PathBuf};

/// Where the installers put the program.
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432"] {
            if let Some(dir) = std::env::var_os(var).map(PathBuf::from) {
                for app in ["Sunshine", "Apollo"] {
                    out.push(dir.join(app).join("sunshine.exe"));
                }
            }
        }
    } else {
        for dir in ["/usr/bin", "/usr/local/bin", "/opt/sunshine"] {
            out.push(Path::new(dir).join("sunshine"));
        }
        let flatpak = "exports/bin/dev.lizardbyte.app.Sunshine";
        out.push(Path::new("/var/lib/flatpak").join(flatpak));
        if let Some(home) = dirs_home() {
            out.push(home.join(".local/share/flatpak").join(flatpak));
        }
    }
    out
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// `true` when Sunshine or Apollo is found in its usual places.
pub fn installed() -> bool {
    candidates().iter().any(|p| p.is_file()) || on_path(if cfg!(windows) { "sunshine.exe" } else { "sunshine" })
}
