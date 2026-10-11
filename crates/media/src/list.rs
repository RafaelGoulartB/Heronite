//! Listing of a game's screenshots. Port of `getCollectionScreenshots` in
//! `src/local-library/screenshots/index.ts`.

use crate::matching::{pick_screenshot_folder, MIN_SCORE};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Lower-case extensions (without the dot) treated as images.
pub const IMAGE_EXTENSIONS: [&str; 9] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "avif", "jxl", "jxr"];

/// What identifies a game for folder matching (the UI passes the Collection
/// title, the Steam app id and `[gameInfo.title, gameInfo.folder_name]`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameQuery {
    pub title: String,
    pub steam_app_id: Option<String>,
    pub aliases: Vec<String>,
}

impl GameQuery {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), ..Default::default() }
    }

    /// Same query the Collection focus panel builds for a game: the Collection
    /// title, `meta.steamAppId`, and the store title / install folder name
    /// when they differ from the Collection title.
    pub fn from_game(game: &heronite_data::CollectionGame) -> Self {
        let mut aliases = Vec::new();
        for alias in [Some(game.game.title.as_str()), game.game.folder_name.as_deref()].into_iter().flatten() {
            if !alias.is_empty() && alias != game.title {
                aliases.push(alias.to_owned());
            }
        }
        Self { title: game.title.clone(), steam_app_id: game.steam_app_id.clone(), aliases }
    }

    /// `uniqueQueries`: trimmed, at least 2 characters, de-duplicated, title first.
    pub fn queries(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for item in std::iter::once(&self.title).chain(self.aliases.iter()) {
            let item = item.trim();
            if item.encode_utf16().count() >= 2 && !out.iter().any(|q| q == item) {
                out.push(item.to_owned());
            }
        }
        out
    }
}

/// One screenshot file.
#[derive(Debug, Clone, PartialEq)]
pub struct Screenshot {
    /// Path relative to the captures root (`Game/shot.png`), stable across runs.
    pub id: String,
    /// Absolute path of the file.
    pub path: PathBuf,
    /// File name without directories.
    pub name: String,
    /// Modification time in milliseconds since the Unix epoch (JS `mtimeMs`);
    /// `None` when unknown.
    pub taken_at: Option<f64>,
}

/// Result of [`list_screenshots`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScreenshotsResult {
    /// The captures root (when configured).
    pub folder: Option<PathBuf>,
    /// Name of the matched sub-folder, if any game folder matched.
    pub matched_folder: Option<String>,
    /// Oldest first (`takenAt` ascending).
    pub items: Vec<Screenshot>,
}

/// Lexical `path.resolve`: absolute, `.`/`..` collapsed, no symlink resolution.
pub fn resolve_path(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// Names of the game folders under `root`: real sub-directories (symlinks are
/// not followed, like `Dirent.isDirectory`) that do not start with `.`, in
/// byte order (what Node's `readdirSync` returns on Linux).
pub fn list_game_folders(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort_unstable();
    out
}

/// True for names with a supported image extension (case-insensitive).
pub fn is_image_file(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            IMAGE_EXTENSIONS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

fn mtime_ms(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

struct Found {
    /// Path relative to the game folder.
    rel: String,
    taken_at: f64,
}

fn sort_found(files: &mut [Found]) {
    // Stable sort by time; ties by name so the result is deterministic.
    files.sort_by(|a, b| a.taken_at.total_cmp(&b.taken_at).then_with(|| a.rel.cmp(&b.rel)));
}

/// `listImages`: image files of `folder`; when it has none directly, those of
/// its sub-folders (one level only).
fn list_images(folder: &Path, depth: u32) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut entries: Vec<(String, bool)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some((name, dir))
        })
        .collect();
    entries.sort_unstable();

    let mut files: Vec<Found> = entries
        .iter()
        .filter(|(name, dir)| !dir && is_image_file(name) && !name.starts_with('.'))
        .map(|(name, _)| Found { rel: name.clone(), taken_at: mtime_ms(&folder.join(name)) })
        .collect();

    if !files.is_empty() || depth >= 1 {
        sort_found(&mut files);
        return files;
    }

    let mut nested = Vec::new();
    for (name, dir) in &entries {
        if !dir || name.starts_with('.') {
            continue;
        }
        for file in list_images(&folder.join(name), depth + 1) {
            nested.push(Found { rel: format!("{name}/{}", file.rel), taken_at: file.taken_at });
        }
    }
    sort_found(&mut nested);
    nested
}

