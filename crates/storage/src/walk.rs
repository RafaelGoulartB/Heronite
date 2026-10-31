//! The space a folder takes, as `du` counts it: allocated blocks (a sparse
//! or compressed file takes less than its length, a small file a whole
//! block), each hard-linked file once, links not followed (on Windows
//! neither symlinks nor junctions nor mounted folders: each folder is read
//! in batches with the sizes, `heronite_platform::disks::sized_entries`).

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use heronite_platform::disks::{allocated_size, sized_entries};
use heronite_platform::fs::allocated_bytes;

/// Bytes `path` (a file or a folder) takes. `None` when `stop` asks (checked
/// every few hundred entries) or `path` is missing.
pub fn disk_usage(path: &Path, stop: &dyn Fn() -> bool) -> Option<u64> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return Some(allocated_size(path, &meta));
    }
    let mut total = allocated_bytes(&meta);
    let mut linked: HashSet<(u64, u64)> = HashSet::new();
    let mut seen = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = sized_entries(&dir) else { continue };
        for entry in entries {
            seen += 1;
            if seen.is_multiple_of(256) && stop() {
                return None;
            }
            if entry.dir {
                stack.push(entry.path);
            } else if entry.link_id.is_some_and(|id| !linked.insert(id)) {
                continue;
            }
            total += entry.bytes;
        }
    }
    (!stop()).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hard links are told apart only on Unix (`hard_link_id`).
    #[cfg(unix)]
    #[test]
    fn counts_blocks_once_per_file_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::create_dir_all(p.join("a/b")).unwrap();
        fs::write(p.join("a/b/big.bin"), vec![1u8; 64 * 1024]).unwrap();
        fs::hard_link(p.join("a/b/big.bin"), p.join("a/twin.bin")).unwrap();
        std::os::unix::fs::symlink("/usr", p.join("a/usr")).unwrap();
        let used = disk_usage(p, &|| false).unwrap();
        // The file once (not twice), the link not followed into /usr.
        assert!((64 * 1024..64 * 1024 + 64 * 1024).contains(&used), "{used}");
        assert_eq!(disk_usage(&p.join("missing"), &|| false), None);
        assert_eq!(disk_usage(p, &|| true), None);
    }

    /// Whole clusters per file, the files of every folder, a junction (no
    /// administrator needed) to a big folder outside not walked into.
    #[cfg(windows)]
    #[test]
    fn counts_clusters_and_leaves_junctions_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (game, outside) = (dir.path().join("game"), dir.path().join("outside"));
        fs::create_dir_all(game.join("data/maps")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(game.join("game.exe"), vec![1u8; 10]).unwrap();
        fs::write(game.join("data/maps/big.bin"), vec![1u8; 300_000]).unwrap();
        fs::write(outside.join("huge.bin"), vec![1u8; 4_000_000]).unwrap();
        let out = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(game.join("data").join("linked"))
            .arg(&outside)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let used = disk_usage(&game, &|| false).unwrap();
        // 10 bytes take a cluster (or live in the MFT record: 0), the big file
        // its length rounded up; nothing of the 4 MB behind the junction.
        assert!((300_000..300_000 + 2 * 65_536).contains(&used), "{used}");
        assert!(disk_usage(&game.join("data/maps/big.bin"), &|| false).unwrap() >= 300_000);
        // The junction itself is a link: none of what it points to.
        assert_eq!(disk_usage(&game.join("data").join("linked"), &|| false), Some(0));
        assert_eq!(disk_usage(&game.join("missing"), &|| false), None);
        assert_eq!(disk_usage(&game, &|| true), None);
        assert!(outside.join("huge.bin").exists());
    }
}
