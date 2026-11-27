//! The media players playing now (a music app, a video in a browser tab) and
//! their play / pause, next and previous, for the overlay. Linux asks every
//! MPRIS player on the session bus (what the desktop's media widgets show),
//! Windows the system media transport controls (what its volume flyout
//! shows). Blocking: call from a worker, not the UI.

use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{control, read};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{control, read};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    Playing,
    Paused,
    Stopped,
}

/// A player's picture of what it plays (the album's cover, a video's
/// thumbnail).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Art {
    File(PathBuf),
    /// `http(s)`: fetched like the other pictures (public addresses only).
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    /// What finds it again for [`control`]: the MPRIS bus name on Linux, the
    /// session's app id on Windows.
    pub id: String,
    /// The app's name ("Spotify", "Firefox"); empty when it gives none.
    pub app: String,
    /// The track or video; empty when the player gives none.
    pub title: String,
    /// The artists joined by ", " (a video's channel); may be empty.
    pub artist: String,
    pub album: String,
    pub playback: Playback,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub art: Option<Art>,
}

impl Player {
    /// A browser plays it (a video, a web player).
    pub fn is_browser(&self) -> bool {
        const BROWSERS: [&str; 12] =
            ["chrome", "chromium", "firefox", "zen", "brave", "vivaldi", "edge", "librewolf", "opera", "floorp", "waterfox", "msedge"];
        let name = format!("{} {}", self.app, self.id).to_lowercase();
        BROWSERS.iter().any(|b| name.contains(b))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    PlayPause,
    Next,
    Previous,
}

/// Without MPRIS or the media controls: nothing to show.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn read(_art_dir: &Path) -> Option<Vec<Player>> {
    None
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn control(_player: &Player, _action: Action) -> Result<(), String> {
    Err("media controls are not available on this system".into())
}

/// The players worth showing, in the order the overlay offers them: the ones
/// playing, then paused, then stopped (each kind in the order found). A
/// stopped player with nothing loaded (a browser that played once) and one
/// that takes no commands are left out.
pub(crate) fn shown(players: Vec<Player>) -> Vec<Player> {
    let mut players: Vec<Player> = players
        .into_iter()
        .filter(|p| !(p.playback == Playback::Stopped && p.title.is_empty()))
        .filter(|p| p.can_play_pause || p.can_next || p.can_previous)
        .collect();
    players.sort_by_key(|p| match p.playback {
        Playback::Playing => 0,
        Playback::Paused => 1,
        Playback::Stopped => 2,
    });
    players
}

/// A name for a Windows media session's app when Windows knows none (a
/// desktop program's AUMID): "Spotify.exe" → "Spotify", the browsers' own
/// ids by their names. An id that is only a hash (Firefox and its forks name
/// themselves by their install folder's) gives none.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn app_name_of_id(id: &str) -> String {
    // A packaged app: "Publisher.App_hash!Entry"; a program: maybe a path.
    let name = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let name = match name.split_once('!') {
        Some((family, _)) => family.split('_').next().unwrap_or(family).rsplit('.').next().unwrap_or(family),
        None => name,
    };
    let name = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(name);
    let known = match name.to_ascii_lowercase().as_str() {
        "chrome" => "Google Chrome",
        "msedge" => "Microsoft Edge",
        "brave" => "Brave",
        "opera" => "Opera",
        "vivaldi" => "Vivaldi",
        "spotify" => "Spotify",
        "vlc" => "VLC",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    if name.len() >= 12 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
        return String::new();
    }
    name.to_string()
}

/// Between an id and the number [`unique_ids`] gives it.
const ID_NUMBER: char = '\u{1f}';

/// Numbers the players after the first that share an id (on Windows a
/// browser's tabs are a session each, all under the browser's id), so the
/// panel tells them apart.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn unique_ids(players: &mut [Player]) {
    let mut seen = std::collections::HashMap::new();
    for p in players {
        let n = seen.entry(p.id.clone()).or_insert(0usize);
        if *n > 0 {
            p.id = format!("{}{ID_NUMBER}{n}", p.id);
        }
        *n += 1;
    }
}

/// The id a player had before [`unique_ids`].
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn base_id(id: &str) -> &str {
    id.split(ID_NUMBER).next().unwrap_or(id)
}

/// Covers kept in the art folder; the oldest go first.
const KEEP_ART: usize = 24;

