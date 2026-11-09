//! Small readers the clients share: text files in either encoding, the
//! prefix's registry file and the Windows paths the registry holds.

use std::path::{Path, PathBuf};

use heronite_data::clients::wine_prefix;

use crate::reg::{self, RegKey};
use crate::winpath;

/// The Uninstall keys of a 64-bit prefix: native programs and 32-bit ones.
pub(crate) const UNINSTALL: [&str; 2] =
    ["Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall", "Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"];

/// Text of a file the clients write: UTF-8 or UTF-16 (with a byte order
/// mark, or UTF-16LE without one), whatever is not valid replaced.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| unit(*c)).collect();
        String::from_utf16_lossy(&units)
    }
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        // ASCII text in UTF-16LE has a zero after each letter.
        [a, 0, ..] if *a != 0 => utf16(bytes, u16::from_le_bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub(crate) fn read_text(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| decode_text(&b))
}

/// The keys of the prefix's `system.reg` (HKLM) under `under`.
pub(crate) fn machine_keys(prefix: &Path, under: &[&str]) -> Vec<RegKey> {
    let file = wine_prefix(prefix).join("system.reg");
    reg::read_keys(&file, under).unwrap_or_else(|e| {
        log::debug!("{}: {e}", file.display());
        Vec::new()
    })
}

/// The Linux folder of a Windows path of the registry.
pub(crate) fn unix_path(prefix: &Path, win: &str) -> Option<PathBuf> {
    winpath::to_unix(&wine_prefix(prefix), win)
}

/// The last part of a Windows or Unix path (`C:/Games/Far Cry 3/` is
/// `Far Cry 3`).
pub(crate) fn last_part(path: &str) -> Option<&str> {
    path.rsplit(['\\', '/']).find(|p| !p.trim().is_empty()).map(str::trim)
}

/// The subfolders of `dir`, sorted; none when it cannot be read.
pub(crate) fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_utf8_and_both_utf16_orders() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFabc"), "abc");
        assert_eq!(decode_text("plain é".as_bytes()), "plain é");
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("a<é>".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode_text(&le), "a<é>");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("a<é>".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_text(&be), "a<é>");
        let bare: Vec<u8> = "<x/>".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_text(&bare), "<x/>");
    }

    #[test]
    fn names_the_last_part_of_a_path() {
        assert_eq!(last_part("C:/Program Files (x86)/Ubisoft/games/Far Cry 3/"), Some("Far Cry 3"));
        assert_eq!(last_part("C:\\Games\\Hades"), Some("Hades"));
        assert_eq!(last_part(""), None);
    }
}
