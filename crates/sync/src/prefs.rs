//! Settings → Sync of this PC (`sync.json` in the app state): where the
//! library syncs, this PC's id and name, and how the last sync went. They
//! belong to the PC: none of it syncs.

use anyhow::Result;
use heronite_data::{json_store, AppPaths};
use serde::{Deserialize, Serialize};

/// Where the PCs meet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// A folder every PC reaches: a drive both systems of a dual boot
    /// mount, or one Syncthing, Dropbox or OneDrive keep in sync.
    #[default]
    Folder,
    Drive,
    Dropbox,
    OneDrive,
    Box,
    Ftp,
    Smb,
    WebDav,
    /// A remote of the user's own rclone settings.
    Custom,
}

impl Provider {
    pub const ALL: [Provider; 9] = [
        Provider::Folder,
        Provider::Drive,
        Provider::Dropbox,
        Provider::OneDrive,
        Provider::Box,
        Provider::Ftp,
        Provider::Smb,
        Provider::WebDav,
        Provider::Custom,
    ];

    /// Signs in through the browser (rclone's OAuth).
    pub fn is_oauth(self) -> bool {
        matches!(self, Provider::Drive | Provider::Dropbox | Provider::OneDrive | Provider::Box)
    }

    /// Needs a server, a user and a password.
    pub fn is_server(self) -> bool {
        matches!(self, Provider::Ftp | Provider::Smb | Provider::WebDav)
    }

    /// Goes through rclone.
    pub fn uses_rclone(self) -> bool {
        self != Provider::Folder
    }
}

/// The WebDAV servers rclone knows (`vendor`).
pub const WEBDAV_VENDORS: [(&str, &str); 5] =
    [("nextcloud", "Nextcloud"), ("owncloud", "ownCloud"), ("sharepoint", "SharePoint"), ("fastmail", "Fastmail"), ("other", "Other")];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncPrefs {
    pub enabled: bool,
    pub provider: Provider,
    /// [`Provider::Folder`]: the shared folder.
    pub folder: String,
    /// Cloud accounts and servers: the folder in them.
    pub remote_folder: String,
    /// [`Provider::Custom`]: `remote:path` of the user's rclone settings.
    pub custom_remote: String,
    /// The account connected in `rclone.conf`, and how it shows
    /// (`user@host`, or empty for a cloud account).
    pub connected: Option<Provider>,
    pub account: String,
    pub device_id: String,
    pub device_name: String,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    pub last_summary: Option<LastSummary>,
    /// The other PCs found by the last sync.
    pub devices: Vec<SeenDevice>,
}

impl Default for SyncPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: Provider::Folder,
            folder: String::new(),
            remote_folder: DEFAULT_REMOTE_FOLDER.into(),
            custom_remote: String::new(),
            connected: None,
            account: String::new(),
            device_id: String::new(),
            device_name: String::new(),
            last_sync_at: None,
            last_error: None,
            last_summary: None,
            devices: Vec::new(),
        }
    }
}

pub const DEFAULT_REMOTE_FOLDER: &str = "Heronite";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct LastSummary {
    pub fields: usize,
    pub sessions: usize,
    pub playtime: usize,
    pub waiting: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SeenDevice {
    pub id: String,
    pub name: String,
    pub os: String,
    /// When it last wrote its file (RFC 3339).
    pub written_at: String,
}

impl SyncPrefs {
    /// Whether a sync has somewhere to go.
    pub fn ready(&self) -> bool {
        match self.provider {
            Provider::Folder => !self.folder.trim().is_empty(),
            Provider::Custom => self.custom_remote.contains(':'),
            p => self.connected == Some(p),
        }
    }

    /// The name this PC shows to the others.
    pub fn shown_name(&self) -> String {
        match self.device_name.trim() {
            "" => default_device_name(),
            name => name.to_owned(),
        }
    }
}

/// Reads the settings; a PC without an id gets one (saved at once, so it
/// never changes).
pub fn load(app: &AppPaths) -> Result<SyncPrefs> {
    let mut prefs: SyncPrefs = json_store::read(&app.sync_prefs())?;
    if prefs.device_id.is_empty() {
        prefs.device_id = heronite_data::sideload::generate_app_name();
        save(app, &prefs)?;
    }
    Ok(prefs)
}

pub fn save(app: &AppPaths, prefs: &SyncPrefs) -> Result<()> {
    if let Some(dir) = app.sync_prefs().parent() {
        std::fs::create_dir_all(dir)?;
    }
    json_store::write_indented(&app.sync_prefs(), prefs, b"  ")
}

/// Changes the settings: read, change, write.
pub fn update(app: &AppPaths, f: impl FnOnce(&mut SyncPrefs)) -> Result<SyncPrefs> {
    let mut prefs = load(app)?;
    f(&mut prefs);
    save(app, &prefs)?;
    Ok(prefs)
}

/// `<host name> (Linux)`.
pub fn default_device_name() -> String {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    match host_name() {
        Some(host) => format!("{host} ({os})"),
        None => os.to_owned(),
    }
}

fn host_name() -> Option<String> {
    let name = if cfg!(windows) {
        std::env::var("COMPUTERNAME").ok()
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname").ok().or_else(|| std::fs::read_to_string("/etc/hostname").ok())
    }?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pc_keeps_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let first = load(&app).unwrap();
        assert!(!first.device_id.is_empty());
        assert_eq!(load(&app).unwrap().device_id, first.device_id);
        assert_eq!(first.remote_folder, DEFAULT_REMOTE_FOLDER);
        assert!(!first.ready());
    }

    #[test]
    fn ready_once_there_is_somewhere_to_go() {
        let mut p = SyncPrefs { folder: "/mnt/shared".into(), ..Default::default() };
        assert!(p.ready());
        p.provider = Provider::Drive;
        assert!(!p.ready());
        p.connected = Some(Provider::Drive);
        assert!(p.ready());
        p.provider = Provider::Custom;
        p.custom_remote = "mine:Heronite".into();
        assert!(p.ready());
    }
}
