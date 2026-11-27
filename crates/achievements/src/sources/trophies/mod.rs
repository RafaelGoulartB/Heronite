//! Emulator trophies: what RPCS3 (PlayStation 3) and shadPS4 (PlayStation 4)
//! keep of a game's trophies, and what Xenia (Xbox 360) keeps of its
//! achievements, read from the emulator's own files. Only read, never
//! written.
//!
//! Each emulator's format is its own (`rpcs3`, `shadps4`, `xenia`). The
//! files are the emulator's and may be half written, from another version or
//! damaged: every read is bounded and checked, and anything unexpected is a
//! [`Fetch::Failed`] (or [`Fetch::None`]) of this source alone. The release
//! build aborts on a panic, so nothing here may panic: no indexing that can
//! go out of bounds, no `unwrap` on what a file says.

mod psxml;
pub mod rpcs3;
pub mod shadps4;
pub mod xenia;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use heronite_launch::emulators::{EmulatorsFile, UserEmulator};

use super::{Ctx, Fetch};
use crate::target::{TrophyEmulator, TrophyTarget};

/// A trophy file bigger than this is not one (the largest real ones are a
/// few hundred KiB).
const MAX_XML: u64 = 8 << 20;
/// Folder entries looked at in one folder: a trophy folder of a few
/// hundred games is common, tens of thousands is not one.
const MAX_ENTRIES: usize = 20_000;

/// Where this PC keeps per-user files, for the emulators that use them.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` (when it is inside the home), then `~/.config`.
    pub config_homes: Vec<PathBuf>,
    /// `$XDG_DATA_HOME` (when it is inside the home), then `~/.local/share`.
    pub data_homes: Vec<PathBuf>,
    /// Windows' `%APPDATA%`.
    pub appdata: Option<PathBuf>,
    /// Windows' Documents.
    pub documents: Option<PathBuf>,
}

impl Env {
    pub fn detect() -> Option<Env> {
        let home = heronite_platform::dirs::home()?;
        let inside = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|d| d.is_absolute() && d.starts_with(&home));
        let homes = |var: &str, default: PathBuf| {
            let mut v: Vec<PathBuf> = inside(var).into_iter().collect();
            if !v.contains(&default) {
                v.push(default);
            }
            v
        };
        let config_homes = homes("XDG_CONFIG_HOME", home.join(".config"));
        let data_homes = homes("XDG_DATA_HOME", home.join(".local/share"));
        let (appdata, documents) = if cfg!(windows) {
            (std::env::var_os("APPDATA").map(PathBuf::from).filter(|p| p.is_absolute()), Some(home.join("Documents")))
        } else {
            (None, None)
        };
        Some(Env { home, config_homes, data_homes, appdata, documents })
    }

    /// The sandbox of a Flatpak: `~/.var/app/<id>/<kind>`.
    pub fn flatpak(&self, id: &str, kind: &str) -> PathBuf {
        self.home.join(".var/app").join(id).join(kind)
    }
}

/// How the Collection runs the emulator: where its executable is (a portable
/// copy keeps its files next to it) and its Flatpak.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Install {
    pub exe_dir: Option<PathBuf>,
    pub flatpak_id: Option<String>,
}

impl Install {
    fn of(e: &UserEmulator) -> Install {
        let exe = PathBuf::from(&e.executable);
        let exe_dir = exe.is_absolute().then(|| exe.parent().map(Path::to_path_buf)).flatten();
        let install_dir = e.install_dir.as_deref().map(PathBuf::from).filter(|d| d.is_absolute());
        Install { exe_dir: exe_dir.or(install_dir), flatpak_id: e.flatpak_id.clone().filter(|id| !id.is_empty()) }
    }
}

/// The installs of the game's emulator the Collection knows: the game's own
/// one, else every one of that emulator. A registry that cannot be read
/// gives none (the usual places are still looked at).
fn installs(ctx: &Ctx, target: &TrophyTarget) -> Vec<Install> {
    let file: EmulatorsFile = match heronite_data::json_store::read(&ctx.paths.emulators()) {
        Ok(f) => f,
        Err(e) => {
            log::debug!("emulator trophies: the emulators file: {e:#}");
            return Vec::new();
        }
    };
    if let Some(e) = target.emulator_id.as_deref().and_then(|id| file.get(id)) {
        return vec![Install::of(e)];
    }
    let definition = match target.emulator {
        TrophyEmulator::Rpcs3 => "rpcs3",
        TrophyEmulator::ShadPs4 => "shadps4",
        TrophyEmulator::Xenia => "xenia",
    };
    file.emulators.iter().filter(|e| e.definition_id == definition).map(Install::of).collect()
}

pub fn fetch(ctx: &Ctx, target: &TrophyTarget) -> Fetch {
    let Some(env) = Env::detect() else { return Fetch::None };
    let installs = installs(ctx, target);
    match target.emulator {
        TrophyEmulator::Rpcs3 => rpcs3::fetch(&env, &installs, target),
        TrophyEmulator::ShadPs4 => shadps4::fetch(&env, &installs, target, ctx.lang),
        TrophyEmulator::Xenia => xenia::fetch(&env, &installs, target, &ctx.paths.achievements_dir()),
    }
}

/// The files whose change may mean new unlocks (for the live check while
/// the game runs), including ones that do not exist yet.
pub fn watched_files(ctx: &Ctx, target: &TrophyTarget) -> Vec<PathBuf> {
    let Some(env) = Env::detect() else { return Vec::new() };
    let installs = installs(ctx, target);
    match target.emulator {
        TrophyEmulator::Rpcs3 => rpcs3::watched_files(&env, &installs, target),
        TrophyEmulator::ShadPs4 => shadps4::watched_files(&env, &installs, target),
        TrophyEmulator::Xenia => xenia::watched_files(&env, &installs, target),
    }
}

