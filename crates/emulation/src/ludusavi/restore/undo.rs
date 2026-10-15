//! A copy of what a restore overwrites, to put it back.
//!
//! Ludusavi's own backup of the current saves is no safety net: with a small
//! retention it would push out the very backup being restored. So before a
//! restore the files it will write are copied to
//! `save-restore-undo/<game>/<time>/` in the data folder, with `undo.json`
//! listing them; files the restore creates are listed without a copy and are
//! deleted on undo. The newest [`KEEP`] copies of each game are kept.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::iso::iso_now;

/// Undo copies kept per game.
pub const KEEP: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoEntry {
    pub target: String,
    /// File name of the copy in `files/`; `None`: the restore created it.
    pub stored: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoRecord {
    /// The folder of this copy.
    #[serde(skip)]
    pub dir: PathBuf,
    pub created_at: String,
    /// Ludusavi's name of the game and the backup restored.
    pub game_name: String,
    pub backup_id: String,
    pub entries: Vec<UndoEntry>,
}

/// `app_name` as a folder name.
fn game_key(app_name: &str) -> String {
    let key: String = app_name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if key.is_empty() {
        "_".into()
    } else {
        key
    }
}

/// Copies the files at `targets` that exist and records the others.
pub fn snapshot(root: &Path, app_name: &str, game_name: &str, backup_id: &str, targets: &[String]) -> Result<UndoRecord> {
    let game_dir = root.join(game_key(app_name));
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
    let dir = game_dir.join(&stamp);
    let files = dir.join("files");
    fs::create_dir_all(&files).with_context(|| format!("creating {}", files.display()))?;
    let mut entries = Vec::new();
    for (i, target) in targets.iter().enumerate() {
        let src = Path::new(target);
        let stored = if src.is_file() {
            let name = i.to_string();
            fs::copy(src, files.join(&name)).with_context(|| format!("copying {target}"))?;
            Some(name)
        } else {
            None
        };
        entries.push(UndoEntry { target: target.clone(), stored });
    }
    let record = UndoRecord { dir: dir.clone(), created_at: iso_now(), game_name: game_name.into(), backup_id: backup_id.into(), entries };
    fs::write(dir.join("undo.json"), serde_json::to_string_pretty(&record)?)?;
    prune(&game_dir);
    Ok(record)
}

fn copies(game_dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(game_dir)
        .map(|it| it.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.join("undo.json").is_file()).collect())
        .unwrap_or_default();
    dirs.sort();
    dirs
}

fn prune(game_dir: &Path) {
    let dirs = copies(game_dir);
    for old in dirs.iter().take(dirs.len().saturating_sub(KEEP)) {
        if let Err(e) = fs::remove_dir_all(old) {
            log::warn!("could not remove the undo copy {}: {e}", old.display());
        }
    }
}

/// The newest undo copy of a game.
pub fn latest(root: &Path, app_name: &str) -> Option<UndoRecord> {
    let dir = copies(&root.join(game_key(app_name))).pop()?;
    let mut record: UndoRecord = serde_json::from_str(&fs::read_to_string(dir.join("undo.json")).ok()?).ok()?;
    record.dir = dir;
    Some(record)
}

/// Puts the copied files back, deletes the ones the restore created, then
/// drops the copy. Returns how many files were put back or removed.
pub fn undo(record: &UndoRecord) -> Result<usize> {
    let mut done = 0;
    for e in &record.entries {
        let target = Path::new(&e.target);
        match &e.stored {
            Some(name) => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(record.dir.join("files").join(name), target).with_context(|| format!("restoring {}", e.target))?;
                done += 1;
            }
            None if target.is_file() => {
                fs::remove_file(target).with_context(|| format!("removing {}", e.target))?;
                done += 1;
            }
            None => {}
        }
    }
    discard(record);
    Ok(done)
}

/// Deletes an undo copy.
pub fn discard(record: &UndoRecord) {
    if let Err(e) = fs::remove_dir_all(&record.dir) {
        log::warn!("could not remove the undo copy {}: {e}", record.dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_and_undo() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("undo");
        let saves = d.path().join("saves");
        fs::create_dir_all(&saves).unwrap();
        let old = saves.join("slot1.sav");
        let new = saves.join("deep/slot2.sav");
        fs::write(&old, "old").unwrap();
        let targets = vec![old.to_string_lossy().into_owned(), new.to_string_lossy().into_owned()];
        let rec = snapshot(&root, "app/1", "Game", "backup-1.zip", &targets).unwrap();
        assert_eq!(rec.entries[0].stored.as_deref(), Some("0"));
        assert_eq!(rec.entries[1].stored, None);

        // The restore overwrites one file and creates the other.
        fs::write(&old, "restored").unwrap();
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::write(&new, "restored").unwrap();

        let found = latest(&root, "app/1").unwrap();
        assert_eq!(found, rec);
        assert_eq!(undo(&found).unwrap(), 2);
        assert_eq!(fs::read_to_string(&old).unwrap(), "old");
        assert!(!new.exists());
        assert!(latest(&root, "app/1").is_none());
    }

    #[test]
    fn keeps_the_newest_copies() {
        let d = tempfile::tempdir().unwrap();
        for _ in 0..KEEP + 2 {
            snapshot(d.path(), "g", "G", "b", &[]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(copies(&d.path().join("g")).len(), KEEP);
        assert_eq!(game_key(""), "_");
        assert_eq!(game_key("a b/c"), "a_b_c");
    }
}
