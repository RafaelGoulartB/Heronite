//! Screenshot capture while a game runs. Port of
//! `src/local-library/screenshots/capture.ts`.
//!
//! The portable pieces live here and are unit-tested: accelerator parsing and
//! translation (Hyprland `hl.bind` syntax, XDG GlobalShortcuts triggers), file
//! naming, destination folder selection, monitor picking. The side-effecting
//! service ([`CaptureService`], Linux only) lives in `service.rs`.
//!
//! # How a capture happens
//! 1. `begin_session` for a launched game registers the shortcut (only while a
//!    game is active and `screenshots.captureEnabled` is set).
//! 2. On Hyprland a temporary, in-memory, non-consuming bind
//!    (`hyprctl eval hl.bind(...)`) runs `kill -USR2 <our pid>`; a
//!    `signal-hook` thread turns SIGUSR2 into a capture. Elsewhere the XDG
//!    GlobalShortcuts portal (`ashpd`) delivers the activation.
//! 3. The display under the pointer is captured with `grim` (Hyprland) or the
//!    portal Screenshot method, written as PNG into the game's matched folder
//!    (or `<root>/<sanitized title>`), and a [`CaptureEvent::Saved`] is sent.

use crate::list::list_game_folders;
use crate::matching::{pick_screenshot_folder, MIN_SCORE};
use chrono::{Datelike, NaiveDateTime, Timelike};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod feedback;
#[cfg(target_os = "linux")]
mod grim;
#[cfg(target_os = "linux")]
mod hyprland;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
mod service;

#[cfg(target_os = "linux")]
pub use service::{CaptureService, ServiceOptions};

/// Description used for the Hyprland bind and the portal shortcut.
pub const BIND_DESCRIPTION: &str = "Heronite game screenshot";

// ---------------------------------------------------------------------------
// Accelerators

/// A parsed Electron accelerator (`CommandOrControl+Shift+F10`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accelerator {
    /// Modifier names as written, in order.
    pub modifiers: Vec<String>,
    /// The key as written (`F10`, `Esc`, `PrintScreen`, `a`).
    pub key: String,
}

/// Parses an Electron-style accelerator. Returns `None` when there is no key
/// or it contains characters that cannot be a key name.
pub fn parse_accelerator(accelerator: &str) -> Option<Accelerator> {
    let mut parts: Vec<&str> = accelerator.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let key = parts.pop()?;
    if !key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b' ' | b'-')) {
        return None;
    }
    Some(Accelerator { modifiers: parts.into_iter().map(str::to_owned).collect(), key: key.to_owned() })
}

/// Hyprland modifier masks (`modmask` in `hyprctl binds -j`).
pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 4;
pub const MOD_ALT: u32 = 8;
pub const MOD_SUPER: u32 = 64;

/// How a shortcut is expressed for Hyprland.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyprlandBinding {
    /// `CTRL + SHIFT + F10`, the string given to `hl.bind` / `hl.unbind`.
    pub keys: String,
    /// The key alone (`F10`).
    pub key: String,
    pub modmask: u32,
}

fn hypr_modifier(name: &str) -> Option<(&'static str, u32)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "commandorcontrol" | "control" | "ctrl" => ("CTRL", MOD_CTRL),
        "command" | "super" | "meta" => ("SUPER", MOD_SUPER),
        "alt" | "option" => ("ALT", MOD_ALT),
        "shift" => ("SHIFT", MOD_SHIFT),
        _ => return None,
    })
}

/// `toHyprlandAccelerator`.
pub fn to_hyprland_binding(accelerator: &str) -> Option<HyprlandBinding> {
    let acc = parse_accelerator(accelerator)?;
    let mut names = Vec::new();
    let mut modmask = 0;
    for m in &acc.modifiers {
        let (name, mask) = hypr_modifier(m)?;
        names.push(name);
        modmask |= mask;
    }
    let key = match acc.key.as_str() {
        "Esc" => "Escape",
        "Up" => "UP",
        "Down" => "DOWN",
        "Left" => "LEFT",
        "Right" => "RIGHT",
        "PrintScreen" => "Print",
        other => other,
    };
    names.push(key);
    Some(HyprlandBinding { keys: names.join(" + "), key: key.to_owned(), modmask })
}

/// Trigger string for the XDG GlobalShortcuts portal
/// (`CTRL+SHIFT+F10`; modifiers `SHIFT CTRL ALT LOGO`, key = XKB keysym name).
pub fn to_portal_trigger(accelerator: &str) -> Option<String> {
    let acc = parse_accelerator(accelerator)?;
    let mut out: Vec<String> = Vec::new();
    for m in &acc.modifiers {
        out.push(
            match m.to_ascii_lowercase().as_str() {
                "commandorcontrol" | "control" | "ctrl" => "CTRL",
                "command" | "super" | "meta" => "LOGO",
                "alt" | "option" => "ALT",
                "shift" => "SHIFT",
                _ => return None,
            }
            .to_owned(),
        );
    }
    out.push(keysym_name(&acc.key));
    Some(out.join("+"))
}

