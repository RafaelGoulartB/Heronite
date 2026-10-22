//! Debug hooks of the overlay (nothing is launched):
//!
//! - `HERONITE_DEBUG_OVERLAY=window[:<app_name>]` opens the panel in a normal
//!   window (`com.heronite.overlay`, for captures) as if the game were
//!   running; `layer[:<app_name>]` opens the real layer-shell panel. Without
//!   an app name, the first installed game of the library; `none` opens it
//!   with no game (as "without a game" allows).
//! - `HERONITE_DEBUG_OVERLAY_PAD=rb,down,a,wait:800,b` plays presses on it
//!   (the names of `HERONITE_DEBUG_PAD`).
//! - `HERONITE_DEBUG_OVERLAY_NOTICE=volume|shot|replay` shows a notice too.
//! - `HERONITE_DEBUG_OVERLAY_FPS=<limit>` pretends the game's limiter can
//!   change while it runs (the FPS limit row shows; nothing is applied).
//! - `HERONITE_DEBUG_OVERLAY_KEYBOARD=1` opens it as the keyboard shortcut
//!   does (it takes the keyboard); `=shortcut` registers the real shortcut
//!   instead of opening it (Settings → Overlay's, removed on exit).

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
        s.debug_live_fps =
            crate::env::parse::<u32>("HERONITE_DEBUG_OVERLAY_FPS").map(|limit| heronite_launch::live_fps::LiveFps::Gamescope { limit });
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

/// `HERONITE_DEBUG_OVERLAY_PAD` on the panel.
async fn play_script(panel: WindowHandle<super::panel::OverlayPanel>, cx: &mut AsyncApp) {
    let script = crate::env::value("HERONITE_DEBUG_OVERLAY_PAD").unwrap_or_default();
    for (wait, pad) in crate::console::input::parse_script(&script) {
        cx.background_executor().timer(wait).await;
        if panel.update(cx, |p, window, cx| p.on_pad(pad, window, cx)).is_err() {
            break;
        }
    }
}
