//! The launcher's root view: the palette in `Mode::Launcher`, fed with the
//! pages and actions at once and the library's games once it is read (off
//! the UI thread), and the keys of its window.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ui_state::UiState;
use heronite_data::{art, AppPaths, ArtContext, DataPaths, Library};
use heronite_images::ImageRef;

use super::run;
use crate::collection::palette::{self, global_entries, library_entries, Act, Mode, PaletteEvent, PaletteView, Setup};
use crate::images::to_chain;

pub(super) struct QuickLauncher {
    palette: Entity<PaletteView>,
    data_paths: DataPaths,
    /// The library, once read.
    lib: Option<Arc<Library>>,
    /// Whether it ever had the keyboard: leaving it after that closes it.
    was_active: bool,
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
        let ui = UiState::load(&AppPaths::for_data(&data_paths));
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
                    cx.defer(super::close);
                }
            }),
            keys_sub,
        ];
        let this = Self { palette, data_paths: data_paths.clone(), lib: None, was_active: false, _subs: subs };
        let include_hidden = ui.show_hidden;
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx.background_spawn(async move { load(&data_paths, include_hidden) }).await;
            this.update_in(cx, |this, window, cx| match loaded {
                Ok(l) => this.apply_loaded(l, keys, window, cx),
                Err(e) => log::warn!("quick launcher: cannot read the library: {e:#}"),
            })
            .ok();
        })
        .detach();
        this
    }

    fn apply_loaded(&mut self, l: Loaded, keys: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let running = crate::system::running_games(cx);
        let mut entries = global_entries(cx);
        entries.extend(library_entries(&l.lib, &|app_name| running.contains(app_name)));
        self.lib = Some(Arc::new(l.lib));
        let covers = Rc::new(l.covers);
        self.palette.update(cx, |p, cx| p.set_entries(Rc::new(entries), covers, cx));
        // `HERONITE_DEBUG_QUICK_LAUNCH_KEYS`, once the games are listed.
        let keys = keys.iter().flat_map(|k| k.split(',')).filter(|k| !k.is_empty()).map(str::to_string).collect();
        self.feed_keys(keys, window, cx);
    }

    /// Debug keys, one per cycle, so what a key asks for (a game's actions)
    /// is there before the next.
    fn feed_keys(&mut self, mut keys: std::collections::VecDeque<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(k) = keys.pop_front() else { return };
        if let Ok(ks) = Keystroke::parse(&k) {
            self.key(&ks, window, cx);
        }
        cx.defer_in(window, move |this, window, cx| this.feed_keys(keys, window, cx));
    }

    fn key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.palette.update(cx, |p, cx| p.key(ks, window, cx))
    }

    fn on_palette_event(&mut self, view: &Entity<PaletteView>, ev: &PaletteEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PaletteEvent::Close => cx.defer(super::close),
            PaletteEvent::Run { act: Act::Random(_), .. } => {}
            PaletteEvent::Run { act, id } => {
                let bits = act.game().and_then(|k| view.read(cx).game_bits_of(k));
                run::run(act.clone(), id, bits, self.lib.clone(), &self.data_paths, cx);
                cx.defer(super::close);
            }
            PaletteEvent::OpenGame(key) => {
                let Some(bits) = view.read(cx).game_bits_of(key) else { return };
                let title = bits.card.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| key.clone().into());
                let entries = palette::launcher::game_level(&title, &bits);
                view.update(cx, |p, cx| p.show_game(title, entries, window, cx));
            }
        }
    }
}

impl Render for QuickLauncher {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.palette.clone())
    }
}
