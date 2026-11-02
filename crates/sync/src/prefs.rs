//! Settings → Cloud of this PC (`sync.json` in the app state): the account
//! or folder everything goes to, what goes (the library sync, backups, game
//! saves, screenshots), this PC's id and name, and how the last sync and
//! uploads went. They belong to the PC: none of it syncs.

use std::collections::BTreeMap;

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

/// When something goes to the account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Off,
    /// Only from Settings → Cloud ("Send now", "Sync now").
    Manual,
    /// By itself too: at start, after a game, as backups are made.
    Auto,
}

/// `off`, `manual` or `auto`; the on/off switches these replaced read as
/// `auto` and `off`, so settings saved by them still load.
impl<'de> Deserialize<'de> for Mode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Switch(bool),
            Name(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Switch(true) => Mode::Auto,
            Raw::Switch(false) => Mode::Off,
            Raw::Name(name) => match name.as_str() {
                "auto" => Mode::Auto,
                "manual" => Mode::Manual,
                _ => Mode::Off,
            },
        })
    }
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Off, Mode::Manual, Mode::Auto];

    pub fn is_on(self) -> bool {
        self != Mode::Off
    }

    /// Whether a run asked by the user (`manual`) or by itself may go.
    pub fn allows(self, manual: bool) -> bool {
        match self {
            Mode::Off => false,
            Mode::Manual => manual,
            Mode::Auto => true,
        }
    }
}

/// The WebDAV servers rclone knows (`vendor`).
pub const WEBDAV_VENDORS: [(&str, &str); 5] =
    [("nextcloud", "Nextcloud"), ("owncloud", "ownCloud"), ("sharepoint", "SharePoint"), ("fastmail", "Fastmail"), ("other", "Other")];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncPrefs {
    /// The library syncs with the other PCs (`enabled` when it was a switch).
    #[serde(alias = "enabled")]
    pub library: Mode,
    /// This PC publishes only its play (sessions and play time): what is
    /// changed here (status, favourites, notes...) stays here. For a PC the
    /// user does not keep organised.
    pub play_only: bool,
    /// Each backup of the data folder goes up as a zip (without sign-ins).
    pub backups: Mode,
    /// The Ludusavi backups go up (automatic: after each game).
    pub saves: Mode,
    /// The screenshots go up once compressed.
    pub captures: Mode,
    /// With the screenshots, the replays (videos).
    pub replays: bool,
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
    /// New on each sign-in: what went up belongs to one sign-in (another
    /// account, or another Google app, sees none of it).
    pub connection: String,
    /// Google Drive: the user's own Google app (`client_id`, `client_secret`)
    /// instead of the one this build carries (or rclone's).
    pub google_client_id: String,
    pub google_client_secret: String,
    pub device_id: String,
    pub device_name: String,
    /// This PC's folder in the account (`Backups/<it>`, `Saves/<it>`):
    /// its name when it first uploaded, so a rename does not split them.
    pub device_folder: String,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    pub last_summary: Option<LastSummary>,
    /// The other PCs found by the last sync.
    pub devices: Vec<SeenDevice>,
    /// How the last upload of each kind went (`Upload::key`).
    pub uploads: BTreeMap<String, LastUpload>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct LastUpload {
    /// When it last went through (RFC 3339).
    pub at: Option<String>,
    pub error: Option<String>,
}

impl Default for SyncPrefs {
    fn default() -> Self {
        Self {
            library: Mode::Off,
            play_only: false,
            backups: Mode::Off,
            saves: Mode::Off,
            captures: Mode::Off,
            replays: false,
            provider: Provider::Folder,
            folder: String::new(),
            remote_folder: DEFAULT_REMOTE_FOLDER.into(),
            custom_remote: String::new(),
            connected: None,
            account: String::new(),
            connection: String::new(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            device_id: String::new(),
            device_name: String::new(),
            device_folder: String::new(),
            last_sync_at: None,
            last_error: None,
            last_summary: None,
            devices: Vec::new(),
            uploads: BTreeMap::new(),
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

    /// Where things go now, to tell what went up there from what went
    /// elsewhere: the folder, the remote, or the sign-in and its folder.
    pub fn destination(&self) -> String {
        match self.provider {
            Provider::Folder => format!("folder:{}", self.folder.trim()),
            Provider::Custom => format!("custom:{}", self.custom_remote.trim()),
            _ => format!("{}:{}", self.connection, self.remote_folder.trim()),
        }
    }

    /// Whether anything goes to the account or folder.
    pub fn any_on(&self) -> bool {
        [self.library, self.backups, self.saves, self.captures].iter().any(|m| m.is_on())
    }

    /// The Google app Google Drive signs in with: the user's own, else the
    /// one this build was made with (`HERONITE_GOOGLE_CLIENT_ID` and
    /// `HERONITE_GOOGLE_CLIENT_SECRET` at build time), else none (rclone's
    /// own, whose requests are shared by all its users).
    pub fn google_app(&self) -> Option<(String, String)> {
        if !self.google_client_id.trim().is_empty() {
            return Some((self.google_client_id.trim().to_owned(), self.google_client_secret.trim().to_owned()));
        }
        let id = option_env!("HERONITE_GOOGLE_CLIENT_ID").filter(|s| !s.is_empty())?;
        Some((id.to_owned(), option_env!("HERONITE_GOOGLE_CLIENT_SECRET").unwrap_or_default().to_owned()))
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
    fn settings_of_the_switches_load_as_modes() {
        let old = r#"{"enabled":true,"backups":false,"saves":true,"provider":"drive","connected":"drive","deviceId":"a"}"#;
        let p: SyncPrefs = serde_json::from_str(old).unwrap();
        assert_eq!((p.library, p.backups, p.saves, p.captures), (Mode::Auto, Mode::Off, Mode::Auto, Mode::Off));
        let new: SyncPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(new, p);
        assert_eq!(serde_json::to_value(Mode::Manual).unwrap(), "manual");
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
