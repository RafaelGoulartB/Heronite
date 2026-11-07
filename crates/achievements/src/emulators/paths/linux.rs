//! Where the emulators keep their config on Linux: the XDG folders, the
//! older dotfiles some still read, and each Flatpak's sandbox.

use std::path::{Path, PathBuf};

use super::ConfigPaths;
use crate::emulators::procs::flatpak_id;
use crate::emulators::Emulator;

impl ConfigPaths {
    pub(in crate::emulators) fn linux(home: &Path) -> Self {
        Self { paths: Emulator::ALL.into_iter().map(|emulator| (emulator, candidates(home, emulator))).collect(), ..Self::default() }
    }
}

/// An absolute `$name` that points inside `home`. Anything else is ignored,
/// so a test home never reaches the real one.
fn env_dir(name: &str, home: &Path) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(name)?);
    (dir.is_absolute() && dir.starts_with(home)).then_some(dir)
}

/// `$XDG_CONFIG_HOME` (when set) and `~/.config`.
fn config_homes(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = env_dir("XDG_CONFIG_HOME", home).into_iter().collect();
    let default = home.join(".config");
    if !dirs.contains(&default) {
        dirs.push(default);
    }
    dirs
}

/// Every place the config file can be: native (and AppImage) then Flatpak,
/// whose sandbox sets `XDG_CONFIG_HOME` to `~/.var/app/<id>/config`.
/// The files may not exist.
fn candidates(home: &Path, emulator: Emulator) -> Vec<PathBuf> {
    let under = |config_home: PathBuf| -> PathBuf {
        match emulator {
            Emulator::RetroArch => config_home.join("retroarch/retroarch.cfg"),
            Emulator::DuckStation => config_home.join("duckstation/settings.ini"),
            Emulator::Pcsx2 => config_home.join("PCSX2/inis/PCSX2.ini"),
            Emulator::Ppsspp => config_home.join("ppsspp/PSP/SYSTEM/ppsspp.ini"),
            Emulator::Dolphin => config_home.join("dolphin-emu/RetroAchievements.ini"),
            Emulator::Flycast => config_home.join("flycast/emu.cfg"),
        }
    };
    let mut paths: Vec<PathBuf> = match emulator {
        // DuckStation (`src/core/core.cpp`) uses `$XDG_CONFIG_HOME/duckstation`
        // when it is set and `~/.local/share/duckstation` otherwise.
        Emulator::DuckStation => {
            let mut p: Vec<PathBuf> = env_dir("XDG_CONFIG_HOME", home).map(under).into_iter().collect();
            p.push(home.join(".local/share/duckstation/settings.ini"));
            p
        }
        _ => config_homes(home).into_iter().map(under).collect(),
    };
    match emulator {
        // RetroArch also reads the old `~/.retroarch.cfg`.
        Emulator::RetroArch => paths.push(home.join(".retroarch.cfg")),
        // Dolphin (`UICommon.cpp`) keeps using `~/.dolphin-emu` when it exists.
        Emulator::Dolphin => paths.insert(0, home.join(".dolphin-emu/Config/RetroAchievements.ini")),
        _ => {}
    }
    paths.push(under(home.join(".var/app").join(flatpak_id(emulator)).join("config")));
    paths
}
