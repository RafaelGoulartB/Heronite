//! Size on disk of a game whose store gives none (Steam, emulated, added
//! manually, imported installs), for the Details panel. Steam's own number
//! comes from its manifest (`heronite_launch::steam`); emulated games count
//! their ROM files; the rest walk the game folder. Walks are bounded, can be
//! stopped, and their result is cached (`install-sizes.json` in the cache
//! folder) so a game is not walked every time it gets the focus.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::json_store;

/// A walk that meets more entries than this gives up: the folder is not one
/// game (a shared folder, a whole drive).
const MAX_ENTRIES: usize = 300_000;

/// How long a walked size is trusted when the folder itself did not change.
const CACHE_SECS: u64 = 3 * 24 * 3600;

/// Folders never walked: the system ones and the home folder or above it.
const SYSTEM_DIRS: [&str; 10] = ["/usr", "/usr/bin", "/usr/local", "/usr/local/bin", "/usr/games", "/bin", "/opt", "/tmp", "/var", "/etc"];

/// Whether `dir` may be walked as one game's folder. On Windows, nothing in
/// the Windows folder is.
pub fn walkable(dir: &Path, home: Option<&Path>) -> bool {
    if !dir.is_absolute() || dir.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    #[cfg(windows)]
    if std::env::var_os("SystemRoot").is_some_and(|w| !w.is_empty() && dir.starts_with(&w)) {
        return false;
    }
    let depth = dir.components().filter(|c| matches!(c, Component::Normal(_))).count();
    depth >= 2 && !SYSTEM_DIRS.iter().any(|s| dir == Path::new(s)) && !home.is_some_and(|h| h.starts_with(dir))
}

/// Bytes of the regular files below `dir` (links are not followed). `None`
/// when `stop` asks (checked every few hundred entries) or the folder holds
/// more than [`MAX_ENTRIES`] entries.
pub fn folder_bytes(dir: &Path, stop: &dyn Fn() -> bool) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = fs::read_dir(&next) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES || (seen.is_multiple_of(256) && stop()) {
                return None;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (!stop()).then_some(total)
}

/// Bytes of a game's ROM files, with the files a `.cue`, `.gdi` or `.m3u`
/// names (tracks, discs) counted once each.
pub fn rom_bytes(roms: &[PathBuf]) -> u64 {
    let mut files: Vec<PathBuf> = Vec::new();
    for rom in roms {
        files.push(rom.clone());
        files.extend(referenced_files(rom));
    }
    files.sort();
    files.dedup();
    files.iter().filter_map(|f| fs::metadata(f).ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()
}

/// The files a disc sheet or playlist names, next to it.
fn referenced_files(rom: &Path) -> Vec<PathBuf> {
    let ext = rom.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !matches!(ext.as_str(), "cue" | "gdi" | "m3u") {
        return Vec::new();
    }
    let Some(dir) = rom.parent() else { return Vec::new() };
    // Sheets are small text; a big file with this extension is not one.
    if fs::metadata(rom).map(|m| m.len() > 1 << 20).unwrap_or(true) {
        return Vec::new();
    }
    let Ok(bytes) = fs::read(rom) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let name = match ext.as_str() {
            // FILE "Track 01.bin" BINARY
            "cue" => line.strip_prefix("FILE ").and_then(|rest| quoted_or_first(rest)),
            // 1 0 4 2352 "track01.raw" 0
            "gdi" => line.split_once(' ').and_then(|_| quoted_or_nth(line, 4)),
            _ => Some(line),
        };
        let Some(name) = name.filter(|n| !n.is_empty()) else { continue };
        let path = dir.join(name);
        // A playlist names sheets: count their tracks too.
        if ext == "m3u" {
            out.extend(referenced_files(&path));
        }
        out.push(path);
    }
    out
}

fn quoted_or_first(text: &str) -> Option<&str> {
    let text = text.trim();
    match text.strip_prefix('"') {
        Some(rest) => rest.split_once('"').map(|(name, _)| name),
        None => text.split_whitespace().next(),
    }
}

/// The `n`th field (0-based) of a `.gdi` line, which may be quoted.
fn quoted_or_nth(line: &str, n: usize) -> Option<&str> {
    if let Some(start) = line.find('"') {
        return line[start + 1..].split_once('"').map(|(name, _)| name);
    }
    line.split_whitespace().nth(n)
}

/// A size as stores write it: bytes ("4518092416"), or a number and a unit
/// ("41.92 GB", "1.5 GiB", "700 MB"), 1024-based.
pub fn parse_store_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(text.len());
    let number: f64 = text[..split].replace(',', "").parse().ok()?;
    let unit = text[split..].trim().to_ascii_lowercase();
    let power = match unit.trim_end_matches("ib").trim_end_matches('b') {
        "" => 0,
        "k" => 1,
        "m" => 2,
        "g" => 3,
        "t" => 4,
        _ => return None,
    };
    let bytes = number * 1024f64.powi(power);
    (bytes.is_finite() && bytes >= 1.).then_some(bytes as u64)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedSize {
    bytes: u64,
    /// Seconds since the epoch.
    checked_at: u64,
    /// Modification time of the folder when it was walked (seconds).
    folder_mtime: u64,
}