/// Electron key name -> XKB keysym name.
fn keysym_name(key: &str) -> String {
    let mapped = match key.to_ascii_lowercase().as_str() {
        "esc" | "escape" => "Escape",
        "printscreen" | "print" => "Print",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "space" => "space",
        "plus" => "plus",
        "enter" | "return" => "Return",
        "backspace" => "BackSpace",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "capslock" => "Caps_Lock",
        "scrolllock" => "Scroll_Lock",
        "numlock" => "Num_Lock",
        "tab" => "Tab",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        _ => "",
    };
    if !mapped.is_empty() {
        return mapped.to_owned();
    }
    // F1..F24 keep their case; single letters are lowercase keysyms.
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
        return key.to_ascii_lowercase();
    }
    let b = key.as_bytes();
    if b.len() >= 2 && (b[0] == b'f' || b[0] == b'F') && b[1..].iter().all(u8::is_ascii_digit) {
        return format!("F{}", &key[1..]);
    }
    key.to_owned()
}

// ---------------------------------------------------------------------------
// Hyprland bind inspection (`hyprctl binds -j`)

/// One entry of `hyprctl binds -j`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HyprlandBind {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub modmask: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
}

impl HyprlandBind {
    /// Same key and modifiers (key compared case-insensitively).
    pub fn is_same_trigger(&self, binding: &HyprlandBinding) -> bool {
        self.key.as_deref().map(str::to_uppercase).as_deref() == Some(binding.key.to_uppercase().as_str())
            && self.modmask == Some(binding.modmask)
    }

    /// One of ours (left behind by a crashed session) with this trigger.
    pub fn is_ours(&self, binding: &HyprlandBinding) -> bool {
        self.description.as_deref() == Some(BIND_DESCRIPTION) && self.is_same_trigger(binding)
    }
}

/// The Lua expression that installs the temporary bind.
pub fn hyprland_bind_expression(binding: &HyprlandBinding, pid: u32) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("string");
    format!(
        "hl.bind({}, hl.dsp.exec_cmd({}), {{ non_consuming = true, dont_inhibit = true, allow_input_capture = true, description = {} }})",
        q(&binding.keys),
        q(&format!("kill -USR2 {pid}")),
        q(BIND_DESCRIPTION),
    )
}

/// The Lua expression that removes a bind.
pub fn hyprland_unbind_expression(keys: &str) -> String {
    format!("hl.unbind({})", serde_json::to_string(keys).expect("string"))
}

// ---------------------------------------------------------------------------
// Monitor under the pointer

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

// ---------------------------------------------------------------------------
// File naming and destination

/// `timestampName`: `YYYY-MM-DD_HH-MM-SS-mmm.png` (local time).
pub fn timestamp_name(now: NaiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}-{:03}.png",
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
    let initial = timestamp_name(now);
    let mut candidate = folder.join(&initial);
    let stem = initial.trim_end_matches(".png");
    let mut suffix = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem}-{suffix}.png"));
        suffix += 1;
    }
    candidate
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
    let name = if title.is_empty() {
        sanitize_filename::sanitize(&game.app_name).trim().to_owned()
    } else {
        title.to_owned()
    };
    root.join(if name.is_empty() { "Game" } else { &name })
}

/// `currentGame`: the most recently started game.
pub fn current_game<'a>(games: impl IntoIterator<Item = &'a ActiveGame>) -> Option<&'a ActiveGame> {
    games.into_iter().max_by_key(|g| g.started_at)
}

// ---------------------------------------------------------------------------
// Public data types

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
}

/// Events emitted by the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureEvent {
    /// `collectionScreenshotSaved`: a PNG was written.
    Saved { app_name: String, runner: String, path: PathBuf },
    Failed { error: String },
    ShortcutRegistered { accelerator: String, backend: ShortcutBackend },
    ShortcutError { error: String },
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

