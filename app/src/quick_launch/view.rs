//! The launcher's root view: the palette in `Mode::Launcher`, fed with the
//! pages and actions at once and the library's games once it is read (off
//! the UI thread), and the keys of its window.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ui_state::UiState;
use heronite_data::{art, AppPaths, ArtContext, DataPaths, Library};
use heronite_i18n::tr;
use heronite_images::ImageRef;

use super::run;
use crate::collection::palette::launcher::{chosen, Chosen};
use crate::collection::palette::{self, global_entries, library_entries, Act, Mode, PaletteEvent, PaletteView, Setup};
use crate::images::to_chain;
use crate::theme;
use crate::ui::controls::{button_sm, kbd, ButtonKind};

pub(super) struct QuickLauncher {
    palette: Entity<PaletteView>,
    data_paths: DataPaths,
    /// The library, once read.
    lib: Option<Arc<Library>>,
    /// The games running when the rows were made (Play or Stop, Enter).
    running: HashSet<String>,
    /// The game whose actions were last asked for (key and title), to make
    /// them again when it starts or ends.
    game_shown: Option<(String, SharedString)>,
    /// Whether it ever had the keyboard: leaving it after that closes it.
    was_active: bool,
    opened: std::time::Instant,
    /// Enter pressed before the games were listed: run once they are, on
    /// what is then first.
    pending_enter: Option<Keystroke>,
    /// Reading the library failed: no games are listed, Enter is not held
    /// for them, and Ctrl R (or the notice's button) reads it again.
    load_failed: bool,
    /// What reading it again needs.
    include_hidden: bool,
    debug_keys: Option<String>,
    _subs: Vec<Subscription>,
}

/// What reading the library gives.
struct Loaded {
    lib: Library,
    covers: HashMap<SharedString, Vec<ImageRef>>,
}

