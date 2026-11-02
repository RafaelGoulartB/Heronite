//! The library synced between PCs (Settings → Sync): the same Collection on
//! a dual boot's Linux and Windows, or on several computers, whatever stores
//! each one is signed in to.
//!
//! Every PC writes only its own file, `devices/<id>/state.json`, in a place
//! all of them reach ([`transport`]: a folder, or a cloud account through
//! [`rclone`]), and reads the others'. Two PCs never write the same file, so
//! nothing a sync service copies can conflict. Each file holds all its PC
//! has to say ([`format`]), and [`merge`] folds them into this PC's
//! Collection by rules that lose nothing; [`ledger`] remembers what was
//! published and taken in.
//!
//! What syncs: completion status, favourite, hidden, rating, notes, play
//! time and play sessions. What does not: installs, Wine prefixes, game
//! settings, store sign-ins.
//!
//! The same account takes the backups, the game saves and the screenshots
//! too ([`cloud`]).
//!
//! All functions block; the app runs them off the UI thread.

pub mod cloud;
pub mod format;
pub mod ledger;
pub mod local;
pub mod merge;
pub mod prefs;
pub mod rclone;
pub mod sent;
pub mod transport;
pub mod usage;

use std::fs;
use std::sync::Mutex;

use heronite_data::proc::Cancel;
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::tr;

pub use merge::Summary;
pub use prefs::{Mode, Provider, SeenDevice, SyncPrefs};
pub use rclone::Server;

use crate::format::DeviceInfo;
use crate::prefs::LastSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// No place chosen, or the sync is off.
    Off,
    /// A sync is running already.
    Busy,
    Cancelled,
    /// The sync folder is not there (a drive not mounted).
    FolderMissing(String),
    /// The sync folder's drive is mounted read-only.
    ReadOnly(String),
    Write(String),
    NoRclone(String),
    /// What rclone said.
    Cloud(String),
    /// The account takes no more requests for now (too many a minute).
    RateLimited,
    /// Another sign-in waits for the browser (rclone's page is taken).
    SignInOpen,
    /// Reading or writing the Collection.
    Data(String),
}

impl SyncError {
    /// What rclone said, told apart when it is a full request quota.
    pub fn from_cloud(error: String) -> SyncError {
        if rclone::is_rate_limited(&error) {
            SyncError::RateLimited
        } else {
            SyncError::Cloud(error)
        }
    }

    /// In the interface language.
    pub fn localized(&self) -> String {
        match self {
            SyncError::Off => tr!("Choose where to sync first.").into(),
            SyncError::Busy => tr!("A sync is already running.").into(),
            SyncError::Cancelled => tr!("Cancelled.").into(),
            SyncError::FolderMissing(folder) => tr!("The sync folder {folder} is not there. Is its drive connected?", folder),
            SyncError::ReadOnly(_) => tr!(
                "The sync folder is read-only. If it is on a Windows drive, turn off Fast Startup in Windows and shut Windows down fully."
            )
            .into(),
            SyncError::Write(error) => tr!("Could not write to the sync folder: {error}", error),
            SyncError::NoRclone(error) => tr!("Could not get rclone: {error}", error),
            SyncError::Cloud(error) => tr!("The account did not answer: {error}", error),
            SyncError::SignInOpen => tr!(
                "Another sign-in is still waiting in the browser (another Heronite window, or rclone). Finish or close it, then connect again."
            )
            .into(),
            SyncError::RateLimited => {
                tr!("The account takes no more requests for now (too many a minute). Heronite tries again by itself in a few minutes.")
                    .into()
            }
            SyncError::Data(error) => tr!("Could not update the Collection: {error}", error),
        }
    }
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub summary: Summary,
    /// The other PCs whose files were read.
    pub devices: Vec<SeenDevice>,
}

/// One sync at a time per process.
static BUSY: Mutex<()> = Mutex::new(());

/// Syncs now: reads the other PCs' files, brings what is new into the
/// Collection and publishes this PC's file. The result (or the error) is
/// kept in `sync.json` for Settings → Sync.
pub fn run(data: &DataPaths, app: &AppPaths, manual: bool) -> Result<Report, SyncError> {
    let Ok(_guard) = BUSY.try_lock() else { return Err(SyncError::Busy) };
    let result = run_locked(data, app, manual);
    let now = now();
    let saved = prefs::update(app, |p| match &result {
        Ok(report) => {
            p.last_sync_at = Some(now.clone());
            p.last_error = None;
            let s = &report.summary;
            p.last_summary = Some(LastSummary { fields: s.fields, sessions: s.sessions, playtime: s.playtime, waiting: s.waiting });
            p.devices.clone_from(&report.devices);
        }
        Err(SyncError::Off | SyncError::Busy) => {}
        Err(e) => p.last_error = Some(e.localized()),
    });
    if let Err(e) = saved {
        log::warn!("sync: saving its state: {e:#}");
    }
    result
}

