//! What gogdl kept of an installed game under `GOGDL_CONFIG_PATH`
//! (`<data>/gogdlConfig`): the build it installed, in
//! `heroic_gogdl/manifests/<id>` (the build's meta with the language, the
//! DLCs and, for v1, the platform), and the support folder in
//! `heroic_gogdl/gog-support/<id>`. A Linux install keeps
//! `.gogdl-linux-manifest` in the game's folder instead.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::api::Client;
use crate::files::FileEntry;
use crate::{v1, v2};

/// The folder gogdl creates under its config path.
const TOOL_DIR: &str = "heroic_gogdl";
/// A Linux install's record, in the game's folder.
pub const LINUX_MANIFEST: &str = ".gogdl-linux-manifest";

/// `<config>/heroic_gogdl/manifests/<id>`.
pub fn manifest_file(config_dir: &Path, game_id: &str) -> PathBuf {
    config_dir.join(TOOL_DIR).join("manifests").join(game_id)
}

/// `<config>/heroic_gogdl/gog-support/<id>`.
pub fn support_dir(config_dir: &Path, game_id: &str) -> PathBuf {
    config_dir.join(TOOL_DIR).join("gog-support").join(game_id)
}

/// The stored build of an installed game; `None` when there is none or it
/// cannot be read (an update then downloads everything).
pub fn read(config_dir: &Path, game_id: &str) -> Option<Value> {
    let bytes = std::fs::read(manifest_file(config_dir, game_id)).ok()?;
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(doc) if doc.is_object() => Some(doc),
        _ => {
            log::warn!("the stored build of GOG game {game_id} cannot be read");
            None
        }
    }
}

/// Saves the build an install or update ends with.
pub fn write(config_dir: &Path, game_id: &str, doc: &Value) -> Result<()> {
    let path = manifest_file(config_dir, game_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    heronite_data::temps::write_atomic(&path, &serde_json::to_vec(doc)?, None).with_context(|| format!("writing {}", path.display()))
}

/// The generation of a stored build (`version`: 1 or 2).
pub fn generation(doc: &Value) -> u8 {
    if doc.get("version").and_then(Value::as_u64) == Some(1) {
        1
    } else {
        2
    }
}

/// The files of a stored build, from its depot manifests (network).
pub fn files(client: &Client, doc: &Value) -> Result<Vec<FileEntry>> {
    match generation(doc) {
        1 => v1::stored_files(client, doc),
        _ => v2::stored_files(client, doc),
    }
}

/// The install language and DLC ids a stored build records.
pub fn language_and_dlcs(doc: &Value) -> (Option<String>, Vec<String>) {
    let lang = doc.get("HGLInstallLanguage").and_then(Value::as_str).map(str::to_string);
    (lang, v2::Dlc::list_from(doc.get("HGLdlcs")).into_iter().map(|d| d.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("gogdlConfig");
        assert!(manifest_file(&cfg, "42").ends_with("gogdlConfig/heroic_gogdl/manifests/42"));
        assert!(support_dir(&cfg, "42").ends_with("gogdlConfig/heroic_gogdl/gog-support/42"));
        assert!(read(&cfg, "42").is_none());
        let doc = json!({"version": 2, "baseProductId": "42", "HGLInstallLanguage": "de-DE", "HGLdlcs": [{"title": "D", "id": "7"}]});
        write(&cfg, "42", &doc).unwrap();
        let back = read(&cfg, "42").unwrap();
        assert_eq!(back, doc);
        assert_eq!(generation(&back), 2);
        assert_eq!(generation(&json!({"version": 1})), 1);
        assert_eq!(language_and_dlcs(&back), (Some("de-DE".into()), vec!["7".into()]));
        std::fs::write(manifest_file(&cfg, "43"), "{broken").unwrap();
        assert!(read(&cfg, "43").is_none());
    }
}
