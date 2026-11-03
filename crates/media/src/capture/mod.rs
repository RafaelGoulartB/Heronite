//! Screenshot capture while a game runs. Port of
//! `src/local-library/screenshots/capture.ts`.
//!
//! The portable pieces live here and are unit-tested: accelerator parsing and
//! translation (Hyprland `hl.bind` syntax, XDG GlobalShortcuts triggers,
//! Windows `RegisterHotKey` keys), file naming, destination folder selection,
//! monitor picking. The side-effecting service ([`CaptureService`]) lives in
//! `service.rs` on Linux and in `win/` on Windows (see there for how it
//! works); elsewhere `unsupported.rs` stands in for it and never captures.
//!
//! # How a capture happens (Linux)
//! 1. `begin_session` for a launched game registers the shortcut (only while a
//!    game is active and `screenshots.captureEnabled` is set).
//! 2. On Hyprland a temporary, in-memory, non-consuming bind
//!    (`hyprctl eval hl.bind(...)`) runs `kill -USR2 <our pid>`; a
//!    `signal-hook` thread turns SIGUSR2 into a capture. Elsewhere the XDG
//!    GlobalShortcuts portal (`ashpd`) delivers the activation.
//! 3. The display under the pointer is captured with `grim` (Hyprland) or the
//!    portal Screenshot method, written as PNG into the game's matched folder
//!    (or `<root>/<sanitized title>`), and a [`CaptureEvent::Saved`] is sent.

mod accelerator;

use crate::list::list_game_folders;
use crate::matching::{pick_screenshot_folder, MIN_SCORE};
use chrono::{Datelike, NaiveDateTime, Timelike};
use heronite_i18n::n_;
use std::path::{Path, PathBuf};

pub use accelerator::{
    hyprland_bind_expression, hyprland_bind_expression_as, hyprland_unbind_expression, parse_accelerator, to_hyprland_binding,
    to_portal_trigger, to_windows_hotkey, Accelerator, HyprlandBind, HyprlandBinding, WindowsHotkey, MOD_ALT, MOD_CTRL, MOD_SHIFT,
    MOD_SUPER, WIN_MOD_ALT, WIN_MOD_CONTROL, WIN_MOD_NOREPEAT, WIN_MOD_SHIFT, WIN_MOD_WIN,
};

#[cfg(target_os = "linux")]
mod feedback;
#[cfg(target_os = "linux")]
mod grim;
#[cfg(target_os = "linux")]
mod hotkey;
#[cfg(target_os = "linux")]
mod hyprland;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
mod service;
#[cfg(not(any(target_os = "linux", windows)))]
mod unsupported;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
pub use hotkey::GlobalHotkey;
#[cfg(target_os = "linux")]
pub use service::CaptureService;
#[cfg(not(any(target_os = "linux", windows)))]
pub use unsupported::{CaptureService, GlobalHotkey};
#[cfg(windows)]
pub use win::{CaptureService, GlobalHotkey};

/// Description used for the Hyprland bind and the portal shortcut.
pub const BIND_DESCRIPTION: &str = "Heronite game screenshot";

/// A monitor from `hyprctl -j monitors`, in logical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub focused: bool,
}

/// The monitor containing `(x, y)`, else the focused one, else the first.
pub fn monitor_under_pointer(monitors: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|m| x >= m.x && x < m.x + m.width && y >= m.y && y < m.y + m.height)
        .or_else(|| monitors.iter().find(|m| m.focused))
        .or_else(|| monitors.first())
}

/// `timestampName`: `YYYY-MM-DD_HH-MM-SS-mmm.png` (local time).
pub fn timestamp_name(now: NaiveDateTime) -> String {
    format!("{}.png", timestamp_stem(now))
}

/// `YYYY-MM-DD_HH-MM-SS-mmm` (local time), the name of every capture.
fn timestamp_stem(now: NaiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}-{:03}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.nanosecond() / 1_000_000 % 1000
    )
}

/// `uniqueScreenshotPath`: the timestamp name in `folder`, with `-2`, `-3`...
/// before the extension while the file exists.
pub fn unique_screenshot_path(folder: &Path, now: NaiveDateTime) -> PathBuf {
    unique_capture_path(folder, now, "png")
}

/// [`unique_screenshot_path`] for a capture of another kind (`mp4`).
pub fn unique_capture_path(folder: &Path, now: NaiveDateTime, extension: &str) -> PathBuf {
    let stem = timestamp_stem(now);
    let mut candidate = folder.join(format!("{stem}.{extension}"));
    let mut suffix = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem}-{suffix}.{extension}"));
        suffix += 1;
    }
    candidate
}

/// Where the replays of `game` go: the `replays` folder inside its
/// captures folder.
pub fn replay_folder(game: &ActiveGame, root: &Path) -> PathBuf {
    capture_folder(game, root).join(crate::list::REPLAYS_FOLDER)
}

/// Temp file used while the PNG is written (`.<app>-capture.tmp`; hidden so
/// listing never shows it).
pub fn temporary_name(app_name: &str) -> String {
    let safe = sanitize_filename::sanitize(app_name);
    format!(".{}-capture.tmp", if safe.is_empty() { "game" } else { &safe })
}

