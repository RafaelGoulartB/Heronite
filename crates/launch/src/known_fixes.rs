//! Keeping `known_fixes.json` up to date (port of `known_fixes/utils.ts`):
//! Heroic compares the MD5 of the local file with `knownFixesSha` of its
//! releases-info document and downloads the latest file from the
//! `known-fixes` repository when they differ. The file is shared with
//! Heroic/Heronite and has the same format.

use std::fs;
use std::path::Path;

use heronite_data::HeroicPaths;
use serde_json::Value;

use crate::net::{Cancel, Net};

pub const RELEASES_INFO_URL: &str =
    "https://raw.githubusercontent.com/Heroic-Games-Launcher/releases-info/refs/heads/main/release-data.json";
pub const KNOWN_FIXES_URL: &str =
    "https://raw.githubusercontent.com/Heroic-Games-Launcher/known-fixes/main/known_fixes.json";

/// MD5 of `data` as lowercase hex (what `createMD5` gives; only used to compare
/// with the published hash, not for security).
pub fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
        14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
        21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    [a0, b0, c0, d0].iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

/// `createMD5(path)`: empty when the file does not exist.
pub fn file_md5(path: &Path) -> String {
    fs::read(path).map(|d| md5_hex(&d)).unwrap_or_default()
}

/// `downloadFileIfNeeded`: downloads the latest `known_fixes.json` unless
/// the local file already has the published hash. `Ok(true)` when it was
/// replaced.
pub fn update_if_needed(paths: &HeroicPaths, net: &dyn Net, cancel: Cancel) -> Result<bool, String> {
    if !net.is_online() {
        return Ok(false);
    }
    let release = net.get_json(RELEASES_INFO_URL)?;
    let latest = release.get("knownFixesSha").and_then(Value::as_str).unwrap_or_default();
    let target = paths.root.join("known_fixes.json");
    if !latest.is_empty() && file_md5(&target) == latest {
        log::debug!("Known Fixes data did not change. Skipping.");
        return Ok(false);
    }
    log::debug!("Known Fixes data changed. Downloading latest file.");
    let part = paths.root.join("known_fixes.json.part");
    net.download(KNOWN_FIXES_URL, &part, cancel).map_err(|e| {
        let _ = fs::remove_file(&part);
        format!("Failed download of Known Fixes data: {e}")
    })?;
    // A broken download must not replace a good file.
    let valid = fs::read_to_string(&part)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|v| v.is_object());
    if !valid {
        let _ = fs::remove_file(&part);
        return Err("Failed download of Known Fixes data: the file is not valid JSON".into());
    }
    fs::rename(&part, &target).map_err(|e| format!("installing known_fixes.json: {e}"))?;
    log::info!("Known Fixes data downloaded");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use serde_json::json;

    #[test]
    fn md5_matches_the_reference_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5_hex(b"The quick brown fox jumps over the lazy dog"), "9e107d9d372bb6826bd81d3542a419d6");
        // More than one block, and a length around the padding boundary.
        let long = "a".repeat(1000);
        assert_eq!(md5_hex(long.as_bytes()), "cabe45dcc9ae5b66ba86600cca6b8ba8");
        assert_eq!(md5_hex("b".repeat(56).as_bytes()).len(), 32);
    }

    fn fixture() -> (tempfile::TempDir, HeroicPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path().join("heroic"));
        fs::create_dir_all(&paths.root).unwrap();
        (dir, paths)
    }

    #[test]
    fn the_file_is_downloaded_when_the_hash_differs() {
        let (dir, paths) = fixture();
        let body = r#"{"epic": {"A": {"winetricks": ["vcrun2022"]}}}"#;
        let src = dir.path().join("new.json");
        fs::write(&src, body).unwrap();
        let net = FakeNet::online()
            .with_json(RELEASES_INFO_URL, json!({"knownFixesSha": md5_hex(body.as_bytes())}))
            .with_file(KNOWN_FIXES_URL, fs::read(&src).unwrap());
        assert!(update_if_needed(&paths, &net, &|| false).unwrap());
        assert_eq!(fs::read_to_string(paths.root.join("known_fixes.json")).unwrap(), body);
        // Same hash now: no download.
        let again = FakeNet::online().with_json(RELEASES_INFO_URL, json!({"knownFixesSha": md5_hex(body.as_bytes())}));
        assert!(!update_if_needed(&paths, &again, &|| false).unwrap());
        assert_eq!(again.requests().len(), 1);
    }

    #[test]
    fn offline_or_broken_downloads_keep_the_current_file() {
        let (_dir, paths) = fixture();
        let file = paths.root.join("known_fixes.json");
        fs::write(&file, r#"{"epic":{}}"#).unwrap();
        assert!(!update_if_needed(&paths, &FakeNet::offline(), &|| false).unwrap());
        let net = FakeNet::online()
            .with_json(RELEASES_INFO_URL, json!({"knownFixesSha": "other"}))
            .with_file(KNOWN_FIXES_URL, b"<html>nope".to_vec());
        assert!(update_if_needed(&paths, &net, &|| false).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), r#"{"epic":{}}"#);
        assert!(!paths.root.join("known_fixes.json.part").exists());
    }
}
