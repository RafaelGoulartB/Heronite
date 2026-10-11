//! Launch requests (CLI, protocol URLs, tray, other instances), with or without a window.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use gpui_kit::*;
use heronite_data::{CollectionGame, Library};

use super::quit::maybe_exit_idle;
use super::window::{close_window_keep_alive, show_window};
use super::{hub, EXIT_CODE};
use crate::system::cli::{LaunchRequest, RUNNERS};

/// Keys of the library entries a request may mean, in search order.
pub fn candidate_keys(req: &LaunchRequest) -> Vec<String> {
    match &req.runner {
        Some(r) => vec![format!("{r}_{}", req.app_name)],
        None => RUNNERS.iter().map(|r| format!("{r}_{}", req.app_name)).collect(),
    }
}

fn find_game(lib: &Library, req: &LaunchRequest) -> Option<Arc<CollectionGame>> {
    candidate_keys(req).iter().find_map(|k| lib.get(k)).cloned()
}

/// Launches `req`. `hide`: keep the window out of it (headless launch);
/// `close_visible`: with `hide`, also close an open window (Heroic hides it)
/// when there is a tray to come back from.
pub(super) fn launch(cx: &mut App, req: LaunchRequest, hide: bool, close_visible: bool) {
    log::info!("launch request: {req:?} hide={hide}");
    if !req.args.is_empty() || req.alt_exe.is_some() {
        log::warn!("launch arguments / altExe from the URL are not supported yet and are ignored");
    }
    if hide {
        if close_visible && close_window_keep_alive(cx) {
            log::info!("window hidden for the launch");
        }
        launch_headless(cx, req);
    } else if let Some(view) = show_window(cx) {
        let _ = view.update(cx, |v, cx| v.system_launch(req, cx));
    }
}

fn launch_headless(cx: &mut App, req: LaunchRequest) {
    let data_paths = hub(cx).0.borrow().data_paths.clone();
    cx.spawn(async move |cx| {
        let (h, r) = (data_paths, req.clone());
        let found = cx
            .background_spawn(async move {
                Library::load(&h).map(|lib| find_game(&lib, &r)).map_err(|e| format!("could not read the library: {e:#}"))
            })
            .await;
        cx.update(|cx| on_headless_game(cx, req, found));
    })
    .detach();
}

fn fail(cx: &mut App, message: String) {
    log::error!("{message}");
    EXIT_CODE.store(1, Ordering::Relaxed);
    let hub = hub(cx);
    let view = hub.0.borrow().window.as_ref().map(|w| w.view.clone());
    if let Some(view) = view {
        let _ = view.update(cx, |v, cx| v.system_toast(message, true, cx));
    }
    maybe_exit_idle(cx);
}

fn on_headless_game(cx: &mut App, req: LaunchRequest, found: Result<Option<Arc<CollectionGame>>, String>) {
    let game = match found {
        Err(e) => return fail(cx, e),
        Ok(None) => return fail(cx, format!("Could not find the game {} in the library", req.app_name)),
        Ok(Some(g)) => g,
    };
    let hub = hub(cx);
    if !game.installed {
        // Heroic asks to install it (and shows the window for that).
        log::info!("{} is not installed: showing the window", game.title);
        if let Some(view) = show_window(cx) {
            let _ = view.update(cx, |v, cx| v.system_launch(req, cx));
        }
        return;
    }
    let (runtime, fake, running) = {
        let s = hub.0.borrow();
        (s.runtime.clone(), s.fake_launch, s.running.contains_key(&game.app_name))
    };
    if running {
        log::info!("{} is already running", game.title);
        return;
    }
    if fake {
        log::info!("[fake-launch] headless {} ({}/{})", game.title, game.runner, game.app_name);
        maybe_exit_idle(cx);
        return;
    }
    // `--no-gui` skips legendary's version check (Heroic does the same).
    let result = if game.runner == "legendary" { runtime.play_skip_version_check(&game) } else { runtime.play(&game) };
    match result {
        Ok(()) => {
            hub.0.borrow_mut().running.entry(game.app_name.clone()).or_insert(None);
        }
        Err(e) => fail(cx, format!("Could not launch {}: {e}", game.title)),
    }
}

#[cfg(test)]
mod tests {
    use super::{candidate_keys, LaunchRequest};

    #[test]
    fn candidate_keys_follow_runner_order() {
        let r = LaunchRequest { runner: Some("gog".into()), app_name: "123".into(), ..Default::default() };
        assert_eq!(candidate_keys(&r), vec!["gog_123"]);
        let r = LaunchRequest { runner: None, app_name: "Quail".into(), ..Default::default() };
        assert_eq!(candidate_keys(&r), vec!["legendary_Quail", "gog_Quail", "nile_Quail", "sideload_Quail", "zoom_Quail"]);
    }
}
