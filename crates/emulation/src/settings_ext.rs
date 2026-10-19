//! Settings writers that `heronite-data` does not have yet: the Ludusavi
//! section of `local_library/settings.json` and its "last backup" status
//! fields (ports of `setLudusaviSettings`, `recordLudusaviBackup`). Same
//! read-modify-write as Heronite's `settingsFile.set('ludusavi', ...)`.

use anyhow::Result;
use heronite_data::models::{CollectionSettingsFile, LudusaviSettings};
use heronite_data::{json_store, settings, DataPaths};

/// `Partial<LudusaviSettings>` of the settings dialog.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviSettingsPatch {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub use_installed_config: Option<bool>,
    #[serde(default)]
    pub binary_path: Option<String>,
    #[serde(default)]
    pub backup_path: Option<String>,
    /// 'zip' | 'simple'
    #[serde(default)]
    pub format: Option<String>,
    /// 'none' | 'deflate' | 'bzip2' | 'zstd'
    #[serde(default)]
    pub compression: Option<String>,
    #[serde(default)]
    pub restore_paths: Option<Vec<String>>,
    /// `Some(None)` goes back to Ludusavi's own retention.
    #[serde(default)]
    pub full_limit: Option<Option<u8>>,
    #[serde(default)]
    pub differential_limit: Option<Option<u8>>,
}

fn current_ludusavi(paths: &DataPaths) -> Result<LudusaviSettings> {
    Ok(settings::load(paths)?.ludusavi)
}

/// `setLudusaviSettings`; returns the effective settings.
pub fn set_ludusavi_settings(paths: &DataPaths, patch: &LudusaviSettingsPatch) -> Result<settings::EffectiveSettings> {
    let mut next = current_ludusavi(paths)?;
    if let Some(v) = patch.enabled {
        next.enabled = v;
    }
    if let Some(v) = patch.use_installed_config {
        next.use_installed_config = v;
    }
    if let Some(v) = &patch.binary_path {
        next.binary_path = v.trim().to_owned();
    }
    if let Some(v) = &patch.backup_path {
        next.backup_path = v.trim().to_owned();
    }
    if let Some(v) = &patch.format {
        next.format = v.clone();
    }
    if let Some(v) = &patch.compression {
        next.compression = v.clone();
    }
    if let Some(v) = &patch.restore_paths {
        let mut paths: Vec<String> = Vec::new();
        for p in v.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            if !paths.iter().any(|q| q == p) {
                paths.push(p.to_owned());
            }
        }
        next.restore_paths = paths;
    }
    if let Some(v) = patch.full_limit {
        next.full_limit = v.filter(|n| *n >= 1);
    }
    if let Some(v) = patch.differential_limit {
        next.differential_limit = v;
    }
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |f| {
        f.ludusavi = Some(next);
        Ok(())
    })?;
    settings::load(paths)
}

/// Leaves one game out of the after-play backup (`on: false`) or puts it
/// back in.
pub fn set_auto_backup(paths: &DataPaths, runner: &str, app_name: &str, on: bool) -> Result<()> {
    let mut next = current_ludusavi(paths)?;
    let key = heronite_data::models::game_key(runner, app_name);
    next.auto_backup_off.retain(|k| *k != key);
    if !on {
        next.auto_backup_off.push(key);
    }
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |f| {
        f.ludusavi = Some(next);
        Ok(())
    })
}

/// Outcome of a Ludusavi save backup to record.
#[derive(Debug, Clone, Default)]
pub struct LudusaviRecord {
    pub at: Option<String>,
    pub game: Option<String>,
    pub path: Option<String>,
    pub error: Option<String>,
}

