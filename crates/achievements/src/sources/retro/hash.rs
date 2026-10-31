//! RetroAchievements' ROM hashes, computed by the vendored rcheevos
//! (`rc_hash`): the same code the emulators' RA integrations run, so a hash
//! here is the hash the site knows (iNES headers skipped, discs read through
//! their file systems, and so on).

use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;

extern "C" {
    fn heronite_rc_hash_file(out: *mut c_char, console_id: u32, path: *const c_char) -> c_int;
    fn heronite_rc_hash_candidates(path: *const c_char, consoles: *mut u8, hashes: *mut [c_char; 33], max: c_int) -> c_int;
}

/// The most hashes the extension of one file can give (`consoles` of rcheevos' iterator).
const MAX_CANDIDATES: usize = 12;

/// A hash and the RA console it was computed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub console: u32,
    pub hash: String,
}

fn c_path(path: &Path) -> Option<CString> {
    heronite_platform::fs::c_path(path)
}

fn hash_text(raw: &[c_char; 33]) -> String {
    // SAFETY: rcheevos writes a NUL-terminated string of at most 32 characters.
    unsafe { CStr::from_ptr(raw.as_ptr()) }.to_string_lossy().into_owned()
}

/// The hash of `path` for RA console `console`; `None` when the file cannot
/// be read or is not valid for that console.
pub fn hash_file(console: u32, path: &Path) -> Option<String> {
    let path = c_path(path)?;
    let mut out = [0 as c_char; 33];
    // SAFETY: `path` is NUL-terminated and `out` has the 33 bytes rcheevos writes.
    let ok = unsafe { heronite_rc_hash_file(out.as_mut_ptr(), console, path.as_ptr()) };
    (ok != 0).then(|| hash_text(&out))
}

/// Every hash the extension of `path` allows, in rcheevos' order of likelihood
/// (`rc_hash_iterate`); for when the console is unknown.
pub fn candidates(path: &Path) -> Vec<Candidate> {
    let Some(path) = c_path(path) else { return Vec::new() };
    let mut consoles = [0u8; MAX_CANDIDATES];
    let mut hashes = [[0 as c_char; 33]; MAX_CANDIDATES];
    // SAFETY: both arrays hold `MAX_CANDIDATES` entries, the `max` passed.
    let n = unsafe { heronite_rc_hash_candidates(path.as_ptr(), consoles.as_mut_ptr(), hashes.as_mut_ptr(), MAX_CANDIDATES as c_int) };
    (0..n.clamp(0, MAX_CANDIDATES as c_int) as usize)
        .map(|i| Candidate { console: consoles[i] as u32, hash: hash_text(&hashes[i]) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MD5 of "abc".
    const ABC: &str = "900150983cd24fb0d6963f7d28e17f72";

    #[test]
    fn game_boy_hashes_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.gb");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(hash_file(4, &file).unwrap(), ABC);
    }

    #[test]
    fn nes_skips_the_ines_header() {
        let dir = tempfile::tempdir().unwrap();
        let mut rom = b"NES\x1a\x01\x01\0\0\0\0\0\0\0\0\0\0".to_vec();
        rom.extend_from_slice(b"abc");
        let file = dir.path().join("game.nes");
        std::fs::write(&file, &rom).unwrap();
        assert_eq!(hash_file(7, &file).unwrap(), ABC);
        // The extension alone gives the console.
        assert_eq!(candidates(&file), [Candidate { console: 7, hash: ABC.into() }]);
    }

    #[test]
    fn missing_and_empty_files_give_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(hash_file(7, &dir.path().join("none.nes")), None);
        assert!(candidates(&dir.path().join("none.nes")).is_empty());
    }
}
