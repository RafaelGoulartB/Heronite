//! Settings writers that `heronite-data` does not have yet: the Ludusavi
//! section and the "last backup" status fields of `local_library/settings.json`
//! (ports of `setLudusaviSettings`, `recordLudusaviBackup`,
//! `recordCollectionBackup`). Same read-modify-write as Heronite's
//! `settingsFile.set('ludusavi' | 'backup', ...)`.

use anyhow::Result;
use heronite_data::models::{BackupSettings, CollectionSettingsFile, LudusaviSettings};
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
}

fn current_ludusavi(paths: &DataPaths) -> Result<LudusaviSettings> {
    Ok(settings::load(paths)?.ludusavi)
}

fn current_backup(paths: &DataPaths) -> Result<BackupSettings> {
    Ok(settings::load(paths)?.backup)
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
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |f| {
        f.ludusavi = Some(next);
        Ok(())
    })?;
    settings::load(paths)
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

/// Outcome of a config backup to record.
#[derive(Debug, Clone, Default)]
pub struct BackupRecord {
    pub at: String,
    pub path: Option<String>,
    pub error: Option<String>,
}

/// `recordCollectionBackup`.
pub fn record_collection_backup(paths: &DataPaths, rec: &BackupRecord) -> Result<()> {
    let mut next = current_backup(paths)?;
    if rec.error.is_none() {
        next.last_backup_at = Some(rec.at.clone());
        next.last_backup_path = rec.path.clone();
    }
    next.last_error = rec.error.clone();
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |f| {
        f.backup = Some(next);
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
    fn backup_records_keep_last_success_on_error() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        settings::save_backup(&paths, "/b", "daily").unwrap();
        record_collection_backup(&paths, &BackupRecord { at: "t1".into(), path: Some("/b/heroic-x".into()), error: None }).unwrap();
        record_collection_backup(&paths, &BackupRecord { at: "t2".into(), path: None, error: Some("bad".into()) }).unwrap();
        let b = settings::load(&paths).unwrap().backup;
        assert_eq!(b.last_backup_at.as_deref(), Some("t1"));
        assert_eq!(b.last_backup_path.as_deref(), Some("/b/heroic-x"));
        assert_eq!(b.last_error.as_deref(), Some("bad"));
        assert_eq!(b.folder, "/b");
        assert_eq!(b.interval, "daily");
    }
}
