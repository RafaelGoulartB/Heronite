//! The media players playing now (a music app, a video in a browser tab) and
//! their play / pause, next and previous, for the overlay. Linux asks every
//! MPRIS player on the session bus (what the desktop's media widgets show),
//! Windows the system media transport controls (what its volume flyout
//! shows). Blocking: call from a worker, not the UI.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

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

/// The players used last, for the app's life (the panel is made again each
/// time it opens).
static RECENT: Mutex<Recent> = Mutex::new(Recent::new());
/// Ids [`Recent`] keeps.
const RECENT_MAX: usize = 16;

/// The players used last, the latest first: one that took a command from
/// the overlay, started playing or paused. What each was doing at the last
/// read tells the last two.
struct Recent {
    ids: Vec<String>,
    seen: Vec<(String, Playback)>,
    /// The system's choice at the last read.
    latest: Option<String>,
}

impl Recent {
    const fn new() -> Self {
        Recent { ids: Vec::new(), seen: Vec::new(), latest: None }
    }

    fn used(&mut self, id: &str) {
        self.ids.retain(|i| i != id);
        self.ids.insert(0, id.to_string());
        self.ids.truncate(RECENT_MAX);
    }

    /// Notes the players that started playing or paused since the last read
    /// (a new one only when it plays), then `latest`, the one the system
    /// says was used last (Windows' current session), when that changed: it
    /// may keep its choice while the overlay's commands go to others.
    fn read(&mut self, players: &[Player], latest: Option<&str>) {
        // Backwards: of the ones noted together, the first found leads.
        for p in players.iter().rev() {
            let changed = match self.seen.iter().find(|(id, _)| id == &p.id) {
                Some((_, before)) => *before != p.playback && p.playback != Playback::Stopped,
                None => p.playback == Playback::Playing,
            };
            if changed {
                self.used(&p.id);
            }
        }
        self.seen = players.iter().map(|p| (p.id.clone(), p.playback)).collect();
        if let Some(id) = latest.filter(|id| self.latest.as_deref() != Some(*id)) {
            self.used(id);
        }
        self.latest = latest.map(str::to_string);
    }

    /// Where `id` is among the ones used last (after them all when it is not).
    fn rank(&self, id: &str) -> usize {
        self.ids.iter().position(|i| i == id).unwrap_or(usize::MAX)
    }
}

/// `player` was just sent a command from the overlay: it comes first among
/// the ones that play (or that do not) from now on.
pub fn used(player: &Player) {
    RECENT.lock().unwrap_or_else(PoisonError::into_inner).used(&player.id);
}

/// The players worth showing, in the order the overlay offers them (see
/// [`ordered`]), after noting which were used since the last read and
/// `latest`, the one the system says was used last.
pub(crate) fn shown(players: Vec<Player>, latest: Option<&str>) -> Vec<Player> {
    let mut recent = RECENT.lock().unwrap_or_else(PoisonError::into_inner);
    recent.read(&players, latest);
    ordered(players, &recent)
}

/// The ones playing first, then the others; within each, the one used last
/// first, then paused before stopped, then in the order found. A stopped
/// player with nothing loaded (a browser that played once) and one that
/// takes no commands are left out.
fn ordered(players: Vec<Player>, recent: &Recent) -> Vec<Player> {
    let mut players: Vec<Player> = players
        .into_iter()
        .filter(|p| !(p.playback == Playback::Stopped && p.title.is_empty()))
        .filter(|p| p.can_play_pause || p.can_next || p.can_previous)
        .collect();
    players.sort_by_key(|p| (p.playback != Playback::Playing, recent.rank(&p.id), p.playback == Playback::Stopped));
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

    fn ids(list: &[Player]) -> Vec<&str> {
        list.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn playing_first_and_idle_players_left_out() {
        let mut deaf = player("deaf", Playback::Playing, "Song");
        (deaf.can_play_pause, deaf.can_next, deaf.can_previous) = (false, false, false);
        let list = ordered(
            vec![
                player("paused", Playback::Paused, "A"),
                player("idle", Playback::Stopped, ""),
                player("stopped", Playback::Stopped, "B"),
                player("playing", Playback::Playing, "C"),
                deaf,
                player("playing-2", Playback::Playing, ""),
            ],
            &Recent::new(),
        );
        assert_eq!(ids(&list), ["playing", "playing-2", "paused", "stopped"]);
    }

    #[test]
    fn the_one_used_last_leads() {
        let mut recent = Recent::new();
        let first = vec![player("a", Playback::Playing, "A"), player("b", Playback::Paused, "B"), player("c", Playback::Paused, "C")];
        recent.read(&first, None);
        assert_eq!(ids(&ordered(first, &recent)), ["a", "b", "c"]);
        // c started playing: it leads a, which played before.
        let next = vec![player("a", Playback::Playing, "A"), player("b", Playback::Paused, "B"), player("c", Playback::Playing, "C")];
        recent.read(&next, None);
        assert_eq!(ids(&ordered(next.clone(), &recent)), ["c", "a", "b"]);
        // A command to b: still after the ones playing, but first of the others.
        recent.used("b");
        let all_paused: Vec<Player> = next.into_iter().map(|p| Player { playback: Playback::Paused, ..p }).collect();
        recent.read(&all_paused, None);
        // a and c paused just now (of the two, the first found leads); b's command came before.
        assert_eq!(ids(&ordered(all_paused.clone(), &recent)), ["a", "c", "b"]);
        // The system's own choice (Windows' current session) comes last, so it leads.
        recent.read(&all_paused, Some("b"));
        assert_eq!(ids(&ordered(all_paused.clone(), &recent)), ["b", "a", "c"]);
        // A command to c afterwards: the system keeping b does not take it back.
        recent.used("c");
        recent.read(&all_paused, Some("b"));
        assert_eq!(ids(&ordered(all_paused.clone(), &recent)), ["c", "b", "a"]);
        // Stopping on its own is no use.
        let a_stopped: Vec<Player> =
            all_paused.into_iter().map(|p| if p.id == "a" { Player { playback: Playback::Stopped, ..p } } else { p }).collect();
        recent.read(&a_stopped, Some("b"));
        assert_eq!(ids(&ordered(a_stopped, &recent)), ["c", "b", "a"]);
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