/// A game that is currently running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGame {
    pub app_name: String,
    pub runner: String,
    pub title: String,
    /// Other names that may name the captures folder.
    pub aliases: Vec<String>,
    pub steam_app_id: Option<String>,
    /// Orders sessions: the most recent one receives the captures.
    pub started_at: std::time::SystemTime,
}

impl ActiveGame {
    pub fn key(&self) -> String {
        format!("{}:{}", self.runner, self.app_name)
    }
}

/// `captureFolder`: the matched game folder under `root`, else
/// `<root>/<sanitized title>` (falling back to the app name, then `Game`).
pub fn capture_folder(game: &ActiveGame, root: &Path) -> PathBuf {
    let folders = list_game_folders(root);
    let mut queries = vec![game.title.clone()];
    queries.extend(game.aliases.iter().cloned());
    if let Some(m) = pick_screenshot_folder(&queries, &folders, game.steam_app_id.as_deref(), MIN_SCORE) {
        return root.join(m.folder);
    }
    let title = sanitize_filename::sanitize(&game.title);
    let title = title.trim();
    let name = if title.is_empty() { sanitize_filename::sanitize(&game.app_name).trim().to_owned() } else { title.to_owned() };
    root.join(if name.is_empty() { "Game" } else { &name })
}

/// `currentGame`: the most recently started game.
pub fn current_game<'a>(games: impl IntoIterator<Item = &'a ActiveGame>) -> Option<&'a ActiveGame> {
    games.into_iter().max_by_key(|g| g.started_at)
}

/// Settings the service needs (`screenshots.*` of `local_library/settings.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureConfig {
    pub enabled: bool,
    pub folder: PathBuf,
    pub accelerator: String,
}

impl CaptureConfig {
    pub fn from_settings(s: &heronite_data::models::ScreenshotsSettings) -> Self {
        Self {
            enabled: s.capture_enabled,
            folder: PathBuf::from(s.folder.trim()),
            accelerator: if s.capture_accelerator.trim().is_empty() {
                heronite_data::settings::DEFAULT_CAPTURE_ACCELERATOR.to_owned()
            } else {
                s.capture_accelerator.trim().to_owned()
            },
        }
    }
}

/// Why a capture did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Disabled,
    NoActiveGame,
    FolderNotConfigured,
    Busy,
}

/// Result of one capture attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureOutcome {
    Saved(PathBuf),
    Skipped(SkipReason),
    /// `capture-unavailable`: every capture method failed.
    Failed(String),
}

/// Which mechanism delivers the shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutBackend {
    Hyprland,
    Portal,
    /// `RegisterHotKey`.
    #[cfg(windows)]
    Windows,
}

/// Events emitted by the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureEvent {
    /// `collectionScreenshotSaved`: a PNG was written.
    Saved {
        app_name: String,
        runner: String,
        path: PathBuf,
    },
    Failed {
        error: String,
    },
    ShortcutRegistered {
        accelerator: String,
        backend: ShortcutBackend,
    },
    ShortcutError {
        error: String,
    },
}

/// `CollectionScreenshotCaptureStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureStatus {
    pub registered: bool,
    pub accelerator: String,
    /// `(app_name, runner, title)` of the game receiving captures.
    pub active_game: Option<(String, String, String)>,
    pub error: Option<String>,
}

/// [`CaptureService`] construction options.
#[derive(Debug, Clone, Default)]
pub struct ServiceOptions {
    /// Reverse-DNS desktop id registered with the portal for a non-sandboxed
    /// app (Heronite uses `app.heronite.Heronite`, matching its `.desktop`
    /// file). Without it the portal may not be able to identify the app.
    pub portal_app_id: Option<String>,
    pub messages: CaptureMessages,
}

/// A [`GlobalHotkey`] to register.
#[derive(Debug, Clone)]
pub struct HotkeySpec {
    /// The shortcut's id in its portal session.
    pub id: &'static str,
    /// Shown by the desktop; on Hyprland it also recognises a bind left
    /// behind by a crash.
    pub description: &'static str,
    /// Signal the Hyprland bind sends this process. Not `SIGUSR2`: that is the
    /// screenshot's.
    #[cfg(target_os = "linux")]
    pub signal: i32,
    /// Electron-style (`Super+G`).
    pub accelerator: String,
    /// The focused window (the game) does not get the keys (Hyprland).
    pub consume: bool,
    /// See [`ServiceOptions::portal_app_id`].
    pub portal_app_id: Option<String>,
}

/// Texts of the desktop notifications (`notify.screenshot.*`), in English:
/// they are translated when shown.
#[derive(Debug, Clone)]
pub struct CaptureMessages {
    pub saved_title: String,
    /// `{game}` and `{path}` are replaced.
    pub saved_body: String,
    pub failed_title: String,
    /// `{error}` is replaced.
    pub failed_body: String,
    pub shortcut_failed_title: String,
    /// `{error}` is replaced.
    pub shortcut_failed_body: String,
}