fn run_locked(data: &DataPaths, app: &AppPaths, manual: bool) -> Result<Report, SyncError> {
    let data_error = |e: anyhow::Error| SyncError::Data(format!("{e:#}"));
    let prefs = prefs::load(app).map_err(data_error)?;
    if !prefs.library.allows(manual) || !prefs.ready() {
        return Err(SyncError::Off);
    }
    let place = transport::place(data, app, &prefs)?;
    let remotes = place.pull(app)?;
    let local = local::read(data).map_err(data_error)?;
    let ledger = ledger::load(data).unwrap_or_else(|e| {
        log::warn!("sync: the ledger could not be read, starting over: {e:#}");
        Default::default()
    });
    let me = DeviceInfo {
        id: prefs.device_id.clone(),
        name: prefs.shown_name(),
        os: std::env::consts::OS.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
    };
    let outcome = merge::merge(&local, ledger, &remotes, &me, &now(), !prefs.play_only);
    if !outcome.changes.is_empty() {
        keep_copies(data);
        local::write(data, &outcome.changes).map_err(data_error)?;
    }
    ledger::save(data, &outcome.ledger).map_err(data_error)?;
    place.push(app, &outcome.own)?;
    let s = &outcome.summary;
    log::info!("sync: {} fields, {} sessions, {} play times from {} other PCs", s.fields, s.sessions, s.playtime, remotes.len());
    let devices = remotes
        .iter()
        .filter(|r| r.device.id != me.id)
        .map(|r| SeenDevice {
            id: r.device.id.clone(),
            name: r.device.name.clone(),
            os: r.device.os.clone(),
            written_at: r.written_at.clone(),
        })
        .collect();
    Ok(Report { summary: outcome.summary, devices })
}

/// Copies the files a sync may change into `local_library/sync-before`
/// (replacing the copies of the previous sync), so its changes can be undone
/// by hand.
fn keep_copies(data: &DataPaths) {
    let dir = data.sync_before_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        log::warn!("sync: {}: {e}", dir.display());
        return;
    }
    let files = [
        data.local_meta(),
        data.sessions(),
        data.ratings(),
        data.metadata(),
        data.statuses(),
        data.status_history(),
        data.timestamps(),
        data.store_config(),
    ];
    for file in files {
        let (Some(name), Some(parent)) = (file.file_name(), file.parent().and_then(|p| p.file_name())) else { continue };
        let copy = dir.join(format!("{}-{}", parent.to_string_lossy(), name.to_string_lossy()));
        if file.is_file() {
            if let Err(e) = fs::copy(&file, &copy) {
                log::warn!("sync: copying {}: {e}", file.display());
            }
        }
    }
}

/// Connects the cloud account or server of `provider` (rclone downloaded
/// first when needed): the browser opens for a cloud account, and `on_link`
/// gets its page. Turns the sync on once it works.
pub fn connect(
    data: &DataPaths,
    app: &AppPaths,
    provider: Provider,
    server: &Server,
    cancel: &Cancel,
    on_link: &mut dyn FnMut(&str),
) -> Result<(), SyncError> {
    let bin = rclone::ensure(data, &|| cancel.is_requested()).map_err(SyncError::NoRclone)?;
    let conf = app.rclone_conf();
    if let Some(dir) = conf.parent() {
        fs::create_dir_all(dir).map_err(|e| SyncError::Data(e.to_string()))?;
    }
    let prefs = prefs::load(app).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    let rc = rclone::Rclone { bin, config: Some(conf.clone()) };
    let mut server = server.clone();
    if let (Provider::Drive, Some((id, secret))) = (provider, prefs.google_app()) {
        server.client_id = id;
        server.client_secret = secret;
    }
    rc.connect(provider, &server, &prefs.remote_folder, cancel, on_link).map_err(|e| match e {
        rclone::RcloneError::Cancelled => SyncError::Cancelled,
        // rclone's sign-in page has one address: a sign-in still open holds it.
        other if other.to_string().contains("address already in use") => SyncError::SignInOpen,
        other => SyncError::from_cloud(other.to_string()),
    })?;
    if let Err(e) = heronite_platform::fs::make_private(&conf) {
        log::warn!("sync: {}: {e}", conf.display());
    }
    prefs::update(app, |p| {
        p.provider = provider;
        p.connected = Some(provider);
        p.account = if provider.is_server() { server.shown() } else { String::new() };
        // Another sign-in: nothing is known to be there yet.
        p.connection = heronite_data::sideload::generate_app_name();
        p.uploads.clear();
        p.last_sync_at = None;
        p.last_summary = None;
        p.devices.clear();
        if !p.library.is_on() {
            p.library = Mode::Auto;
        }
        p.last_error = None;
    })
    .map_err(|e| SyncError::Data(format!("{e:#}")))?;
    Ok(())
}

/// Signs this PC out of the connected account (its token is deleted).
pub fn disconnect(data: &DataPaths, app: &AppPaths) -> Result<(), SyncError> {
    if let Some(bin) = rclone::find(data) {
        let rc = rclone::Rclone { bin, config: Some(app.rclone_conf()) };
        if let Err(e) = rc.disconnect() {
            log::warn!("sync: removing the account: {e}");
        }
    }
    prefs::update(app, |p| {
        p.connected = None;
        p.account.clear();
        p.connection.clear();
        p.uploads.clear();
    })
    .map_err(|e| SyncError::Data(format!("{e:#}")))?;
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