/// A file's bytes, when it is a file of at most `max` bytes.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = file.metadata().map_err(|e| format!("{}: {e}", path.display()))?.len();
    if len > max {
        return Err(format!("{}: {len} bytes, more than a trophy file has", path.display()));
    }
    let mut bytes = Vec::new();
    // The file may grow while it is read: never past the limit.
    file.take(max).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(bytes)
}

/// The entries of a folder (at most [`MAX_ENTRIES`]); none when it cannot
/// be read.
pub(crate) fn entries(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().take(MAX_ENTRIES).map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    }
}

pub(crate) fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok())
}

/// The file written last of `files` (the existing ones).
pub(crate) fn newest(files: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    files.into_iter().filter_map(|f| modified(&f).map(|t| (t, f))).max_by_key(|(t, _)| *t).map(|(_, f)| f)
}

/// The folders a game's files are in: each ROM's (or the ROM itself when it
/// is a folder) and up to `up` of their parents, nearest first, no repeats.
pub(crate) fn game_dirs(roms: &[PathBuf], up: usize) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for rom in roms {
        let start = if rom.is_dir() { Some(rom.as_path()) } else { rom.parent() };
        for dir in start.into_iter().flat_map(Path::ancestors).take(up + 1) {
            if dir.as_os_str().is_empty() || dir.parent().is_none() {
                break;
            }
            if !out.iter().any(|d| d == dir) {
                out.push(dir.to_path_buf());
            }
        }
    }
    out
}

/// A title as compared: letters and digits only, lowercased (`Gran
/// Turismo® 5` and `GRAN TURISMO 5` are one).
pub(crate) fn same_title(a: &str, b: &str) -> bool {
    let fold = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let (a, b) = (fold(a), fold(b));
    !a.is_empty() && a == b
}

/// An unlock time when it is plausible: after the consoles came out (2005)
/// and not in the future (a day of slack for clocks). Anything else is a
/// time the file got wrong, and the unlock is kept without one.
pub(crate) fn plausible(at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let first = DateTime::from_timestamp(1_104_537_600, 0)?;
    (at >= first && at <= Utc::now() + chrono::Duration::days(1)).then_some(at)
}

/// A PlayStation tick (`CellRtcTick`: microseconds since 0001-01-01 UTC);
/// `0` is no time.
pub(crate) fn from_rtc_tick(tick: u64) -> Option<DateTime<Utc>> {
    const UNIX_EPOCH_TICK: u64 = 62_135_596_800_000_000;
    let micros = i64::try_from(tick.checked_sub(UNIX_EPOCH_TICK)?).ok()?;
    plausible(DateTime::from_timestamp_micros(micros)?)
}

/// Copies of `bytes` with a few bytes overwritten at random (a fixed
/// seed): damaged files for the parsers' tests.
#[cfg(test)]
pub(crate) fn mutants(bytes: &[u8], count: usize) -> Vec<Vec<u8>> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..count)
        .map(|_| {
            let mut b = bytes.to_vec();
            for _ in 0..1 + next() % 4 {
                if b.is_empty() {
                    break;
                }
                let at = (next() % b.len() as u64) as usize;
                b[at] = match next() % 3 {
                    0 => 0xFF,
                    1 => 0,
                    _ => next() as u8,
                };
            }
            b
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_compare_without_marks_and_case() {
        assert!(same_title("Gran Turismo® 5", "GRAN TURISMO 5"));
        assert!(same_title("Demon's Souls", "Demons Souls"));
        assert!(!same_title("Gran Turismo 5", "Gran Turismo 6"));
        assert!(!same_title("™", "®"), "nothing left is no match");
    }

    #[test]
    fn rtc_ticks_are_microseconds_since_year_one() {
        // 2023-11-14 22:13:20 UTC.
        let tick = 62_135_596_800_000_000 + 1_700_000_000_000_000;
        assert_eq!(from_rtc_tick(tick).unwrap().timestamp(), 1_700_000_000);
        assert_eq!(from_rtc_tick(0), None);
        assert_eq!(from_rtc_tick(5), None);
        assert_eq!(from_rtc_tick(u64::MAX), None, "far in the future");
    }

    #[test]
    fn game_dirs_walk_up_from_each_rom() {
        let d = tempfile::tempdir().unwrap();
        let usrdir = d.path().join("Game/PS3_GAME/USRDIR");
        std::fs::create_dir_all(&usrdir).unwrap();
        let eboot = usrdir.join("EBOOT.BIN");
        std::fs::write(&eboot, b"x").unwrap();
        let dirs = game_dirs(&[eboot.clone(), eboot], 2);
        assert_eq!(dirs, [usrdir.clone(), d.path().join("Game/PS3_GAME"), d.path().join("Game")]);
        // A folder is a start of its own.
        assert_eq!(game_dirs(std::slice::from_ref(&usrdir), 0), [usrdir]);
    }

    #[test]
    fn a_file_over_the_limit_is_not_read() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("big");
        std::fs::write(&f, vec![0u8; 100]).unwrap();
        assert!(read_capped(&f, 99).is_err());
        assert_eq!(read_capped(&f, 100).unwrap().len(), 100);
        assert!(read_capped(&d.path().join("none"), 100).is_err());
    }
}