fn load(paths: &DataPaths, include_hidden: bool) -> anyhow::Result<Loaded> {
    let lib = Library::load_with(paths, include_hidden)?;
    let shots = (!lib.settings.screenshots.folder.is_empty()).then(|| std::path::PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(paths, shots.as_deref());
    let covers = lib.games.iter().map(|g| (SharedString::from(g.key.clone()), to_chain(&art::card_cover(&ctx, g)))).collect();
    Ok(Loaded { lib, covers })
}

impl QuickLauncher {
    pub fn new(query: &str, keys: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let data_paths = DataPaths::detect();
        let app_paths = AppPaths::for_data(&data_paths);
        let ui = UiState::load(&app_paths);
        // The open window's history may be newer than the file.
        let recent = crate::system::main_view(cx)
            .and_then(|v| v.upgrade())
            .map(|v| v.read(cx).palette_history())
            .unwrap_or_else(|| ui.palette_recent.clone());
        let setup = Setup {
            entries: Rc::new(global_entries(cx)),
            recent: Rc::new(recent),
            covers: Rc::default(),
            data_paths: data_paths.clone(),
            mode: Mode::Launcher,
        };
        let palette = cx.new(|cx| PaletteView::new(setup, query, window, cx));
        let handle = window.window_handle();
        let weak = cx.entity().downgrade();
        let keys_sub = cx.intercept_keystrokes(move |ev, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            if this.update(cx, |this, cx| this.key(&ev.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        });
        let subs = vec![
            cx.subscribe_in(&palette, window, Self::on_palette_event),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.was_active = true;
                } else if this.was_active {
                    // Clicked away: it goes, like a menu.
                    let id = window.window_handle().window_id();
                    cx.defer(move |cx| super::close_window(id, cx));
                }
            }),
            keys_sub,
        ];
        let this = Self {
            palette,
            data_paths: data_paths.clone(),
            lib: None,
            running: HashSet::new(),
            game_shown: None,
            was_active: false,
            opened: std::time::Instant::now(),
            pending_enter: None,
            load_failed: false,
            include_hidden: crate::services::hidden_lock::shown(ui.show_hidden, &app_paths),
            debug_keys: keys,
            _subs: subs,
        };
        Self::spawn_load(data_paths, this.include_hidden, window, cx);
        this
    }

    /// Reads the library off the UI thread, then lists its games.
    fn spawn_load(data_paths: DataPaths, include_hidden: bool, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx.background_spawn(async move { load(&data_paths, include_hidden) }).await;
            this.update_in(cx, |this, window, cx| match loaded {
                Ok(l) => {
                    let keys = this.debug_keys.take();
                    this.apply_loaded(l, keys, window, cx);
                }
                Err(e) => this.on_load_failed(e, cx),
            })
            .ok();
        })
        .detach();
    }

    /// No games: an Enter held for them is dropped (it would run whatever
    /// page or action is first instead), and the notice says why.
    fn on_load_failed(&mut self, e: anyhow::Error, cx: &mut Context<Self>) {
        log::warn!("quick launcher: cannot read the library: {e:#}");
        self.load_failed = true;
        self.pending_enter = None;
        cx.notify();
    }

    fn retry_load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_failed = false;
        cx.notify();
        Self::spawn_load(self.data_paths.clone(), self.include_hidden, window, cx);
    }

    fn apply_loaded(&mut self, l: Loaded, keys: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        log::info!("quick launcher: {} games listed {:?} after opening", l.lib.games.len(), self.opened.elapsed());
        self.running = crate::system::running_games(cx);
        let entries = self.entries_for(&l.lib, cx);
        self.lib = Some(Arc::new(l.lib));
        let covers = Rc::new(l.covers);
        self.palette.update(cx, |p, cx| p.set_entries(Rc::new(entries), covers, cx));
        if let Some(ks) = self.pending_enter.take() {
            self.key(&ks, window, cx);
        }
        // `HERONITE_DEBUG_QUICK_LAUNCH_KEYS`, once the games are listed.
        let keys = keys.iter().flat_map(|k| k.split(',')).filter(|k| !k.is_empty()).map(str::to_string).collect();
        self.feed_keys(keys, window, cx);
    }

    /// The pages, the actions and `lib`'s games, as Play or Stop by
    /// [`Self::running`].
    fn entries_for(&self, lib: &Library, cx: &App) -> Vec<palette::Entry> {
        let mut entries = global_entries(cx);
        entries.extend(library_entries(lib, &|app_name| self.running.contains(app_name)));
        entries
    }

    /// A game started or ended while the launcher is open: its row, the
    /// actions shown for it, the footer and Enter follow.
    pub(super) fn set_running(&mut self, running: HashSet<String>, cx: &mut Context<Self>) {
        if running == self.running {
            return;
        }
        self.running = running;
        let Some(lib) = self.lib.clone() else { return };
        let entries = Rc::new(self.entries_for(&lib, cx));
        let game = self.game_shown.clone().filter(|_| self.palette.read(cx).in_game());
        self.palette.update(cx, |p, cx| {
            let game_entries = game.and_then(|(key, title)| {
                let bits = entries.iter().find(|e| matches!(&e.act, Act::SelectGame(k) if *k == key))?.game.clone()?;
                Some(game_level(&lib, &title, &bits))
            });
            p.refresh_entries(entries.clone(), game_entries, cx);
        });
    }

    /// Debug keys, one per cycle, so what a key asks for (a game's actions)
    /// is there before the next.
    fn feed_keys(&mut self, mut keys: std::collections::VecDeque<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(k) = keys.pop_front() else { return };
        if let Ok(ks) = Keystroke::parse(&k) {
            // What the palette does not take is typed, as a real key would be
            // (outside this update: the keys come back through `key`).
            if !self.key(&ks, window, cx) {
                let handle = window.window_handle();
                cx.defer(move |cx| {
                    handle.update(cx, |_, window, cx| window.dispatch_keystroke(ks, cx)).ok();
                });
            }
        }
        cx.defer_in(window, move |this, window, cx| this.feed_keys(keys, window, cx));
    }

    fn key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &ks.modifiers;
        if self.lib.is_none() && !self.load_failed && ks.key == "enter" && !(m.control || m.alt || m.platform) {
            self.pending_enter = Some(ks.clone());
            return true;
        }
        if self.load_failed && ks.key == "r" && (m.control || m.platform) && !(m.alt || m.shift) {
            self.retry_load(window, cx);
            return true;
        }
        self.palette.update(cx, |p, cx| p.key(ks, window, cx))
    }

    fn on_palette_event(&mut self, view: &Entity<PaletteView>, ev: &PaletteEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PaletteEvent::Close => {
                let id = window.window_handle().window_id();
                cx.defer(move |cx| super::close_window(id, cx));
            }
            PaletteEvent::Run { act: Act::Random(_), .. } => {}
            PaletteEvent::Run { act, id } => {
                let bits = act.game().and_then(|k| view.read(cx).game_bits_of(k));
                if let Chosen::BackToGame(key) = chosen(act, bits.as_ref()) {
                    return self.back_to_game(key, window, cx);
                }
                run::run(act.clone(), id, bits, self.lib.clone(), &self.data_paths, cx);
                let id = window.window_handle().window_id();
                cx.defer(move |cx| super::close_window(id, cx));
            }
            PaletteEvent::OpenGame(key) => self.open_game(key, window, cx),
        }
    }

    /// The actions of the game under `key`, in place of the list.
    fn open_game(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(bits), Some(lib)) = (self.palette.read(cx).game_bits_of(key), self.lib.clone()) else { return };
        let title = bits.card.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| key.to_string().into());
        let entries = game_level(&lib, &title, &bits);
        self.game_shown = Some((key.to_string(), title.clone()));
        self.palette.update(cx, |p, cx| p.show_game(title, entries, window, cx));
    }

    /// Gives the focus to the running game's window, and closes; when it has
    /// none (or it cannot be found), its actions are shown instead, Stop
    /// among them.
    fn back_to_game(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let pid = self.lib.as_ref().and_then(|l| l.get(&key)).and_then(|g| crate::system::game_pid(cx, &g.app_name));
        let Some(pid) = pid else {
            log::info!("quick launcher: back to the game: its process is not known yet");
            return self.open_game(&key, window, cx);
        };
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_overlay::game_window::focus(pid) }).await;
            this.update_in(cx, |this, window, cx| match res {
                Ok(()) => {
                    let id = window.window_handle().window_id();
                    cx.defer(move |cx| super::close_window(id, cx));
                }
                Err(e) => {
                    log::info!("quick launcher: back to the game: {e}; showing its actions");
                    this.open_game(&key, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

/// The quick launcher's actions of a game of `lib`.
fn game_level(lib: &Library, title: &str, bits: &palette::GameBits) -> Vec<palette::Entry> {
    palette::launcher::game_level(title, bits, &lib.statuses, palette::launcher::can_focus_games())
}

impl Render for QuickLauncher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notice = self.load_failed.then(|| load_error_notice(cx));
        div().relative().size_full().child(self.palette.clone()).children(notice)
    }
}

/// Over the bottom of the list, above the footer: the library could not be
/// read, so only pages and actions are listed; Ctrl R or the button tries again.
fn load_error_notice(cx: &mut Context<QuickLauncher>) -> Stateful<Div> {
    let row = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(14.))
        .py(px(10.))
        .bg(theme::danger_fill())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(theme::danger_text())
                .child(tr!("Heronite could not read the library, so no games are listed.")),
        )
        .child(kbd("Ctrl R"))
        .child(
            button_sm("quick-launch-retry", ButtonKind::Glass, tr!("Try again"))
                .on_click(cx.listener(|this, _, window, cx| this.retry_load(window, cx))),
        );
    // The tint is see-through: the page colour under it hides the list.
    div()
        .id("quick-launch-load-error")
        .occlude()
        .absolute()
        .left(px(12.))
        .right(px(12.))
        .bottom(PaletteView::footer_height() + px(8.))
        .overflow_hidden()
        .rounded(px(theme::R_INPUT))
        .border_1()
        .border_color(theme::card_border())
        .bg(theme::page_bg())
        .child(row)
}
