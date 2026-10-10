//! Finding the work ([`pending`]) and measuring the folder ([`usage`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{png, webp, SETTLE};
use crate::list::list_game_folders;

/// How deep [`usage`] walks below the captures root.
const USAGE_DEPTH: u32 = 8;

/// PNGs that can become lossless WebPs, newest first: those of each game
/// folder and of its sub-folders one level down (what the gallery lists),
/// skipping hidden names, files changed in the last [`SETTLE`] and files
/// already kept as PNG. Also sweeps the temp files of a killed run.
pub fn pending(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    let now = SystemTime::now();
    for game in list_game_folders(root) {
        let dir = root.join(&game);
        collect(&dir, now, &mut found);
        for sub in list_game_folders(&dir) {
            collect(&dir.join(sub), now, &mut found);
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

fn collect(dir: &Path, now: SystemTime, out: &mut Vec<(SystemTime, PathBuf)>) {
    heronite_data::temps::sweep_dir(dir);
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !is_png(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(mtime) = meta.modified() else { continue };
        let settled = now.duration_since(mtime).is_ok_and(|age| age >= SETTLE);
        if !meta.is_file() || !settled {
            continue;
        }
        let path = entry.path();
        if matches!(png::probe(&path), Ok(png::Probe::Convertible)) {
            out.push((mtime, path));
        }
    }
}

fn is_png(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

/// Space used by the captures folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderUsage {
    /// Every file below the root (apparent sizes).
    pub total_bytes: u64,
    pub files: u64,
    /// PNG files (converted or not).
    pub png_files: u64,
    pub png_bytes: u64,
    /// WebPs with our marker, and how much smaller they are than their PNGs.
    pub compressed_files: u64,
    pub saved_bytes: u64,
}

/// Walks the whole folder (symlinks are not followed) and reads the marker of
/// each WebP (a few small reads per file).
pub fn usage(root: &Path) -> FolderUsage {
    let mut u = FolderUsage::default();
    walk(root, 0, &mut u);
    u
}

fn walk(dir: &Path, depth: u32, u: &mut FolderUsage) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let path = entry.path();
        if meta.is_dir() {
            if depth < USAGE_DEPTH {
                walk(&path, depth + 1, u);
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        u.files += 1;
        u.total_bytes += meta.len();
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        match ext.as_deref() {
            Some("png") => {
                u.png_files += 1;
                u.png_bytes += meta.len();
            }
            Some("webp") => {
                if let Ok(Some(m)) = webp::read_marker(&path) {
                    u.compressed_files += 1;
                    u.saved_bytes += m.source_size.saturating_sub(meta.len());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compress::png::tests::{noisy_rgb, write_png};
    use crate::compress::{compress_file, Outcome};
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    fn png_at(path: &Path, age_s: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_png(path, 128, 96, ::png::ColorType::Rgb, &noisy_rgb(128, 96), &[]);
        let t = SystemTime::now() - Duration::from_secs(age_s);
        fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn pending_lists_settled_pngs_of_game_folders_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        png_at(&root.join("Elden Ring/old.png"), 7200);
        png_at(&root.join("Elden Ring/new.PNG"), 600);
        png_at(&root.join("Elden Ring/2024/nested.png"), 3600);
        png_at(&root.join("Elden Ring/fresh.png"), 0);
        png_at(&root.join("Elden Ring/.hidden.png"), 7200);
        png_at(&root.join("Elden Ring/a/b/too-deep.png"), 7200);
        png_at(&root.join(".cache/x.png"), 7200);
        png_at(&root.join("loose.png"), 7200);
        fs::write(root.join("Elden Ring/notes.txt"), "x").unwrap();
        let names: Vec<String> =
            pending(root).iter().map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Elden Ring/new.PNG", "Elden Ring/2024/nested.png", "Elden Ring/old.png"]);
    }

    #[test]
    fn usage_counts_everything_and_the_space_freed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        png_at(&root.join("Game/a.png"), 7200);
        png_at(&root.join("Game/b.png"), 7200);
        fs::write(root.join("Game/other.webp"), b"RIFF....WEBPVP8L").unwrap();
        let before = usage(root);
        assert_eq!((before.files, before.png_files, before.compressed_files), (3, 2, 0));
        let a_size = fs::metadata(root.join("Game/a.png")).unwrap().len();
        let out = compress_file(&root.join("Game/a.png"), &AtomicBool::new(false)).unwrap();
        let Outcome::Converted { saved, .. } = out else { panic!("{out:?}") };
        let after = usage(root);
        assert_eq!((after.files, after.png_files, after.compressed_files), (3, 1, 1));
        assert_eq!(after.saved_bytes as i64, saved);
        assert_eq!(after.total_bytes + saved as u64, before.total_bytes);
        assert!(a_size > saved as u64);
        assert!(pending(root).iter().all(|p| !p.ends_with("a.png")));
    }
}
