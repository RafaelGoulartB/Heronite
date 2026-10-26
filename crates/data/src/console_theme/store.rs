//! Saved console themes (`AppPaths::console_themes_dir()`, one
//! `<id>.json` each, its own picture next to it as
//! `<id>.background-<hash>.<ext>`), export and import. An exported file is
//! the theme's JSON with its picture inside it (a `data:` URL), so the one
//! file can be shared, put in a theme store, or imported on another machine;
//! saving it writes the picture out again.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use base64::Engine as _;

use super::{presets, slug, BackgroundSource, ConsoleTheme};
use crate::paths::AppPaths;

/// Suggested extension of exported themes.
pub const EXTENSION: &str = "heronite-theme.json";
/// A picture bigger than this stays out of an export (a shared theme file
/// should stay small enough to send).
const MAX_EMBED: u64 = 16 * 1024 * 1024;
/// The picture formats a theme can carry, as `(extension, MIME type)`.
const PICTURES: [(&str, &str); 4] = [("png", "image/png"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg"), ("webp", "image/webp")];

fn path_of(paths: &AppPaths, id: &str) -> PathBuf {
    paths.console_themes_dir().join(format!("{id}.json"))
}

/// The start of the file names of the pictures of theme `id`.
fn picture_prefix(id: &str) -> String {
    format!("{id}.background-")
}

/// The user's saved themes, by name. Unreadable files are skipped (logged).
pub fn list(paths: &AppPaths) -> Vec<ConsoleTheme> {
    let Ok(dir) = std::fs::read_dir(paths.console_themes_dir()) else { return Vec::new() };
    let mut out: Vec<ConsoleTheme> = dir
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            match ConsoleTheme::from_json(&text) {
                Ok(mut t) => {
                    if t.id.is_empty() {
                        t.id = e.path().file_stem()?.to_string_lossy().into_owned();
                    }
                    Some(t)
                }
                Err(err) => {
                    log::warn!("console theme {}: {err:#}", e.path().display());
                    None
                }
            }
        })
        .collect();
    out.sort_by_key(|t| crate::fuzzy::fold(&t.name));
    out
}

/// An id for `name` that no built-in or saved theme uses.
pub fn free_id(paths: &AppPaths, name: &str) -> String {
    let base = slug(name);
    let taken = |id: &str| presets::by_id(id).is_some() || path_of(paths, id).exists();
    if !taken(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).unwrap_or(base)
}

/// Saves `theme` under its id (a new one when empty or built-in); returns it.
/// Its own picture becomes a file of the theme (see [`own_picture`]).
pub fn save(paths: &AppPaths, theme: &ConsoleTheme) -> Result<ConsoleTheme> {
    let mut t = theme.clone().normalized();
    if t.id.is_empty() || presets::by_id(&t.id).is_some() {
        t.id = free_id(paths, &t.name);
    }
    std::fs::create_dir_all(paths.console_themes_dir())?;
    own_picture(paths, &mut t)?;
    crate::json_store::write_indented(&path_of(paths, &t.id), &t, b"  ")?;
    Ok(t)
}

/// Deletes the saved theme `id` and its picture.
pub fn delete(paths: &AppPaths, id: &str) -> Result<()> {
    let p = path_of(paths, id);
    if p.exists() {
        std::fs::remove_file(&p).with_context(|| p.display().to_string())?;
    }
    remove_pictures(paths, id, None);
    Ok(())
}

/// A built-in or saved theme.
pub fn find(paths: &AppPaths, id: &str) -> Option<ConsoleTheme> {
    presets::by_id(id).or_else(|| list(paths).into_iter().find(|t| t.id == id))
}

/// Writes `theme` as a file to share: its picture goes inside it.
pub fn export(theme: &ConsoleTheme, to: &Path) -> Result<()> {
    std::fs::write(to, portable(theme).to_json()?).with_context(|| to.display().to_string())
}

/// `theme` as it is shared: its own picture inside it as a `data:` URL when
/// the picture is in use, a local path that would mean nothing on another
/// machine left out. A missing or too big picture keeps its path.
pub fn portable(theme: &ConsoleTheme) -> ConsoleTheme {
    let mut t = theme.clone();
    let image = t.background.image.trim().to_string();
    if image.is_empty() || is_url(&image) || image.starts_with("data:") {
        return t;
    }
    if t.background.source != BackgroundSource::Image {
        t.background.image.clear();
        return t;
    }
    let path = PathBuf::from(&image);
    let Some(mime) = mime_of(&path) else { return t };
    let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_EMBED);
    match std::fs::read(&path) {
        Ok(bytes) if small => {
            t.background.image = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
        }
        Ok(_) => log::warn!("console theme picture {}: too big to put in the file", path.display()),
        Err(e) => log::warn!("console theme picture {}: {e}", path.display()),
    }
    t
}

