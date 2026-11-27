//! MPRIS (`org.mpris.MediaPlayer2.*` on the session bus): every player that
//! registers one (Spotify, mpv, a browser's tab playing media, KDE Connect's
//! phone) gives its name, what it plays and its controls. In the Flatpak the
//! manifest lets the app talk to those names.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use base64::Engine as _;

use super::{art_file, keep_art, Action, Art, Playback, Player};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// A player that does not answer (a hung browser) holds a read this long at
/// most per call.
const TIMEOUT: Duration = Duration::from_millis(800);
/// A `data:` cover bigger than this is left out.
const DATA_ART_MAX: usize = 8 << 20;

type Props = HashMap<String, OwnedValue>;

/// The session bus, kept between reads (the panel reads every second or two
/// while it is open); dropped after an error so the next read connects again.
static BUS: Mutex<Option<Connection>> = Mutex::new(None);

fn bus() -> zbus::Result<Connection> {
    let mut bus = BUS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(c) = bus.as_ref() {
        return Ok(c.clone());
    }
    let c = zbus::blocking::connection::Builder::session()?.method_timeout(TIMEOUT).build()?;
    *bus = Some(c.clone());
    Ok(c)
}

fn forget_bus() {
    *BUS.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// The players worth showing (see [`super::shown`]); `None` without a
/// session bus. A cover sent as a `data:` URI is kept in `art_dir`.
pub fn read(art_dir: &Path) -> Option<Vec<Player>> {
    let conn = match bus() {
        Ok(c) => c,
        Err(e) => {
            log::debug!("media: no session bus: {e}");
            return None;
        }
    };
    let names: Vec<String> = match conn
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .and_then(|reply| reply.body().deserialize())
    {
        Ok(names) => names,
        Err(e) => {
            log::debug!("media: listing the bus names: {e}");
            forget_bus();
            return None;
        }
    };
    let players = player_names(names)
        .into_iter()
        .filter_map(|name| {
            let player = get_all(&conn, &name, PLAYER).map_err(|e| log::debug!("media: {name}: {e}")).ok()?;
            let app = get_all(&conn, &name, "org.mpris.MediaPlayer2").unwrap_or_default();
            Some(player_of(name, &app, &player, art_dir))
        })
        .collect();
    Some(super::shown(players))
}

/// The MPRIS names, sorted (each kind of player keeps its place between
/// reads), without playerctld's: it stands for another player already listed.
fn player_names(names: Vec<String>) -> Vec<String> {
    let mut names: Vec<String> =
        names.into_iter().filter(|n| n.starts_with(PREFIX) && n.as_str() != "org.mpris.MediaPlayer2.playerctld").collect();
    names.sort();
    names
}

fn get_all(conn: &Connection, name: &str, interface: &str) -> zbus::Result<Props> {
    conn.call_method(Some(name), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &(interface,))?.body().deserialize()
}

/// A property as `T`. The values of a dictionary inside a variant
/// (`Metadata`) come wrapped in a variant of their own: unwrapped first.
fn prop<T: TryFrom<OwnedValue>>(props: &Props, key: &str) -> Option<T> {
    let mut v: &Value = props.get(key)?;
    while let Value::Value(inner) = v {
        v = inner;
    }
    T::try_from(v.try_to_owned().ok()?).ok()
}

/// A player from its two interfaces' properties: `app` is
/// `org.mpris.MediaPlayer2`'s (`Identity`), `player` the `Player` one's.
fn player_of(id: String, app: &Props, player: &Props, art_dir: &Path) -> Player {
    let meta: Props = prop(player, "Metadata").unwrap_or_default();
    let text = |key: &str| prop::<String>(&meta, key).map(|s| s.trim().to_string()).unwrap_or_default();
    // A list by the spec; a few players send one string.
    let artist = prop::<Vec<String>>(&meta, "xesam:artist").map(|a| a.join(", ")).unwrap_or_else(|| text("xesam:artist"));
    let playback = match prop::<String>(player, "PlaybackStatus").as_deref() {
        Some("Playing") => Playback::Playing,
        Some("Paused") => Playback::Paused,
        _ => Playback::Stopped,
    };
    let can = |key: &str| prop::<bool>(player, key).unwrap_or(false);
    // A player that cannot be controlled takes no command at all.
    let control = prop::<bool>(player, "CanControl").unwrap_or(true);
    let app_name = prop::<String>(app, "Identity").filter(|s| !s.trim().is_empty()).unwrap_or_else(|| name_of_bus(&id));
    Player {
        app: app_name,
        title: text("xesam:title"),
        artist: artist.trim().to_string(),
        album: text("xesam:album"),
        playback,
        can_play_pause: control && (can("CanPlay") || can("CanPause")),
        can_next: control && can("CanGoNext"),
        can_previous: control && can("CanGoPrevious"),
        art: art_of(&text("mpris:artUrl"), art_dir),
        id,
    }
}

/// "org.mpris.MediaPlayer2.chromium.instance123" → "chromium", for a player
/// with no `Identity`.
fn name_of_bus(id: &str) -> String {
    id.strip_prefix(PREFIX).unwrap_or(id).split('.').next().unwrap_or_default().to_string()
}

/// `mpris:artUrl`: a local file (browsers write the page's artwork to a
/// temporary file), an `http(s)` address (Spotify's covers) or a `data:`
/// picture (mpv's embedded covers), kept as a file in `art_dir`.
fn art_of(url: &str, art_dir: &Path) -> Option<Art> {
    if let Some(data) = url.strip_prefix("data:") {
        return data_art(data, art_dir);
    }
    if let Some(rest) = url.strip_prefix("file://") {
        // `file:///tmp/x.png`, or with a host: `file://localhost/tmp/x.png`.
        let path = rest.strip_prefix("localhost").unwrap_or(rest);
        if path.starts_with('/') {
            return Some(Art::File(heronite_data::paths::percent_decode(path).into()));
        }
        return None;
    }
    (url.starts_with("https://") || url.starts_with("http://")).then(|| Art::Url(url.to_string()))
}

/// A base64 `data:` picture as a file of `dir`, written the first time.
fn data_art(data: &str, dir: &Path) -> Option<Art> {
    let (head, body) = data.split_once(',')?;
    if !head.starts_with("image/") || !head.ends_with(";base64") {
        return None;
    }
    let file = art_file(dir, body);
    if !file.exists() {
        let bytes =
            base64::engine::general_purpose::STANDARD.decode(body.trim()).ok().filter(|b| !b.is_empty() && b.len() <= DATA_ART_MAX)?;
        if let Err(e) = keep_art(dir, &file, &bytes) {
            log::debug!("media: keeping a cover: {e}");
            return None;
        }
    }
    Some(Art::File(file))
}

/// Sends `action` to the player. Blocking.
pub fn control(player: &Player, action: Action) -> Result<(), String> {
    let method = match action {
        Action::PlayPause => "PlayPause",
        Action::Next => "Next",
        Action::Previous => "Previous",
    };
    let conn = bus().map_err(|e| e.to_string())?;
    conn.call_method(Some(player.id.as_str()), PATH, Some(PLAYER), method, &()).map(|_| ()).map_err(|e| {
        if matches!(e, zbus::Error::InputOutput(_)) {
            forget_bus();
        }
        format!("{}: {e}", player.app)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(v: Value<'_>) -> OwnedValue {
        v.try_to_owned().unwrap()
    }

    fn props(entries: Vec<(&str, Value<'static>)>) -> Props {
        entries.into_iter().map(|(k, v)| (k.to_string(), owned(v))).collect()
    }

    fn metadata(entries: Vec<(&str, Value<'static>)>) -> Value<'static> {
        let map: HashMap<String, Value<'static>> = entries.into_iter().map(|(k, v)| (k.to_string(), Value::new(v))).collect();
        Value::from(map)
    }

    #[test]
    fn reads_a_player() {
        let meta = metadata(vec![
            ("xesam:title", Value::from("Hollow Knight")),
            ("xesam:artist", Value::from(vec!["Christopher Larkin", "Team Cherry"])),
            ("xesam:album", Value::from("Hollow Knight (Original Soundtrack)")),
            ("mpris:artUrl", Value::from("file:///tmp/firefox-mpris/1_2%20x.png")),
        ]);
        let player = props(vec![
            ("PlaybackStatus", Value::from("Playing")),
            ("Metadata", meta),
            ("CanControl", Value::from(true)),
            ("CanPlay", Value::from(true)),
            ("CanPause", Value::from(true)),
            ("CanGoNext", Value::from(true)),
            ("CanGoPrevious", Value::from(false)),
        ]);
        let app = props(vec![("Identity", Value::from("Mozilla Firefox"))]);
        let p = player_of("org.mpris.MediaPlayer2.firefox.instance_1_42".into(), &app, &player, Path::new("/nowhere"));
        assert_eq!(p.app, "Mozilla Firefox");
        assert_eq!(p.title, "Hollow Knight");
        assert_eq!(p.artist, "Christopher Larkin, Team Cherry");
        assert_eq!(p.album, "Hollow Knight (Original Soundtrack)");
        assert_eq!(p.playback, Playback::Playing);
        assert!(p.can_play_pause && p.can_next && !p.can_previous);
        assert_eq!(p.art, Some(Art::File("/tmp/firefox-mpris/1_2 x.png".into())));
    }

    #[test]
    fn a_player_out_of_control_takes_no_command() {
        let player = props(vec![
            ("PlaybackStatus", Value::from("Paused")),
            ("CanControl", Value::from(false)),
            ("CanPlay", Value::from(true)),
            ("CanGoNext", Value::from(true)),
        ]);
        let p = player_of("org.mpris.MediaPlayer2.kdeconnect.mpris_phone".into(), &Props::new(), &player, Path::new("/nowhere"));
        assert_eq!(p.playback, Playback::Paused);
        assert!(!p.can_play_pause && !p.can_next && !p.can_previous);
        assert_eq!(p.app, "kdeconnect", "no Identity: the bus name's");
        assert_eq!(p.title, "");
    }

    #[test]
    fn one_artist_as_a_string() {
        let meta = metadata(vec![("xesam:artist", Value::from(" Lena Raine "))]);
        let p = player_of("org.mpris.MediaPlayer2.mpv".into(), &Props::new(), &props(vec![("Metadata", meta)]), Path::new("/nowhere"));
        assert_eq!(p.artist, "Lena Raine");
        assert_eq!(p.playback, Playback::Stopped, "no status: stopped");
    }

    #[test]
    fn art_addresses() {
        let dir = Path::new("/nowhere");
        assert_eq!(art_of("https://i.scdn.co/image/ab67", dir), Some(Art::Url("https://i.scdn.co/image/ab67".into())));
        assert_eq!(art_of("file://localhost/home/me/c%C3%A9u.jpg", dir), Some(Art::File("/home/me/céu.jpg".into())));
        assert_eq!(art_of("file://server/share/x.png", dir), None);
        assert_eq!(art_of("ftp://example.com/x.png", dir), None);
        assert_eq!(art_of("HTTPS://example.com/x.png", dir), None, "the images fetcher takes lowercase schemes only");
        assert_eq!(art_of("", dir), None);
    }

    #[test]
    fn data_covers_become_files() {
        let dir = tempfile::tempdir().unwrap();
        let url = "data:image/png;base64,iVBORw0KGgo=";
        let Some(Art::File(file)) = art_of(url, dir.path()) else { panic!("a file") };
        assert_eq!(std::fs::read(&file).unwrap(), b"\x89PNG\r\n\x1a\n");
        assert_eq!(art_of(url, dir.path()), Some(Art::File(file)), "the same file again");
        assert_eq!(art_of("data:text/plain;base64,aGk=", dir.path()), None);
        assert_eq!(art_of("data:image/png,raw", dir.path()), None);
        assert_eq!(art_of("data:image/png;base64,!!!", dir.path()), None);
    }

    #[test]
    fn player_names_without_playerctld() {
        let names = vec![
            "org.freedesktop.Notifications".to_string(),
            "org.mpris.MediaPlayer2.spotify".into(),
            "org.mpris.MediaPlayer2.playerctld".into(),
            "org.mpris.MediaPlayer2.chromium.instance7".into(),
            ":1.42".into(),
        ];
        assert_eq!(player_names(names), ["org.mpris.MediaPlayer2.chromium.instance7", "org.mpris.MediaPlayer2.spotify"]);
        assert_eq!(name_of_bus("org.mpris.MediaPlayer2.chromium.instance7"), "chromium");
    }
}
