//! Command palette (Ctrl K): one box to reach pages, actions and games.
//!
//! * `matching`: folded fuzzy scoring. `sources`: what can be found.
//!   `rank`: query -> grouped results (pure, unit tested). `view`: the overlay.
//! * This file is the glue with [`CollectionView`]: opening and closing, the
//!   keys (palette navigation while open, global shortcuts while closed) and
//!   running what was chosen through the existing code paths.
//! * Keys are taken with a keystroke interceptor, i.e. before any input
//!   (search box, palette box) sees them, so Ctrl K, Ctrl 1..5, Ctrl `,`,
//!   Ctrl N and F5 work wherever the focus is, and the arrows, Tab, Enter and
//!   Esc drive the list while the palette is open.
//!
//! Debug: `HERONITE_DEBUG_PALETTE=<query>` opens it with that query (`-` for
//! none); `HERONITE_DEBUG_PALETTE=<query>@pages|actions|games` also picks the
//! filter.

mod matching;
mod rank;
mod sources;
mod view;

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use self::sources::{Act, GameRow};
use self::view::{PaletteEvent, PaletteView};
use super::CollectionView;
use crate::pages::Page;

/// Palette state of the Collection.
#[derive(Default)]
pub struct Host {
    view: Option<Entity<PaletteView>>,
    sub: Option<Subscription>,
    /// Keeps the keystroke interceptor alive.
    _keys: Option<Subscription>,
    debug_done: bool,
}

impl Host {
    pub fn new(window: &Window, cx: &mut Context<CollectionView>) -> Host {
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        let keys = cx.intercept_keystrokes(move |ev, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let Some(view) = weak.upgrade() else { return };
            if view.update(cx, |this, cx| this.palette_key(&ev.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        });
        Host { _keys: Some(keys), ..Host::default() }
    }
}

impl CollectionView {
    /// Opens the palette (the ☰ menu offers it too). Does nothing while a
    /// dialog is open.
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette_with("", window, cx);
    }

    fn open_palette_with(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<PaletteView>> {
        if self.dialog.is_some() || self.shots.is_open() {
            return None;
        }
        if let Some(v) = self.palette.view.clone() {
            return Some(v);
        }
        self.open_menu = None;
        self.context_menu = None;
        let entries = Rc::new(self.palette_entries());
        let view = cx.new(|cx| PaletteView::new(entries, query, window, cx));
        self.palette.sub = Some(cx.subscribe_in(&view, window, Self::on_palette_event));
        self.palette.view = Some(view.clone());
        cx.notify();
        Some(view)
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.view.take().is_some() {
            self.palette.sub = None;
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    fn on_palette_event(&mut self, _: &Entity<PaletteView>, ev: &PaletteEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PaletteEvent::Close => self.close_palette(window, cx),
            PaletteEvent::Run(act) => {
                self.close_palette(window, cx);
                self.run_palette_act(act.clone(), window, cx);
            }
        }
    }

    /// Pages, actions and the games of the library as they are now.
    fn palette_entries(&self) -> Vec<sources::Entry> {
        let mut v = sources::static_entries();
        if let Some(lib) = &self.lib {
            let mut games: Vec<_> = lib.games.iter().collect();
            games.sort_by(|a, b| a.title_lower.cmp(&b.title_lower).then(a.key.cmp(&b.key)));
            v.reserve(games.len());
            for g in games {
                let playtime = heronite_data::collection::playtime_label(g.playtime_minutes);
                v.push(sources::game_entry(&GameRow {
                    key: &g.key,
                    title: &g.title,
                    store_title: &g.game.title,
                    source: g.source_label,
                    playtime: &playtime,
                    installed: g.installed,
                    has_folder: g.details.install_path.is_some(),
                    last_played: g.last_played.as_deref(),
                }));
            }
        }
        v
    }

    /// Every keystroke of the window, before anything else sees it. Returns
    /// whether it was taken.
    fn palette_key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &ks.modifiers;
        let key = ks.key.as_str();
        let ctrl = m.control && !m.alt && !m.platform && !m.shift;
        if let Some(view) = self.palette.view.clone() {
            if ctrl && key == "k" {
                self.close_palette(window, cx);
                return true;
            }
            let plain = !m.control && !m.alt && !m.platform;
            if !plain {
                return false;
            }
            match key {
                "escape" => self.close_palette(window, cx),
                "up" => view.update(cx, |p, cx| p.move_selection(-1, cx)),
                "down" => view.update(cx, |p, cx| p.move_selection(1, cx)),
                "pageup" => view.update(cx, |p, cx| p.move_selection(-8, cx)),
                "pagedown" => view.update(cx, |p, cx| p.move_selection(8, cx)),
                "tab" => {
                    let back = m.shift;
                    view.update(cx, |p, cx| p.cycle_filter(back, cx));
                }
                "enter" => view.update(cx, |p, cx| p.activate(cx)),
                _ => return false,
            }
            return true;
        }
        // Closed: global shortcuts, unless something modal is up.
        if self.dialog.is_some() || self.shots.is_open() {
            return false;
        }
        if ctrl {
            match key {
                "k" => {
                    self.open_palette(window, cx);
                }
                "1" => self.navigate(Page::Collection, window, cx),
                "2" => self.navigate(Page::Downloads, window, cx),
                "3" => self.navigate(Page::WineManager, window, cx),
                "4" => self.navigate(Page::Epic, window, cx),
                "5" => self.navigate(Page::Emulators, window, cx),
                "," => self.navigate(Page::Settings, window, cx),
                "n" => {
                    self.open_add_game(window, cx);
                }
                _ => return false,
            }
            return true;
        }
        if key == "f5" && !m.control && !m.alt && !m.platform && !m.shift {
            self.refresh_all(cx);
            return true;
        }
        false
    }

    /// Runs what the palette item asks for, through the same paths as the
    /// menus, buttons and context menu.
    fn run_palette_act(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        match act {
            Act::Go(page) => self.navigate(page, window, cx),
            Act::Section(section) => {
                crate::pages::settings::request_section(section, cx);
                self.navigate(Page::Settings, window, cx);
            }
            Act::AddGame => {
                self.open_add_game(window, cx);
            }
            Act::Refresh => self.refresh_all(cx),
            Act::ScanRoms => self.open_scan_roms(Default::default(), window, cx),
            Act::EpicUpdates => {
                let svc = crate::services::downloads(cx);
                if svc.read(cx).account().logged_in {
                    svc.update(cx, |s, cx| s.refresh_library(cx));
                } else {
                    self.show_toast("Sign in to Epic Games first (\u{2630} \u{203a} Epic Games)", true, cx);
                }
            }
            Act::OpenLogs => {
                let dir = heronite_launch::log_writer::LogPaths::for_heroic(&self.heroic).base;
                if dir.is_dir() {
                    self.open_in_file_manager(&dir, cx);
                } else {
                    self.show_toast("There is no logs folder yet: it appears when a game is launched", true, cx);
                }
            }
            // The quit question needs the window: ask once this update ended.
            Act::Quit => cx.defer(crate::system::request_quit),
            Act::SelectGame(key) => self.palette_select(&key, window, cx),
            Act::PlayGame(key) => {
                self.palette_select(&key, window, cx);
                self.primary_action(&key, cx);
                cx.notify();
            }
            Act::GameSettings(key) => {
                self.palette_select(&key, window, cx);
                let (Some(ctx), Some(g)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(&key)).cloned()) else { return };
                let tab = crate::dialogs::game_settings::Tab::heroic_start(&g);
                let view = cx.new(|cx| crate::dialogs::game_settings::GameSettingsDialog::new(ctx, g, tab, window, cx));
                self.open_dialog(view, cx);
            }
            Act::OpenFolder(key) => {
                let Some(path) = self.lib.as_ref().and_then(|l| l.get(&key)).and_then(|g| g.details.install_path.clone()) else { return };
                let dir = std::path::Path::new(&path);
                let target = if dir.is_file() { dir.parent().unwrap_or(dir) } else { dir };
                self.open_in_file_manager(target, cx);
            }
        }
    }

