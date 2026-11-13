//! Debug hooks of the overlay (nothing is launched):
//!
//! - `HERONITE_DEBUG_OVERLAY=window[:<app_name>]` opens the panel in a normal
//!   window (`app.heronite.overlay`, for captures) as if the game were
//!   running; `layer[:<app_name>]` opens the native overlay surface. Without
//!   an app name, the first installed game of the library; `none` opens it
//!   with no game (as "without a game" allows).
//! - `HERONITE_DEBUG_OVERLAY_PAD=rb,down,a,wait:800,b` plays presses on it
//!   (the names of `HERONITE_DEBUG_PAD`).
//! - `HERONITE_DEBUG_OVERLAY_NOTICE=volume|shot|replay` shows a notice too;
//!   `achievement` a made-up achievement's (with the lines Settings →
//!   Achievements asks for), `achievement-queue` six in a row
//!   (one on screen, four waiting, then the sixth: one past the queue shows
//!   as itself, more as "N more achievements"), with the icon of
//!   `HERONITE_DEBUG_OVERLAY_NOTICE_ICON` (a URL or a file) when set.
//! - `HERONITE_DEBUG_OVERLAY_FPS=<limit>` pretends the game's limiter can
//!   change while it runs (the FPS limit row shows; nothing is applied).
//! - `HERONITE_DEBUG_OVERLAY_KEYBOARD=1` opens it as the keyboard shortcut
//!   does (it takes the keyboard); `=shortcut` registers the real shortcut
//!   instead of opening it (Settings → Overlay's, removed on exit).
//! - `HERONITE_DEBUG_OVERLAY_MONITOR=<x>,<y>` (Windows) puts the native
//!   surfaces on the monitor holding that desktop point instead of the
//!   foreground window's.

use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_i18n::tr;

use super::toast::Notice;
use super::{OverlayService, RunningGame};
use crate::icons::Icon;

pub(super) fn start(entity: &Entity<OverlayService>, cx: &mut App) {
    let Some(spec) = crate::env::value("HERONITE_DEBUG_OVERLAY") else { return };
    let (mode, wanted) = match spec.split_once(':') {
        Some((m, a)) => (m.to_string(), Some(a.to_string())),
        None => (spec, None),
    };
    let entity = entity.clone();
    let paths = entity.read(cx).data_paths.clone();
    if wanted.as_deref() == Some("none") {
        entity.update(cx, |s, cx| {
            s.debug_window = mode != "layer";
            // In memory only: the saved options stay as they are.
            s.prefs.without_game = true;
            match crate::env::value("HERONITE_DEBUG_OVERLAY_KEYBOARD").filter(|v| v != "0").as_deref() {
                Some(_) => s.open_panel_with(true, cx),
                None => s.open_panel(cx),
            }
            if let Some(panel) = s.panel {
                cx.spawn(async move |_, cx| play_script(panel, cx).await).detach();
            }
        });
        return;
    }
    entity.update(cx, |s, _| {
        s.debug_window = mode != "layer";
        s.debug_live_fps = crate::env::value("HERONITE_DEBUG_OVERLAY_FPS").and_then(|v| debug_live_fps(&v));
    });
    cx.spawn(async move |cx| {
        let game = cx
            .background_spawn(async move {
                let lib = heronite_data::Library::load_with(&paths, true).ok()?;
                let found = match &wanted {
                    Some(app) => lib.games.iter().find(|g| &g.app_name == app),
                    None => lib.games.iter().find(|g| g.installed),
                };
                found.cloned()
            })
            .await;
        let opened = entity.update(cx, |s, cx| {
            let app_name = match &game {
                Some(g) => {
                    s.runtime.debug_register_session(g);
                    g.app_name.clone()
                }
                None => "debug".to_string(),
            };
            let started = Instant::now().checked_sub(Duration::from_secs(72 * 60)).unwrap_or_else(Instant::now);
            s.games.push(RunningGame { app_name, pid: None, started });
            match crate::env::value("HERONITE_DEBUG_OVERLAY_KEYBOARD").filter(|v| v != "0").as_deref() {
                Some("shortcut") => s.sync_hotkey(cx),
                Some(_) => s.open_panel_with(true, cx),
                None => s.open_panel(cx),
            }
            s.panel
        });
        let Some(panel) = opened else { return };
        if let Some(kind) = crate::env::value("HERONITE_DEBUG_OVERLAY_NOTICE") {
            if kind.starts_with("achievement") {
                // Made-up unlocks; the icon is `HERONITE_DEBUG_OVERLAY_NOTICE_ICON` (a URL or a file).
                let icon = crate::env::value("HERONITE_DEBUG_OVERLAY_NOTICE_ICON").unwrap_or_default();
                let mut made_up = super::toast::made_up_unlocks(&icon);
                made_up.truncate(if kind == "achievement-queue" { made_up.len() } else { 1 });
                entity.update(cx, |s, cx| {
                    // The lines Settings → Achievements asks for.
                    let fields = crate::services::achievements::get(cx).read(cx).prefs().notice;
                    for unlock in made_up {
                        s.show_achievement(unlock, fields, cx);
                    }
                });
                play_script(panel, cx).await;
                return;
            }
            let notice = match kind.as_str() {
                "volume" => Notice::volume(65),
                "replay" => {
                    Notice::text(Icon::Videocam, tr!("Replay saved"), false).sub(tr!("The last {n} s. Write about it in Notes.", n = 30))
                }
                _ => Notice::text(Icon::PhotoCamera, tr!("Screenshot saved"), false)
                    .sub(game.as_ref().map(|g| g.title.clone()).unwrap_or_default()),
            };
            entity.update(cx, |s, cx| s.notify(notice, cx));
        }
        play_script(panel, cx).await;
    })
    .detach();
}