impl Default for CaptureMessages {
    fn default() -> Self {
        Self {
            saved_title: n_!("Screenshot saved").into(),
            saved_body: n_!("{game} screenshot saved to {path}").into(),
            failed_title: n_!("Screenshot failed").into(),
            failed_body: n_!("The screen could not be captured: {error}").into(),
            shortcut_failed_title: n_!("Screenshot shortcut unavailable").into(),
            shortcut_failed_body: n_!("The configured shortcut could not be activated: {error}").into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn game(title: &str) -> ActiveGame {
        ActiveGame {
            app_name: "app".into(),
            runner: "sideload".into(),
            title: title.into(),
            aliases: vec![],
            steam_app_id: None,
            started_at: UNIX_EPOCH,
        }
    }

    #[test]
    fn picks_monitor() {
        let m = |name: &str, x, w, focused| Monitor { name: name.into(), x, y: 0, width: w, height: 1080, focused };
        let ms = [m("A", 0, 1920, true), m("B", 1920, 2560, false)];
        assert_eq!(monitor_under_pointer(&ms, 100, 100).unwrap().name, "A");
        assert_eq!(monitor_under_pointer(&ms, 1920, 5).unwrap().name, "B");
        assert_eq!(monitor_under_pointer(&ms, 9999, 5).unwrap().name, "A");
        assert!(monitor_under_pointer(&[], 0, 0).is_none());
    }

    #[test]
    fn timestamp_and_unique_names() {
        let t = NaiveDate::from_ymd_opt(2026, 3, 4).unwrap().and_hms_milli_opt(5, 6, 7, 89).unwrap();
        assert_eq!(timestamp_name(t), "2026-03-04_05-06-07-089.png");
        let dir = tempfile::tempdir().unwrap();
        let p1 = unique_screenshot_path(dir.path(), t);
        assert_eq!(p1, dir.path().join("2026-03-04_05-06-07-089.png"));
        std::fs::write(&p1, b"").unwrap();
        let p2 = unique_screenshot_path(dir.path(), t);
        assert_eq!(p2, dir.path().join("2026-03-04_05-06-07-089-2.png"));
        std::fs::write(&p2, b"").unwrap();
        assert_eq!(unique_screenshot_path(dir.path(), t), dir.path().join("2026-03-04_05-06-07-089-3.png"));
        assert_eq!(unique_capture_path(dir.path(), t, "mp4"), dir.path().join("2026-03-04_05-06-07-089.mp4"));
    }

    #[test]
    fn replays_go_in_the_game_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Death Stranding - Director's Cut")).unwrap();
        assert_eq!(
            replay_folder(&game("Death Stranding"), dir.path()),
            dir.path().join("Death Stranding - Director's Cut").join("replays")
        );
    }

    #[test]
    fn temp_names() {
        assert_eq!(temporary_name("steam_123"), ".steam_123-capture.tmp");
        assert_eq!(temporary_name("a/b:c"), ".abc-capture.tmp");
        assert_eq!(temporary_name(""), ".game-capture.tmp");
    }

    #[test]
    fn folder_selection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Death Stranding - Director's Cut")).unwrap();
        std::fs::create_dir_all(root.join("1245620")).unwrap();
        // Matches an existing folder.
        assert_eq!(capture_folder(&game("Death Stranding"), root), root.join("Death Stranding - Director's Cut"));
        // Aliases and Steam ids too.
        let mut g = game("Something Else");
        g.steam_app_id = Some("1245620".into());
        assert_eq!(capture_folder(&g, root), root.join("1245620"));
        let mut g = game("Unmatched");
        g.aliases = vec!["Death Stranding".into()];
        assert_eq!(capture_folder(&g, root), root.join("Death Stranding - Director's Cut"));
        // New folder: sanitized title, then app name, then "Game".
        assert_eq!(capture_folder(&game("New: Game/Title?"), root), root.join("New GameTitle"));
        let mut g = game("???");
        g.app_name = "steam_9".into();
        assert_eq!(capture_folder(&g, root), root.join("steam_9"));
        let mut g = game("???");
        g.app_name = "".into();
        assert_eq!(capture_folder(&g, root), root.join("Game"));
    }

    #[test]
    fn most_recent_game_wins() {
        let mut a = game("A");
        a.started_at = UNIX_EPOCH + Duration::from_secs(10);
        let mut b = game("B");
        b.app_name = "other".into();
        b.started_at = UNIX_EPOCH + Duration::from_secs(20);
        assert_eq!(current_game([&a, &b]).unwrap().title, "B");
        assert!(current_game(std::iter::empty::<&ActiveGame>()).is_none());
        let _ = SystemTime::now();
    }

    #[test]
    fn config_from_settings() {
        use heronite_data::models::ScreenshotsSettings;
        let s = ScreenshotsSettings {
            folder: " /x/y ".into(),
            capture_enabled: true,
            capture_accelerator: String::new(),
            ..Default::default()
        };
        let c = CaptureConfig::from_settings(&s);
        assert_eq!(c, CaptureConfig { enabled: true, folder: "/x/y".into(), accelerator: "F10".into() });
    }
}