    fn open_in_file_manager(&mut self, dir: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(err) = std::process::Command::new("xdg-open").arg(dir).spawn() {
            self.show_toast(format!("{err}"), true, cx);
        }
    }

    /// Focuses `key` in the Collection (leaving the page that is shown).
    fn palette_select(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Collection {
            self.navigate(Page::Collection, window, cx);
        }
        if self.lib.as_ref().is_some_and(|l| l.get(key).is_some()) {
            self.select(key.to_string().into(), true);
            self.reveal_selected();
        }
        cx.notify();
    }

    /// Called from `render`: opens the palette the debug hook asks for and
    /// drops it when a dialog came up (e.g. the quit question).
    pub(super) fn drive_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.view.is_some() && (self.dialog.is_some() || self.shots.is_open()) {
            self.close_palette(window, cx);
        }
        if self.palette.debug_done || self.lib.is_none() {
            return;
        }
        self.palette.debug_done = true;
        let Ok(spec) = std::env::var("HERONITE_DEBUG_PALETTE") else { return };
        let (query, filter) = spec.split_once('@').unwrap_or((spec.as_str(), ""));
        let query = if query == "-" { "" } else { query };
        if let Some(view) = self.open_palette_with(query, window, cx) {
            let n = match filter {
                "pages" => 1,
                "actions" => 2,
                "games" => 3,
                _ => 0,
            };
            for _ in 0..n {
                view.update(cx, |p, cx| p.cycle_filter(false, cx));
            }
            // `HERONITE_DEBUG_PALETTE_KEYS=down,down,tab`: keys fed to the palette.
            if let Ok(keys) = std::env::var("HERONITE_DEBUG_PALETTE_KEYS") {
                for k in keys.split(',').filter(|k| !k.is_empty()) {
                    if let Ok(ks) = Keystroke::parse(k) {
                        self.palette_key(&ks, window, cx);
                    }
                }
            }
        }
    }

    pub(super) fn render_palette(&self) -> Option<AnyView> {
        self.palette.view.clone().map(Into::into)
    }
}