/// Screenshots of `game` under the captures root `folder`.
///
/// The matched sub-folder is chosen with [`pick_screenshot_folder`]
/// (minimum score 64); items are oldest first. When the root does not exist
/// the result is empty with `folder` still set; when no folder matches,
/// `matched_folder` is `None`.
pub fn list_screenshots(folder: &Path, game: &GameQuery) -> ScreenshotsResult {
    if folder.as_os_str().is_empty() {
        return ScreenshotsResult::default();
    }
    let root = resolve_path(folder);
    if !root.exists() {
        return ScreenshotsResult { folder: Some(root), ..Default::default() };
    }
    let queries = game.queries();
    let folders = list_game_folders(&root);
    let Some(found) = pick_screenshot_folder(&queries, &folders, game.steam_app_id.as_deref(), MIN_SCORE) else {
        return ScreenshotsResult { folder: Some(root), ..Default::default() };
    };
    let matched = root.join(&found.folder);
    let items = list_images(&matched, 0)
        .into_iter()
        .map(|file| {
            let path = matched.join(&file.rel);
            let name = file.rel.rsplit('/').next().unwrap_or(&file.rel).to_owned();
            Screenshot {
                id: format!("{}/{}", found.folder, file.rel),
                path,
                name,
                taken_at: (file.taken_at != 0.0).then_some(file.taken_at),
            }
        })
        .collect();
    ScreenshotsResult { folder: Some(root), matched_folder: Some(found.folder), items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, secs: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let f = fs::File::create(path).unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn extension_rules() {
        for ok in ["a.png", "a.PNG", "a.JpEg", "a.webp", "a.gif", "a.bmp", "a.avif", "a.jxl", "a.jxr", "x.y.jpg"] {
            assert!(is_image_file(ok), "{ok}");
        }
        for bad in ["a.txt", "png", "a.", "a.tmp", "a.mp4", "a.tiff", ""] {
            assert!(!is_image_file(bad), "{bad}");
        }
    }

    #[test]
    fn queries_dedupe_and_trim() {
        let g = GameQuery {
            title: " Elden Ring ".into(),
            steam_app_id: None,
            aliases: vec!["Elden Ring".into(), "x".into(), "".into(), "ER".into()],
        };
        assert_eq!(g.queries(), vec!["Elden Ring", "ER"]);
    }

    #[test]
    fn lists_oldest_first_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("Elden Ring/b.png"), 300);
        touch(&root.join("Elden Ring/a.jpg"), 100);
        touch(&root.join("Elden Ring/c.PNG"), 200);
        touch(&root.join("Elden Ring/.hidden.png"), 50);
        touch(&root.join("Elden Ring/notes.txt"), 10);
        touch(&root.join("Elden Ring/.cap.tmp"), 10);
        // A sub-folder is ignored when the folder has images itself.
        touch(&root.join("Elden Ring/sub/z.png"), 1);
        touch(&root.join("Other/x.png"), 1);

        let r = list_screenshots(root, &GameQuery::new("Elden Ring"));
        assert_eq!(r.matched_folder.as_deref(), Some("Elden Ring"));
        let ids: Vec<_> = r.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["Elden Ring/a.jpg", "Elden Ring/c.PNG", "Elden Ring/b.png"]);
        assert_eq!(r.items[0].taken_at, Some(100_000.0));
        assert_eq!(r.items[0].name, "a.jpg");
        assert_eq!(r.items[0].path, root.join("Elden Ring/a.jpg"));
    }

    #[test]
    fn recurses_one_level_when_no_direct_images() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("God of War/2024/b.png"), 20);
        touch(&root.join("God of War/2023/a.png"), 10);
        touch(&root.join("God of War/.cache/h.png"), 5);
        touch(&root.join("God of War/2023/deep/never.png"), 1);
        let r = list_screenshots(root, &GameQuery::new("God of War"));
        let ids: Vec<_> = r.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["God of War/2023/a.png", "God of War/2024/b.png"]);
        assert_eq!(r.items[0].name, "a.png");
    }

    #[test]
    fn no_match_and_missing_root() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Elden Ring/a.png"), 1);
        let r = list_screenshots(dir.path(), &GameQuery::new("Totally Unknown"));
        assert!(r.matched_folder.is_none() && r.items.is_empty());
        assert_eq!(r.folder.as_deref(), Some(dir.path()));
        let r = list_screenshots(&dir.path().join("nope"), &GameQuery::new("Elden Ring"));
        assert!(r.items.is_empty());
        let r = list_screenshots(Path::new(""), &GameQuery::new("Elden Ring"));
        assert!(r.folder.is_none());
    }

    #[test]
    fn steam_id_and_hidden_folders() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("1245620/a.png"), 1);
        touch(&dir.path().join(".Elden Ring/a.png"), 1);
        let g = GameQuery { title: "Elden Ring".into(), steam_app_id: Some("1245620".into()), aliases: vec![] };
        assert_eq!(list_screenshots(dir.path(), &g).matched_folder.as_deref(), Some("1245620"));
        // Hidden folders are never candidates.
        assert!(list_screenshots(dir.path(), &GameQuery::new("Elden Ring")).matched_folder.is_none());
        let _ = SystemTime::now();
    }

    #[test]
    fn resolve_path_collapses() {
        assert_eq!(resolve_path(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
        assert_eq!(resolve_path(Path::new("/a/b/")), PathBuf::from("/a/b"));
    }
}
