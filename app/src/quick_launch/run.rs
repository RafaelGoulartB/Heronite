//! Carrying out what was chosen in the launcher. What needs no window runs
//! here (through the hub, like the tray's recent games); the rest opens
//! Heronite and runs there as Ctrl K would.
//!
//! Favourites and hiding go through the open window when there is one, so
//! it reloads its library and shows the change at once.

use std::sync::Arc;

use gpui_kit::*;
use heronite_data::ui_state::UiState;
use heronite_data::{AppPaths, CollectionGame, DataPaths, Library};
use heronite_i18n::tr;

use crate::collection::palette::launcher::{chosen, Chosen};
use crate::collection::palette::{self, Act, GameBits};

/// Runs `act` (`id` goes to the palette's history). `bits`: the game it is
/// about; `lib`: the library the launcher read.
pub(super) fn run(act: Act, id: &str, bits: Option<GameBits>, lib: Option<Arc<Library>>, paths: &DataPaths, cx: &mut App) {
    let game = |key: &str| lib.as_ref().and_then(|l| l.get(key)).cloned();
    match chosen(&act, bits.as_ref()) {
        Chosen::InApp(act) => in_app(act, id, true, cx),
        Chosen::Favourite(_) | Chosen::Hide(_) if crate::system::has_window(cx) => in_app(act, id, false, cx),
        Chosen::Play(key) => {
            remember(id, paths, cx);
            if let Some(g) = game(&key) {
                crate::system::play_game(cx, g);
            }
        }
        Chosen::Stop(key) => {
            remember(id, paths, cx);
            if let Some(g) = game(&key) {
                crate::system::stop_running(cx, &g.app_name);
            }
        }
        Chosen::OpenFolder(key) => {
            remember(id, paths, cx);
            let (Some(g), paths) = (game(&key), paths.clone()) else { return };
            super::background(cx, move || {
                if let Err(e) = crate::collection::open_install_folder_of(&paths, &g) {
                    tell(e);
                }
            });
        }
        Chosen::Favourite(key) | Chosen::Hide(key) => {
            remember(id, paths, cx);
            let favourite = matches!(chosen(&act, bits.as_ref()), Chosen::Favourite(_));
            if let (Some(lib), Some(g)) = (lib.as_deref(), game(&key)) {
                if let Err(e) = toggle_list(paths, lib, &g, favourite) {
                    let message = format!("{e:#}");
                    super::background(cx, move || tell(message));
                }
            }
        }
        Chosen::OpenLogs => {
            remember(id, paths, cx);
            let dir = heronite_launch::log_writer::LogPaths::for_data(paths).base;
            super::background(cx, move || {
                if !dir.is_dir() {
                    tell(tr!("There is no logs folder yet: it appears when a game is launched").to_string());
                } else if let Err(e) = heronite_platform::shell::open(&dir) {
                    tell(format!("{e}"));
                }
            });
        }
        Chosen::Quit => cx.defer(crate::system::request_quit),
    }
}

/// A notification, handed over before returning: the app may quit right
/// after (a start made only for the launcher). Off the UI thread.
fn tell(message: String) {
    crate::system::notify::send_and_wait("Heronite", message, std::time::Duration::from_secs(2));
}

/// Runs `act` in the main window, opening it (`raise`) or leaving it where
/// it is.
fn in_app(act: Act, id: &str, raise: bool, cx: &mut App) {
    if raise {
        crate::system::show_main_window(cx);
    }
    let id = id.to_string();
    crate::system::update_main(cx, move |view, window, cx| view.run_from_launcher(act, &id, window, cx));
}

/// Puts `id` first in the palette's history: the open window's, else the
/// file's.
fn remember(id: &str, paths: &DataPaths, cx: &mut App) {
    let id = id.to_string();
    if crate::system::update_main(cx, |view, _, _| view.remember_launcher_item(&id)).is_some() {
        return;
    }
    let app = AppPaths::for_data(paths);
    let mut ui = UiState::load(&app);
    palette::remember(&mut ui.palette_recent, &id);
    if let Err(e) = ui.save(&app) {
        log::warn!("quick launcher: saving the history: {e:#}");
    }
}

/// Favourite or hide `game` (and its copies while they are grouped), as it
/// is not now; with no window open.
fn toggle_list(paths: &DataPaths, lib: &Library, game: &CollectionGame, favourite: bool) -> anyhow::Result<()> {
    let grouped = UiState::load(&AppPaths::for_data(paths)).group_copies;
    let games: Vec<&Arc<CollectionGame>> = if grouped { lib.copies_of(&game.key) } else { lib.get(&game.key).into_iter().collect() };
    let on = if favourite { !game.favourite } else { !game.hidden };
    let writes: Vec<_> = games.iter().map(|g| (g.app_name.clone(), g.title.clone(), on)).collect();
    if favourite {
        heronite_data::config_games::set_favourites(paths, &writes)?;
    } else {
        heronite_data::config_games::set_hidden(paths, &writes)?;
    }
    Ok(())
}
