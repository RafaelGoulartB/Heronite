//! The space a folder takes, as `du` counts it: allocated blocks (a sparse
//! or compressed file takes less than its length, a small file a whole
//! block), each hard-linked file once, links not followed.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use heronite_platform::fs::{allocated_bytes, hard_link_id};

/// Bytes `path` (a file or a folder) takes. `None` when `stop` asks (checked
/// every few hundred entries) or `path` is missing.
pub fn disk_usage(path: &Path, stop: &dyn Fn() -> bool) -> Option<u64> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return Some(allocated(&meta));
    }
    let mut total = allocated(&meta);
    let mut linked: HashSet<(u64, u64)> = HashSet::new();
    let mut seen = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen.is_multiple_of(256) && stop() {
                return None;
            }
            // Not followed: a link takes its own few bytes.
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if hard_link_id(&meta).is_some_and(|id| !linked.insert(id)) {
                continue;
            }
            total += allocated(&meta);
        }
    }
    (!stop()).then_some(total)
}

fn allocated(meta: &fs::Metadata) -> u64 {
    allocated_bytes(meta)
}

// Hard links are told apart only on Unix (`hard_link_id`), and Windows makes
// symlinks only in Developer Mode or as an administrator.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

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
}
