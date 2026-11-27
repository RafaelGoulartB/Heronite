//! The system media transport controls (`heronite_platform::media`): every
//! app that shows in Windows' volume flyout (Spotify, Apple Music, a
//! browser's tab playing media). Their thumbnails come as bytes: each is
//! kept as a file in the art folder, so the panel loads it like a cover.

use std::path::Path;

use heronite_platform::media::{self as native, Command, MediaSession};

use super::{art_file, base_id, keep_art, unique_ids, Action, Art, Playback, Player};

/// The players worth showing (see [`super::shown`]); Windows' current
/// session counts as the one used last. `None` when the media controls
/// cannot be asked.
pub fn read(art_dir: &Path) -> Option<Vec<Player>> {
    let key = |s: &MediaSession| format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", s.app_id, s.title, s.artist, s.album);
    let mut sessions = native::sessions(|s| !s.title.is_empty() && !art_file(art_dir, &key(s)).exists())
        .map_err(|e| log::debug!("media: reading the media controls: {e}"))
        .ok()?;
    // Stable: the current one leads, the others keep Windows' order.
    sessions.sort_by_key(|s| !s.current);
    let current = sessions.first().is_some_and(|s| s.current);
    let mut players: Vec<Player> = sessions
        .into_iter()
        .map(|s| {
            let file = art_file(art_dir, &key(&s));
            if let Some(bytes) = &s.thumbnail {
                if let Err(e) = keep_art(art_dir, &file, bytes) {
                    log::debug!("media: keeping a thumbnail: {e}");
                }
            }
            let art = (!s.title.is_empty() && file.exists()).then_some(Art::File(file));
            let app = if s.app_name.is_empty() { super::app_name_of_id(&s.app_id) } else { s.app_name };
            Player {
                id: s.app_id,
                app,
                title: s.title,
                artist: s.artist,
                album: s.album,
                playback: match s.playback {
                    native::Playback::Playing => Playback::Playing,
                    native::Playback::Paused => Playback::Paused,
                    native::Playback::Stopped => Playback::Stopped,
                },
                can_play_pause: s.can_play_pause,
                can_next: s.can_next,
                can_previous: s.can_previous,
                art,
            }
        })
        .collect();
    unique_ids(&mut players);
    let latest = current.then(|| players[0].id.clone());
    Some(super::shown(players, latest.as_deref()))
}

/// Sends `action` to the player's session. Blocking.
pub fn control(player: &Player, action: Action) -> Result<(), String> {
    let command = match action {
        Action::PlayPause => Command::PlayPause,
        Action::Next => Command::Next,
        Action::Previous => Command::Previous,
    };
    match native::send(base_id(&player.id), &player.title, command) {
        Ok(true) => Ok(()),
        Ok(false) => Err("the player did not take it".into()),
        Err(e) => Err(e.to_string()),
    }
}