/// Reads a theme file without saving it (to show it before it is imported).
pub fn read(from: &Path) -> Result<ConsoleTheme> {
    let text = std::fs::read_to_string(from).with_context(|| from.display().to_string())?;
    ConsoleTheme::from_json(&text)
}

/// How a theme about to be imported stands with the saved ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// Nothing like it is saved.
    New,
    /// The saved theme with this id is the same theme.
    Same(String),
    /// A saved theme has its id but differs (another version of it).
    SameId(String),
}

/// Compares `theme` with the saved ones; pictures count as the same when
/// both themes have one (their paths always differ).
pub fn compare(theme: &ConsoleTheme, saved: &[ConsoleTheme]) -> Match {
    let key = |t: &ConsoleTheme| {
        let mut t = t.clone().normalized();
        t.id.clear();
        if !t.background.image.is_empty() {
            t.background.image = "picture".into();
        }
        t
    };
    let mine = key(theme);
    if let Some(s) = saved.iter().find(|s| key(s) == mine) {
        return Match::Same(s.id.clone());
    }
    match saved.iter().find(|s| !theme.id.is_empty() && s.id == theme.id) {
        Some(s) => Match::SameId(s.id.clone()),
        None => Match::New,
    }
}

/// Saves a theme read from a file: `replace` overwrites the saved theme
/// with its id, otherwise a taken id gives it a new one (both are kept).
pub fn add(paths: &AppPaths, theme: &ConsoleTheme, replace: bool) -> Result<ConsoleTheme> {
    let mut t = theme.clone();
    if !replace && (t.id.is_empty() || path_of(paths, &t.id).exists()) {
        t.id.clear();
    }
    save(paths, &t)
}

/// Reads a theme file and saves it with the user's themes (under a new id
/// when its id is taken).
pub fn import(paths: &AppPaths, from: &Path) -> Result<ConsoleTheme> {
    add(paths, &read(from)?, false)
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn mime_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    PICTURES.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

/// FNV-1a: tells the versions of a picture apart in its file name, so a
/// new picture never reuses the cached texture of the old one.
fn short_hash(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    format!("{:08x}", h as u32)
}

/// Makes the picture of `t` a file of the theme folder: a `data:` URL (an
/// imported theme) is written out and a file elsewhere copied, so it
/// survives the original moving and goes away with the theme. A file that
/// cannot be read keeps its path.
fn own_picture(paths: &AppPaths, t: &mut ConsoleTheme) -> Result<()> {
    let image = t.background.image.trim().to_string();
    if image.is_empty() || is_url(&image) {
        return Ok(());
    }
    let dir = paths.console_themes_dir();
    let prefix = picture_prefix(&t.id);
    let (bytes, ext) = if let Some(data) = image.strip_prefix("data:") {
        let (head, body) = data.split_once(',').context("a picture in the theme is not readable")?;
        let mime = head.strip_suffix(";base64").context("a picture in the theme is not readable")?;
        let ext = PICTURES.iter().find(|(_, m)| *m == mime).map(|(e, _)| *e).context("the theme's picture is not a PNG, JPEG or WebP")?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(body.trim()).context("a picture in the theme is not readable")?;
        (bytes, ext.to_string())
    } else {
        let from = PathBuf::from(&image);
        let ours = from.parent() == Some(dir.as_path()) && from.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix));
        if ours {
            return Ok(());
        }
        match std::fs::read(&from) {
            Ok(b) => (b, from.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "png".into())),
            Err(e) => {
                log::warn!("console theme picture {}: {e}", from.display());
                return Ok(());
            }
        }
    };
    let name = format!("{prefix}{}.{ext}", short_hash(&bytes));
    let to = dir.join(&name);
    remove_pictures(paths, &t.id, Some(&name));
    std::fs::write(&to, bytes).with_context(|| to.display().to_string())?;
    t.background.image = to.display().to_string();
    Ok(())
}

/// Removes the pictures of theme `id` but `keep`.
fn remove_pictures(paths: &AppPaths, id: &str, keep: Option<&str>) {
    let prefix = picture_prefix(id);
    let Ok(dir) = std::fs::read_dir(paths.console_themes_dir()) else { return };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && Some(name.as_str()) != keep {
            std::fs::remove_file(e.path()).ok();
        }
    }
}
