//! Saved console themes (`AppPaths::console_themes_dir()`, one
//! `<id>.json` each), export and import. An exported file is the theme's
//! JSON as is, so the same file can be shared, put in a theme store, or
//! imported on another machine.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use super::{presets, slug, ConsoleTheme};
use crate::paths::AppPaths;

/// Suggested extension of exported themes.
pub const EXTENSION: &str = "heronite-theme.json";

fn path_of(paths: &AppPaths, id: &str) -> PathBuf {
    paths.console_themes_dir().join(format!("{id}.json"))
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
pub fn save(paths: &AppPaths, theme: &ConsoleTheme) -> Result<ConsoleTheme> {
    let mut t = theme.clone().normalized();
    if t.id.is_empty() || presets::by_id(&t.id).is_some() {
        t.id = free_id(paths, &t.name);
    }
    std::fs::create_dir_all(paths.console_themes_dir())?;
    crate::json_store::write_indented(&path_of(paths, &t.id), &t, b"  ")?;
    Ok(t)
}

pub fn delete(paths: &AppPaths, id: &str) -> Result<()> {
    let p = path_of(paths, id);
    if p.exists() {
        std::fs::remove_file(&p).with_context(|| p.display().to_string())?;
    }
    Ok(())
}

/// A built-in or saved theme.
pub fn find(paths: &AppPaths, id: &str) -> Option<ConsoleTheme> {
    presets::by_id(id).or_else(|| list(paths).into_iter().find(|t| t.id == id))
}

pub fn export(theme: &ConsoleTheme, to: &Path) -> Result<()> {
    std::fs::write(to, theme.to_json()?).with_context(|| to.display().to_string())
}

/// Reads a theme file and saves it with the user's themes (under a new id
/// when its id is taken by a built-in one).
pub fn import(paths: &AppPaths, from: &Path) -> Result<ConsoleTheme> {
    let text = std::fs::read_to_string(from).with_context(|| from.display().to_string())?;
    let t = ConsoleTheme::from_json(&text)?;
    save(paths, &t)
}