/// Texts of the desktop notifications (`notify.screenshot.*`).
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
            saved_title: "Screenshot saved".into(),
            saved_body: "{game} screenshot saved to {path}".into(),
            failed_title: "Screenshot failed".into(),
            failed_body: "The screen could not be captured: {error}".into(),
            shortcut_failed_title: "Screenshot shortcut unavailable".into(),
            shortcut_failed_body: "The configured shortcut could not be activated: {error}".into(),
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
    fn parses_accelerators() {
        assert_eq!(
            parse_accelerator("CommandOrControl + Shift+F10"),
            Some(Accelerator { modifiers: vec!["CommandOrControl".into(), "Shift".into()], key: "F10".into() })
        );
        assert_eq!(parse_accelerator("F12").unwrap().modifiers.len(), 0);
        assert!(parse_accelerator("").is_none());
        assert!(parse_accelerator("Ctrl+").map(|a| a.key) == Some("Ctrl".into()));
        assert!(parse_accelerator("Ctrl+?").is_none());
    }

    #[test]
    fn hyprland_translation() {
        let b = to_hyprland_binding("F12").unwrap();
        assert_eq!((b.keys.as_str(), b.key.as_str(), b.modmask), ("F12", "F12", 0));
        let b = to_hyprland_binding("CommandOrControl+Shift+PrintScreen").unwrap();
        assert_eq!(b.keys, "CTRL + SHIFT + Print");
        assert_eq!(b.key, "Print");
        assert_eq!(b.modmask, MOD_CTRL | MOD_SHIFT);
        let b = to_hyprland_binding("Super+Alt+Esc").unwrap();
        assert_eq!(b.keys, "SUPER + ALT + Escape");
        assert_eq!(b.modmask, MOD_SUPER | MOD_ALT);
        assert_eq!(to_hyprland_binding("Meta+Up").unwrap().keys, "SUPER + UP");
        assert_eq!(to_hyprland_binding("alt+Left").unwrap().keys, "ALT + LEFT");
        assert_eq!(to_hyprland_binding("Option+Down").unwrap().keys, "ALT + DOWN");
        assert_eq!(to_hyprland_binding("Command+Right").unwrap().modmask, MOD_SUPER);
        assert_eq!(to_hyprland_binding("Control+A").unwrap().keys, "CTRL + A");
        // Unknown modifier / bad key.
        assert!(to_hyprland_binding("Hyper+F1").is_none());
        assert!(to_hyprland_binding("Ctrl+F1+?").is_none());
        assert!(to_hyprland_binding("").is_none());
    }

    #[test]
    fn portal_translation() {
        assert_eq!(to_portal_trigger("F12").unwrap(), "F12");
        assert_eq!(to_portal_trigger("CommandOrControl+Shift+PrintScreen").unwrap(), "CTRL+SHIFT+Print");
        assert_eq!(to_portal_trigger("Super+Alt+Esc").unwrap(), "LOGO+ALT+Escape");
        assert_eq!(to_portal_trigger("Ctrl+A").unwrap(), "CTRL+a");
        assert_eq!(to_portal_trigger("Ctrl+5").unwrap(), "CTRL+5");
        assert_eq!(to_portal_trigger("Alt+PageUp").unwrap(), "ALT+Page_Up");
        assert_eq!(to_portal_trigger("Ctrl+Space").unwrap(), "CTRL+space");
        assert_eq!(to_portal_trigger("f9").unwrap(), "F9");
        assert!(to_portal_trigger("Hyper+F1").is_none());
    }

    #[test]
    fn hyprland_expressions() {
        let b = to_hyprland_binding("Ctrl+F10").unwrap();
        assert_eq!(
            hyprland_bind_expression(&b, 4242),
            r#"hl.bind("CTRL + F10", hl.dsp.exec_cmd("kill -USR2 4242"), { non_consuming = true, dont_inhibit = true, allow_input_capture = true, description = "Heronite game screenshot" })"#
        );
        assert_eq!(hyprland_unbind_expression("CTRL + F10"), r#"hl.unbind("CTRL + F10")"#);
    }

    #[test]
    fn bind_matching() {
        let b = to_hyprland_binding("Ctrl+f10").unwrap();
        let bind: HyprlandBind =
            serde_json::from_str(r#"{"key":"F10","modmask":4,"description":"Heronite game screenshot","dispatcher":"exec"}"#).unwrap();
        assert!(bind.is_same_trigger(&b) && bind.is_ours(&b));
        let other: HyprlandBind = serde_json::from_str(r#"{"key":"F10","modmask":4,"description":""}"#).unwrap();
        assert!(other.is_same_trigger(&b) && !other.is_ours(&b));
        let diff: HyprlandBind = serde_json::from_str(r#"{"key":"F10","modmask":5}"#).unwrap();
        assert!(!diff.is_same_trigger(&b));
        let missing: HyprlandBind = serde_json::from_str("{}").unwrap();
        assert!(!missing.is_same_trigger(&b));
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
        assert_eq!(
            capture_folder(&game("Death Stranding"), root),
            root.join("Death Stranding - Director's Cut")
        );
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
