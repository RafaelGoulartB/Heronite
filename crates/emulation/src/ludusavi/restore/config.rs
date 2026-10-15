//! The throwaway Ludusavi config folder of one restore (`--config <dir>`).
//!
//! Ludusavi reads YAML, and JSON is YAML, so the file is written with
//! serde_json; every key left out takes Ludusavi's default. No manifest is
//! needed to restore a game by its backup name, so none is copied and the
//! commands run with `--no-manifest-update`.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::plan::Redirect;

/// Removed with its folder when dropped.
pub(super) struct TempConfig {
    dir: PathBuf,
}

impl TempConfig {
    /// `redirects` apply on restore; `skipped` are target paths of `game`
    /// to leave out (`restore.toggledPaths`).
    pub(super) fn write(game: &str, redirects: &[Redirect], skipped: &[&str]) -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("heronite-ludusavi-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir)?;
        let this = Self { dir };
        fs::write(this.dir.join("config.yaml"), contents(game, redirects, skipped).to_string())?;
        Ok(this)
    }

    #[cfg(test)]
    fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// `--config <dir> --no-manifest-update`, before the subcommand.
    pub(super) fn args(&self) -> Vec<String> {
        vec!["--config".into(), self.dir.to_string_lossy().into_owned(), "--no-manifest-update".into()]
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.dir) {
            log::warn!("could not remove {}: {e}", self.dir.display());
        }
    }
}

fn contents(game: &str, redirects: &[Redirect], skipped: &[&str]) -> Value {
    let redirects: Vec<Value> = redirects.iter().map(|r| json!({"kind": "restore", "source": r.source, "target": r.target})).collect();
    let mut toggled = Map::new();
    if !skipped.is_empty() {
        let paths: Map<String, Value> = skipped.iter().map(|p| (p.to_string(), Value::Bool(false))).collect();
        toggled.insert(game.to_string(), Value::Object(paths));
    }
    json!({
        "release": {"check": false},
        "redirects": redirects,
        "restore": {"toggledPaths": toggled},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_redirects_and_skipped_paths() {
        let r = Redirect { source: "C:/Users/x".into(), target: "/p/drive_c/users/steamuser".into() };
        let cfg = TempConfig::write("Elden Ring", &[r], &["/p/user.reg"]).unwrap();
        let dir = cfg.dir().to_path_buf();
        let text = fs::read_to_string(dir.join("config.yaml")).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["redirects"][0]["kind"], "restore");
        assert_eq!(v["redirects"][0]["source"], "C:/Users/x");
        assert_eq!(v["restore"]["toggledPaths"]["Elden Ring"]["/p/user.reg"], false);
        assert_eq!(cfg.args()[0], "--config");
        drop(cfg);
        assert!(!dir.exists());
        let empty = contents("G", &[], &[]);
        assert_eq!(empty["restore"]["toggledPaths"], json!({}));
    }
}