/// The file a cover that comes as bytes (Windows' thumbnails, a `data:`
/// URI) is kept in, inside the art folder `dir`, by what identifies it.
pub(crate) fn art_file(dir: &Path, key: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    dir.join(format!("{:016x}.img", h.finish()))
}

/// Writes a cover to `file` (an [`art_file`] of `dir`) and leaves the
/// newest [`KEEP_ART`] covers there.
pub(crate) fn keep_art(dir: &Path, file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    heronite_data::temps::write_atomic(file, bytes, None)?;
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    let mut covers: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "img"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if covers.len() > KEEP_ART {
        covers.sort_by_key(|c| std::cmp::Reverse(c.0));
        for (_, path) in covers.into_iter().skip(KEEP_ART) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn player(id: &str, playback: Playback, title: &str) -> Player {
        Player {
            id: id.into(),
            app: String::new(),
            title: title.into(),
            artist: String::new(),
            album: String::new(),
            playback,
            can_play_pause: true,
            can_next: true,
            can_previous: true,
            art: None,
        }
    }

    #[test]
    fn playing_first_and_idle_players_left_out() {
        let mut deaf = player("deaf", Playback::Playing, "Song");
        (deaf.can_play_pause, deaf.can_next, deaf.can_previous) = (false, false, false);
        let list = shown(vec![
            player("paused", Playback::Paused, "A"),
            player("idle", Playback::Stopped, ""),
            player("stopped", Playback::Stopped, "B"),
            player("playing", Playback::Playing, "C"),
            deaf,
            player("playing-2", Playback::Playing, ""),
        ]);
        let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["playing", "playing-2", "paused", "stopped"]);
    }

    #[test]
    fn browsers_by_app_or_bus_name() {
        let mut p = player("org.mpris.MediaPlayer2.chromium.instance42", Playback::Playing, "Video");
        assert!(p.is_browser());
        p.id = "org.mpris.MediaPlayer2.spotify".into();
        p.app = "Spotify".into();
        assert!(!p.is_browser());
        p.app = "Mozilla Firefox".into();
        assert!(p.is_browser());
    }

    #[test]
    fn windows_app_names() {
        assert_eq!(app_name_of_id("Spotify.exe"), "Spotify");
        assert_eq!(app_name_of_id("Chrome"), "Google Chrome");
        assert_eq!(app_name_of_id("MSEdge"), "Microsoft Edge");
        assert_eq!(app_name_of_id("308046B0AF4A39CB"), "", "Firefox's hash");
        assert_eq!(app_name_of_id("C:\\Program Files\\foobar2000\\foobar2000.exe"), "foobar2000");
        assert_eq!(app_name_of_id("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "SpotifyMusic");
        assert_eq!(app_name_of_id("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "ZuneMusic");
    }

    #[test]
    fn players_sharing_an_id_are_told_apart() {
        let mut list = vec![
            player("Chrome", Playback::Playing, "A"),
            player("Spotify.exe", Playback::Paused, "B"),
            player("Chrome", Playback::Paused, "C"),
        ];
        unique_ids(&mut list);
        assert_eq!(list[0].id, "Chrome");
        assert_eq!(list[1].id, "Spotify.exe");
        assert_ne!(list[2].id, "Chrome");
        assert_eq!(base_id(&list[2].id), "Chrome");
        assert_eq!(base_id("Spotify.exe"), "Spotify.exe");
    }

    #[test]
    fn keeps_the_newest_covers() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "kept").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for i in 0..KEEP_ART {
            let file = art_file(dir.path(), &i.to_string());
            std::fs::write(&file, [1]).unwrap();
            std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        }
        let new = art_file(dir.path(), "new");
        keep_art(dir.path(), &new, &[7, 7]).unwrap();
        assert_eq!(std::fs::read(&new).unwrap(), [7, 7]);
        let covers = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "img")).count();
        assert_eq!(covers, KEEP_ART, "one old cover went");
        assert!(other.exists(), "only covers are removed");
    }

    #[test]
    fn art_files_by_key() {
        let dir = Path::new("/cache/art");
        assert_eq!(art_file(dir, "a"), art_file(dir, "a"));
        assert_ne!(art_file(dir, "a"), art_file(dir, "b"));
        assert!(art_file(dir, "a").starts_with(dir));
    }
}
