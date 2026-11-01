//! Where the PCs' files are read and written: a folder, or a cloud account
//! through rclone (its files copied into `AppPaths::sync_mirror_dir` first).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use heronite_data::AppPaths;

use crate::format::{DeviceFile, DEVICES_DIR, STATE_FILE};
use crate::prefs::{Provider, SyncPrefs};
use crate::rclone::{self, Rclone, RcloneError};
use crate::SyncError;

pub enum Place {
    Folder(PathBuf),
    Cloud { rclone: Rclone, root: String },
}

/// Where `prefs` sync to. A cloud account needs rclone, downloaded here when
/// missing.
pub fn place(data: &heronite_data::DataPaths, app: &AppPaths, prefs: &SyncPrefs) -> Result<Place, SyncError> {
    if prefs.provider == Provider::Folder {
        return Ok(Place::Folder(PathBuf::from(prefs.folder.trim())));
    }
    let bin = rclone::ensure(data, &|| false).map_err(SyncError::NoRclone)?;
    Ok(match prefs.provider {
        Provider::Custom => Place::Cloud { rclone: Rclone { bin, config: None }, root: prefs.custom_remote.trim().to_owned() },
        _ => Place::Cloud {
            rclone: Rclone { bin, config: Some(app.rclone_conf()) },
            root: rclone::remote_path(&format!("{}:", rclone::REMOTE), &prefs.remote_folder),
        },
    })
}

impl Place {
    /// Every PC's file (this one's too).
    pub fn pull(&self, app: &AppPaths) -> Result<Vec<DeviceFile>, SyncError> {
        match self {
            Place::Folder(folder) => {
                if !folder.is_dir() {
                    return Err(SyncError::FolderMissing(folder.display().to_string()));
                }
                Ok(read_devices(folder))
            }
            Place::Cloud { rclone, root } => {
                let mirror = app.sync_mirror_dir();
                fs::create_dir_all(&mirror).map_err(|e| SyncError::Data(e.to_string()))?;
                rclone.pull(root, &mirror).map_err(cloud_error)?;
                Ok(read_devices(&mirror))
            }
        }
    }

    /// Writes this PC's file.
    pub fn push(&self, app: &AppPaths, own: &DeviceFile) -> Result<(), SyncError> {
        let bytes = own.to_bytes();
        match self {
            Place::Folder(folder) => {
                if !folder.is_dir() {
                    return Err(SyncError::FolderMissing(folder.display().to_string()));
                }
                let dir = folder.join(DEVICES_DIR).join(&own.device.id);
                fs::create_dir_all(&dir)
                    .and_then(|()| heronite_data::temps::write_atomic(&dir.join(STATE_FILE), &bytes, None))
                    .map_err(|e| write_error(folder, e))
            }
            Place::Cloud { rclone, root } => {
                let outbox = app.sync_mirror_dir().join("outbox");
                fs::create_dir_all(&outbox).map_err(|e| SyncError::Data(e.to_string()))?;
                let file = outbox.join(STATE_FILE);
                heronite_data::temps::write_atomic(&file, &bytes, None).map_err(|e| SyncError::Data(e.to_string()))?;
                rclone.push(&file, root, &own.device.id).map_err(cloud_error)
            }
        }
    }
}

fn cloud_error(e: RcloneError) -> SyncError {
    match e {
        RcloneError::Cancelled => SyncError::Cancelled,
        other => SyncError::Cloud(other.to_string()),
    }
}

fn write_error(folder: &Path, e: io::Error) -> SyncError {
    if e.kind() == io::ErrorKind::ReadOnlyFilesystem {
        SyncError::ReadOnly(folder.display().to_string())
    } else {
        SyncError::Write(e.to_string())
    }
}

/// `devices/*/state.json` under `root`. A file that cannot be read (being
/// copied, broken, newer) is skipped until the next sync.
fn read_devices(root: &Path) -> Vec<DeviceFile> {
    let Ok(dirs) = fs::read_dir(root.join(DEVICES_DIR)) else { return Vec::new() };
    let mut files = Vec::new();
    for dir in dirs.flatten() {
        let path = dir.path().join(STATE_FILE);
        let Ok(bytes) = fs::read(&path) else { continue };
        match DeviceFile::parse(&bytes) {
            Some(file) => files.push(file),
            None => log::warn!("sync: skipping {} (not readable by this version)", path.display()),
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{DeviceInfo, FORMAT};

    #[test]
    fn a_folder_holds_one_file_per_pc() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("c"), cache: dir.path().join("k") };
        let place = Place::Folder(dir.path().join("shared"));
        assert!(matches!(place.pull(&app), Err(SyncError::FolderMissing(_))));
        fs::create_dir_all(dir.path().join("shared")).unwrap();
        assert!(place.pull(&app).unwrap().is_empty());
        for id in ["linux", "windows"] {
            let file = DeviceFile { format: FORMAT, device: DeviceInfo { id: id.into(), ..Default::default() }, ..Default::default() };
            place.push(&app, &file).unwrap();
        }
        fs::create_dir_all(dir.path().join("shared/devices/broken")).unwrap();
        fs::write(dir.path().join("shared/devices/broken/state.json"), b"{\"format\":1,").unwrap();
        let mut ids: Vec<String> = place.pull(&app).unwrap().into_iter().map(|f| f.device.id).collect();
        ids.sort();
        assert_eq!(ids, ["linux", "windows"]);
    }
}