/// `HERONITE_DEBUG_OVERLAY_PAD` on the panel, then `HERONITE_DEBUG_OVERLAY_MOUSE`
/// (`console::pointer::parse_script`).
async fn play_script(panel: WindowHandle<super::panel::OverlayPanel>, cx: &mut AsyncApp) {
    let script = crate::env::value("HERONITE_DEBUG_OVERLAY_PAD").unwrap_or_default();
    for (wait, pad) in crate::console::input::parse_script(&script) {
        cx.background_executor().timer(wait).await;
        if panel.update(cx, |p, window, cx| p.on_pad(pad, window, cx)).is_err() {
            return;
        }
    }
    let script = crate::env::value("HERONITE_DEBUG_OVERLAY_MOUSE").unwrap_or_default();
    for (wait, step) in crate::console::pointer::parse_script(&script) {
        cx.background_executor().timer(wait).await;
        // The window alone: a click updates the panel itself.
        if cx.update_window(panel.into(), |_, window, cx| crate::console::pointer::play(step, window, cx)).is_err() {
            return;
        }
    }
}

/// `HERONITE_DEBUG_OVERLAY_FPS`: `<limit>` for gamescope, `mangohud:<limit>`
/// (or `mangohud-hidden:<limit>`) for MangoHud. Nothing is written: the panel
/// skips its changes under the debug hooks.
fn debug_live_fps(v: &str) -> Option<heronite_launch::live_fps::LiveFps> {
    use heronite_launch::live_fps::LiveFps;
    let (kind, limit) = v.split_once(':').unwrap_or(("gamescope", v));
    let limit = limit.parse().ok()?;
    match kind {
        "gamescope" => Some(LiveFps::Gamescope { limit }),
        "mangohud" | "mangohud-hidden" => Some(LiveFps::MangoHud {
            file: std::env::temp_dir().join("heronite-debug-mangohud.conf"),
            base: String::new(),
            smooth: false,
            limit,
            hud: kind == "mangohud",
        }),
        _ => None,
    }
}