/// Walked sizes by folder, in `install-sizes.json` of the app's cache folder.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SizeCache {
    #[serde(default)]
    folders: HashMap<String, CachedSize>,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn mtime(dir: &Path) -> u64 {
    fs::metadata(dir).and_then(|m| m.modified()).map(secs).unwrap_or(0)
}

impl SizeCache {
    pub fn file(cache_dir: &Path) -> PathBuf {
        cache_dir.join("install-sizes.json")
    }

    pub fn load(cache_dir: &Path) -> Self {
        json_store::read(&Self::file(cache_dir)).unwrap_or_default()
    }

    /// The size walked for `dir`, while it is recent and the folder did not change.
    pub fn get(&self, dir: &Path) -> Option<u64> {
        let hit = self.folders.get(dir.to_string_lossy().as_ref())?;
        let fresh = secs(SystemTime::now()).saturating_sub(hit.checked_at) < CACHE_SECS;
        (fresh && hit.folder_mtime == mtime(dir)).then_some(hit.bytes)
    }

    /// Stores the size of `dir` (read-modify-write, so concurrent saves keep each other's entries).
    pub fn put(cache_dir: &Path, dir: &Path, bytes: u64) -> anyhow::Result<()> {
        let entry = CachedSize { bytes, checked_at: secs(SystemTime::now()), folder_mtime: mtime(dir) };
        json_store::update::<SizeCache, _>(&Self::file(cache_dir), |c| {
            c.folders.insert(dir.to_string_lossy().into_owned(), entry);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_in_store_shapes() {
        assert_eq!(parse_store_size("4518092416"), Some(4_518_092_416));
        assert_eq!(parse_store_size("1.5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_store_size("41.92 GB"), Some(45_011_257_262));
        assert_eq!(parse_store_size("700 MB"), Some(734_003_200));
        assert_eq!(parse_store_size("0"), None);
        assert_eq!(parse_store_size("big"), None);
    }

    #[test]
    fn walks_only_game_folders() {
        // A full path on Windows starts with a drive.
        let p = |s: &str| if cfg!(windows) { PathBuf::from(format!("C:{s}")) } else { PathBuf::from(s) };
        let home = &p("/home/me");
        assert!(walkable(&p("/home/me/Games/Celeste"), Some(home)));
        assert!(walkable(&p("/data/Games"), Some(home)));
        assert!(!walkable(&p("/home/me"), Some(home)), "home");
        assert!(!walkable(&p("/home"), Some(home)), "above home");
        #[cfg(unix)]
        assert!(!walkable(Path::new("/usr/bin"), Some(home)));
        #[cfg(windows)]
        assert!(!walkable(&Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32"), Some(home)));
        assert!(!walkable(&p("/data"), Some(home)), "a drive's root");
        assert!(!walkable(Path::new("games/x"), Some(home)), "relative");
    }

    #[test]
    fn folder_sizes_add_up_and_stop() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("x.bin"), vec![0u8; 100]).unwrap();
        fs::write(dir.path().join("a/b/y.bin"), vec![0u8; 23]).unwrap();
        assert_eq!(folder_bytes(dir.path(), &|| false), Some(123));
        assert_eq!(folder_bytes(dir.path(), &|| true), None);
    }

    #[test]
    fn roms_count_their_tracks_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::write(
            p.join("Game (Disc 1).cue"),
            "FILE \"Game (Disc 1) (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\nFILE \"Game (Disc 1) (Track 2).bin\" BINARY\n",
        )
        .unwrap();
        fs::write(p.join("Game (Disc 1) (Track 1).bin"), vec![0u8; 1000]).unwrap();
        fs::write(p.join("Game (Disc 1) (Track 2).bin"), vec![0u8; 200]).unwrap();
        fs::write(p.join("Game (Disc 2).cue"), "FILE \"d2.bin\" BINARY\n").unwrap();
        fs::write(p.join("d2.bin"), vec![0u8; 50]).unwrap();
        fs::write(p.join("Game.m3u"), "Game (Disc 1).cue\nGame (Disc 2).cue\n").unwrap();
        let sheets = fs::metadata(p.join("Game (Disc 1).cue")).unwrap().len()
            + fs::metadata(p.join("Game (Disc 2).cue")).unwrap().len()
            + fs::metadata(p.join("Game.m3u")).unwrap().len();
        // The playlist and a disc listed twice still count each file once.
        let roms = [p.join("Game.m3u"), p.join("Game (Disc 1).cue")];
        assert_eq!(rom_bytes(&roms), 1250 + sheets);

        fs::write(p.join("g.gdi"), "2\n1 0 4 2352 track01.bin 0\n2 600 0 2352 \"track 02.raw\" 0\n").unwrap();
        fs::write(p.join("track01.bin"), vec![0u8; 7]).unwrap();
        fs::write(p.join("track 02.raw"), vec![0u8; 3]).unwrap();
        let gdi = fs::metadata(p.join("g.gdi")).unwrap().len();
        assert_eq!(rom_bytes(&[p.join("g.gdi")]), 10 + gdi);
    }

    #[test]
    fn cache_trusts_an_unchanged_folder() {
        let cache = tempfile::tempdir().unwrap();
        let game = tempfile::tempdir().unwrap();
        SizeCache::put(cache.path(), game.path(), 42).unwrap();
        assert_eq!(SizeCache::load(cache.path()).get(game.path()), Some(42));
        assert_eq!(SizeCache::load(cache.path()).get(Path::new("/nowhere")), None);
    }
}
