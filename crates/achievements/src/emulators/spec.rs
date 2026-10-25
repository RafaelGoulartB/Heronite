//! Where each emulator keeps its RetroAchievements login and how it writes
//! it. Every name below is checked against the emulator's source (the file
//! is named per emulator).

use super::procs::flatpak_id;
use super::Emulator;
use std::path::{Path, PathBuf};

/// How an emulator spells a boolean in its file.
#[derive(Clone, Copy)]
pub enum Bools {
    /// `true` / `false`.
    Lower,
    /// `True` / `False`.
    Title,
    /// `yes` / `no`.
    YesNo,
}

impl Bools {
    pub fn word(self, on: bool) -> &'static str {
        match (self, on) {
            (Bools::Lower, true) => "true",
            (Bools::Lower, false) => "false",
            (Bools::Title, true) => "True",
            (Bools::Title, false) => "False",
            (Bools::YesNo, true) => "yes",
            (Bools::YesNo, false) => "no",
        }
    }
}

/// Where the Connect token lives.
#[derive(Clone, Copy)]
pub enum Token {
    /// A key in the config file.
    Plain(&'static str),
    /// A key in the config file, encrypted (`secret.rs`).
    Encrypted(&'static str),
    /// A file of its own next to the config file, holding only the token.
    SideFile(&'static str),
    /// A key in the config file, or in `secrets.ini` next to it when that exists.
    Secrets(&'static str),
}

pub struct Spec {
    /// `""` for a flat file.
    pub section: &'static str,
    pub enabled: &'static str,
    pub hardcore: &'static str,
    pub username: &'static str,
    pub token: Token,
    pub timestamp: Option<&'static str>,
    /// Keys that hold no secret of ours; blanked at sign-in when present.
    pub blank: &'static [&'static str],
    pub bools: Bools,
    /// Values are written as `key = "value"`.
    pub quote: bool,
}

pub fn spec(emulator: Emulator) -> Spec {
    match emulator {
        // RetroArch `configuration.c`: bools `cheevos_enable` and
        // `cheevos_hardcore_mode_enable`, strings `cheevos_username`,
        // `cheevos_password` (RetroArch blanks it after a token login) and
        // `cheevos_token`; `config_file.c` writes `key = "value"`.
        Emulator::RetroArch => Spec {
            section: "",
            enabled: "cheevos_enable",
            hardcore: "cheevos_hardcore_mode_enable",
            username: "cheevos_username",
            token: Token::Plain("cheevos_token"),
            timestamp: None,
            blank: &["cheevos_password"],
            bools: Bools::Lower,
            quote: true,
        },
        // DuckStation `src/core/settings.cpp` (section `Cheevos`, `Enabled`,
        // `ChallengeMode`) and `src/core/achievements.cpp` (`SaveLoginSettings`:
        // `Username`, encrypted `Token`, `LoginTimestamp`).
        Emulator::DuckStation => Spec {
            section: "Cheevos",
            enabled: "Enabled",
            hardcore: "ChallengeMode",
            username: "Username",
            token: Token::Encrypted("Token"),
            timestamp: Some("LoginTimestamp"),
            blank: &[],
            bools: Bools::Lower,
            quote: false,
        },
        // PCSX2 `pcsx2/Achievements.cpp` (`Username`, `LoginTimestamp`, and
        // the token in `secrets.ini` in current builds, in the ini in older
        // ones) and `pcsx2-qt/Settings/AchievementSettingsWidget.cpp`
        // (`Enabled`, `ChallengeMode`).
        Emulator::Pcsx2 => Spec {
            section: "Achievements",
            enabled: "Enabled",
            hardcore: "ChallengeMode",
            username: "Username",
            token: Token::Secrets("Token"),
            timestamp: Some("LoginTimestamp"),
            blank: &[],
            bools: Bools::Lower,
            quote: false,
        },
        // PPSSPP `Core/Config.cpp` (`achievementSettings`: section
        // `Achievements`, `AchievementsEnable`, `AchievementsChallengeMode`,
        // `AchievementsUserName`) and `Core/RetroAchievements.cpp` +
        // `UI/NativeApp.cpp` (the token is the secret `retroachievements`, the
        // file `ppsspp_retroachievements.dat` next to `ppsspp.ini`).
        Emulator::Ppsspp => Spec {
            section: "Achievements",
            enabled: "AchievementsEnable",
            hardcore: "AchievementsChallengeMode",
            username: "AchievementsUserName",
            token: Token::SideFile("ppsspp_retroachievements.dat"),
            timestamp: None,
            blank: &[],
            bools: Bools::Title,
            quote: false,
        },
        // Dolphin `Source/Core/Core/Config/AchievementSettings.cpp` (section
        // `Achievements`: `Enabled`, `Username`, `ApiToken`, `HardcoreEnabled`)
        // and `Common/CommonPaths.h` (`RetroAchievements.ini`).
        Emulator::Dolphin => Spec {
            section: "Achievements",
            enabled: "Enabled",
            hardcore: "HardcoreEnabled",
            username: "Username",
            token: Token::Plain("ApiToken"),
            timestamp: None,
            blank: &[],
            bools: Bools::Title,
            quote: false,
        },
        // Flycast `core/cfg/option.cpp` (section `achievements`: `Enabled`,
        // `HardcoreMode`, `UserName`, `Token`) and `core/cfg/ini.h` (`yes`/`no`).
        Emulator::Flycast => Spec {
            section: "achievements",
            enabled: "Enabled",
            hardcore: "HardcoreMode",
            username: "UserName",
            token: Token::Plain("Token"),
            timestamp: None,
            blank: &[],
            bools: Bools::YesNo,
            quote: false,
        },
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
pub fn candidates(home: &Path, emulator: Emulator) -> Vec<PathBuf> {
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
