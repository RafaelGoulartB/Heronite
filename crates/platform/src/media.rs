//! The system media transport controls (`Windows.Media.Control`): the media
//! sessions apps register (Spotify, a browser tab, Media Player), what each
//! plays, and its play / pause, next and previous, as the volume flyout
//! shows them. Blocking: every call waits on WinRT's async operations.

use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, PoisonError};

use windows::core::HSTRING;
use windows::ApplicationModel::AppInfo;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Properties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;

/// A thumbnail bigger than this is left out (they are a few hundred KiB).
const THUMBNAIL_MAX: u64 = 8 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaSession {
    /// The app's id (`SourceAppUserModelId`): "Spotify.exe", "Chrome", a
    /// packaged app's AUMID.
    pub app_id: String,
    /// The app's name as Windows shows it; empty when it knows none (most
    /// desktop programs).
    pub app_name: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playback: Playback,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
    /// The session the media keys drive now.
    pub current: bool,
    /// The thumbnail's bytes (PNG, JPEG...), when the caller asked for them.
    pub thumbnail: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
}

fn manager() -> io::Result<Manager> {
    crate::com::keep_mta();
    Manager::RequestAsync().and_then(|op| op.join()).map_err(io::Error::other)
}

/// Every media session. `want_thumbnail` says, for each one read, whether to
/// read its thumbnail too (the caller keeps the ones it has).
pub fn sessions(mut want_thumbnail: impl FnMut(&MediaSession) -> bool) -> io::Result<Vec<MediaSession>> {
    let manager = manager()?;
    // An error when no app plays anything.
    let current = manager.GetCurrentSession().ok();
    let list = manager.GetSessions().map_err(io::Error::other)?;
    let mut out = Vec::new();
    for session in &list {
        match read(&session) {
            Ok((mut s, props)) => {
                // The same object, or (if the manager hands out new ones) the same app.
                s.current = current
                    .as_ref()
                    .is_some_and(|c| c == &session || c.SourceAppUserModelId().is_ok_and(|id| id.to_string_lossy() == s.app_id));
                if want_thumbnail(&s) {
                    s.thumbnail = thumbnail(&props).unwrap_or_else(|e| {
                        log::debug!("media: thumbnail of {}: {e}", s.app_id);
                        None
                    });
                }
                out.push(s);
            }
            Err(e) => log::debug!("media: a session: {e}"),
        }
    }
    Ok(out)
}

fn read(session: &Session) -> windows::core::Result<(MediaSession, Properties)> {
    let app_id = session.SourceAppUserModelId()?.to_string_lossy();
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let text = |s: windows::core::Result<HSTRING>| s.map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default();
    let info = session.GetPlaybackInfo()?;
    let status = info.PlaybackStatus()?;
    // Changing: between two tracks, still playing.
    let playback = if status == Status::Playing || status == Status::Changing {
        Playback::Playing
    } else if status == Status::Paused {
        Playback::Paused
    } else {
        Playback::Stopped
    };
    let controls = info.Controls()?;
    let on = |c: windows::core::Result<bool>| c.unwrap_or(false);
    let can_play_pause = on(controls.IsPlayPauseToggleEnabled()) || on(controls.IsPlayEnabled()) || on(controls.IsPauseEnabled());
    let s = MediaSession {
        app_name: app_name(&app_id),
        title: text(props.Title()),
        artist: text(props.Artist()),
        album: text(props.AlbumTitle()),
        playback,
        can_play_pause,
        can_next: on(controls.IsNextEnabled()),
        can_previous: on(controls.IsPreviousEnabled()),
        current: false,
        thumbnail: None,
        app_id,
    };
    Ok((s, props))
}

/// A packaged app's name (Windows 10 2004 and later); desktop programs'
/// AUMIDs are not apps it knows. Asked once per app (the overlay reads the
/// sessions every couple of seconds).
fn app_name(app_id: &str) -> String {
    static NAMES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    names
        .get_or_insert_with(HashMap::new)
        .entry(app_id.to_string())
        .or_insert_with(|| {
            AppInfo::GetFromAppUserModelId(&HSTRING::from(app_id))
                .and_then(|info| info.DisplayInfo())
                .and_then(|d| d.DisplayName())
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        })
        .clone()
}

fn thumbnail(props: &Properties) -> windows::core::Result<Option<Vec<u8>>> {
    // An error when it has none.
    let Ok(reference) = props.Thumbnail() else { return Ok(None) };
    let stream = reference.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if size == 0 || size > THUMBNAIL_MAX {
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let read = reader.LoadAsync(size as u32)?.join()?;
    let mut bytes = vec![0; read as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(Some(bytes))
}

/// Sends `command` to the session of `app_id` playing `title` (a browser has
/// one per tab), else to the first one of `app_id`. `Ok(false)`: the app
/// did not take it.
pub fn send(app_id: &str, title: &str, command: Command) -> io::Result<bool> {
    let manager = manager()?;
    let list = manager.GetSessions().map_err(io::Error::other)?;
    let mine: Vec<Session> =
        (&list).into_iter().filter(|s| s.SourceAppUserModelId().is_ok_and(|id| id.to_string_lossy() == app_id)).collect();
    let same_title = |s: &&Session| {
        s.TryGetMediaPropertiesAsync().and_then(|op| op.join()).and_then(|p| p.Title()).is_ok_and(|t| t.to_string_lossy().trim() == title)
    };
    let Some(session) = mine.iter().find(same_title).or(mine.first()) else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "the player is gone"));
    };
    let op = match command {
        Command::PlayPause => {
            let info = session.GetPlaybackInfo().map_err(io::Error::other)?;
            let toggles = info.Controls().and_then(|c| c.IsPlayPauseToggleEnabled()).unwrap_or(false);
            let playing = info.PlaybackStatus().is_ok_and(|s| s == Status::Playing);
            if toggles {
                session.TryTogglePlayPauseAsync()
            } else if playing {
                session.TryPauseAsync()
            } else {
                session.TryPlayAsync()
            }
        }
        Command::Next => session.TrySkipNextAsync(),
        Command::Previous => session.TrySkipPreviousAsync(),
    };
    op.and_then(|op| op.join()).map_err(io::Error::other)
}