/// `recordLudusaviBackup`: an error keeps the previous success fields and sets
/// `lastError`; a success replaces them and clears `lastError`.
pub fn record_ludusavi_backup(paths: &DataPaths, rec: &LudusaviRecord) -> Result<()> {
    let mut next = current_ludusavi(paths)?;
    if rec.error.is_none() {
        next.last_backup_at = rec.at.clone();
        next.last_backup_game = rec.game.clone();
        next.last_backup_path = rec.path.clone();
    }
    next.last_error = rec.error.clone();
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |f| {
        f.ludusavi = Some(next);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ludusavi_settings_patch_and_records() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(
            paths.collection_settings(),
            "{\n\t\"greyUninstalledGames\": false,\n\t\"ludusavi\": {\n\t\t\"enabled\": true,\n\t\t\"useInstalledConfig\": true,\n\t\t\"binaryPath\": \"\",\n\t\t\"backupPath\": \"\",\n\t\t\"format\": \"zip\",\n\t\t\"compression\": \"deflate\"\n\t}\n}",
        )
        .unwrap();
        let eff = set_ludusavi_settings(
            &paths,
            &LudusaviSettingsPatch {
                binary_path: Some("  /usr/bin/ludusavi ".into()),
                format: Some("simple".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(eff.ludusavi.binary_path, "/usr/bin/ludusavi");
        assert_eq!(eff.ludusavi.format, "simple");
        assert!(eff.ludusavi.enabled);
        let eff = set_ludusavi_settings(
            &paths,
            &LudusaviSettingsPatch {
                restore_paths: Some(vec![" /win ".into(), "".into(), "/win".into(), "/old".into()]),
                full_limit: Some(Some(5)),
                differential_limit: Some(Some(0)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(eff.ludusavi.restore_paths, ["/win", "/old"]);
        assert_eq!((eff.ludusavi.full_limit, eff.ludusavi.differential_limit), (Some(5), Some(0)));
        assert_eq!(eff.ludusavi.format, "simple");
        let eff = set_ludusavi_settings(&paths, &LudusaviSettingsPatch { full_limit: Some(None), ..Default::default() }).unwrap();
        assert_eq!(eff.ludusavi.full_limit, None);
        assert_eq!(eff.ludusavi.restore_paths.len(), 2);
        // Unknown top-level keys survive.
        let raw = json_store::read_value(&paths.collection_settings()).unwrap();
        assert_eq!(raw["greyUninstalledGames"], false);

        record_ludusavi_backup(
            &paths,
            &LudusaviRecord {
                at: Some("2026-10-07T02:50:47.455Z".into()),
                game: Some("Tony Hawk's Underground".into()),
                path: Some("/b/Tony".into()),
                error: None,
            },
        )
        .unwrap();
        record_ludusavi_backup(&paths, &LudusaviRecord { error: Some("boom".into()), ..Default::default() }).unwrap();
        let l = settings::load(&paths).unwrap().ludusavi;
        assert_eq!(l.last_backup_game.as_deref(), Some("Tony Hawk's Underground"));
        assert_eq!(l.last_error.as_deref(), Some("boom"));
        record_ludusavi_backup(&paths, &LudusaviRecord { at: Some("t2".into()), game: Some("G".into()), path: None, error: None }).unwrap();
        let l = settings::load(&paths).unwrap().ludusavi;
        assert_eq!(l.last_error, None);
        assert_eq!(l.last_backup_path, None);
        assert_eq!(l.last_backup_at.as_deref(), Some("t2"));
    }

    #[test]
    fn auto_backup_can_be_off_for_one_game() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.collection_settings(), r#"{"ludusavi":{"enabled":true,"backupPath":"/b"}}"#).unwrap();
        set_auto_backup(&paths, "gog", "123", false).unwrap();
        set_auto_backup(&paths, "gog", "123", false).unwrap();
        let l = settings::load(&paths).unwrap().ludusavi;
        assert_eq!(l.auto_backup_off, ["gog:123"]);
        assert!(!l.auto_backup_for("gog", "123"));
        assert!(l.auto_backup_for("legendary", "123"));
        // Other settings and records keep the list.
        set_ludusavi_settings(&paths, &LudusaviSettingsPatch { format: Some("simple".into()), ..Default::default() }).unwrap();
        record_ludusavi_backup(&paths, &LudusaviRecord { error: Some("x".into()), ..Default::default() }).unwrap();
        assert_eq!(settings::load(&paths).unwrap().ludusavi.auto_backup_off, ["gog:123"]);
        set_auto_backup(&paths, "gog", "123", true).unwrap();
        let raw = json_store::read_value(&paths.collection_settings()).unwrap();
        assert!(raw["ludusavi"].get("autoBackupOff").is_none());
    }
}
