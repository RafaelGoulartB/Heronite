//! Add game / Edit game glue of the Collection: opens the dialog from the ☰
//! menu, the game context menu and the debug hook, and "Add games from a
//! folder" from the ☰ menu and the palette. The work is in
//! `dialogs::add_game` and `dialogs::scan_folder`.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::CollectionView;
use crate::dialogs::add_game::{AddGameDialog, Mode};
use crate::dialogs::scan_folder::ScanFolderDialog;
use crate::icons::Icon;
use crate::theme;

impl CollectionView {
    pub(super) fn open_add_game(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<AddGameDialog>> {
        self.open_add_game_mode(Mode::Add, window, cx)
    }

    /// Files dropped on the window: the first one opens "Add game" filled in.
    /// Only the Collection page takes it, with no dialog open.
    /// Whether a file dropped now opens Add game.
    fn takes_drop(&self) -> bool {
        self.page == crate::pages::Page::Collection && !crate::console::active() && self.dialog.is_none()
    }

    pub(super) fn drop_external_paths(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if !self.takes_drop() {
            return;
        }
        let Some(first) = paths.paths().first() else { return };
        if let Some(view) = self.open_add_game(window, cx) {
            view.update(cx, |d, cx| d.prefill_from_path(first, window, cx));
        }
    }

    /// Edit dialog of a sideloaded game (`key` is the card key).
    pub(super) fn open_edit_game(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<AddGameDialog>> {
        let game = self.lib.as_ref()?.get(key)?.clone();
        self.open_add_game_mode(Mode::Edit(game.app_name.clone()), window, cx)
    }

    fn open_add_game_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<AddGameDialog>> {
        self.open_menu = None;
        self.context_menu = None;
        let ctx = self.dialog_ctx()?;
        let view = cx.new(|cx| AddGameDialog::new(ctx, mode, window, cx));
        self.open_dialog(view.clone(), cx);
        Some(view)
    }

    pub(super) fn open_scan_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<ScanFolderDialog>> {
        self.open_menu = None;
        self.context_menu = None;
        let ctx = self.dialog_ctx()?;
        let view = cx.new(|cx| ScanFolderDialog::new(ctx, window, cx));
        self.open_dialog(view.clone(), cx);
        Some(view)
    }

    /// `HERONITE_DEBUG_DIALOG=scan-folder[:<folder>[#<states>]]`: the dialog,
    /// with that folder searched (states are `+`-joined, see
    /// `ScanFolderDialog::debug_state`).
    pub(super) fn scan_folder_debug_dialog(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        let rest = which.strip_prefix("scan-folder").unwrap_or("").trim_start_matches(':');
        let (folder, states) = rest.split_once('#').unwrap_or((rest, ""));
        if let Some(view) = self.open_scan_folder(window, cx) {
            if !folder.is_empty() {
                view.update(cx, |d, cx| d.debug_state(folder, states, window, cx));
            }
        }
    }

    /// `HERONITE_DEBUG_DIALOG=add-game[:<state>]` / `edit-game:<appName>[:<state>]`
    /// (states are `+`-joined, see `AddGameDialog::debug_state`). Returns whether
    /// `which` was one of these.
    pub(super) fn add_game_debug_dialog(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (kind, rest) = which.split_once(':').unwrap_or((which, ""));
        let (view, state) = match kind {
            "add-game" => (self.open_add_game(window, cx), rest),
            "edit-game" => {
                let (app, state) = rest.split_once(':').unwrap_or((rest, ""));
                let key = format!("sideload_{app}");
                (self.open_edit_game(&key, window, cx), state)
            }
            _ => return false,
        };
        if let Some(view) = view {
            if !state.is_empty() {
                view.update(cx, |d, cx| d.debug_state(state, window, cx));
            }
        }
        true
    }

    /// The whole window lights up while a file is dragged over it, with what
    /// a drop does. Invisible otherwise; it takes no clicks (no listeners,
    /// not occluding), only the drag-over style.
    pub(super) fn render_drop_target(&self) -> Option<Stateful<Div>> {
        if !self.takes_drop() {
            return None;
        }
        let label = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(18.))
            .py(px(12.))
            .rounded(px(14.))
            .bg(theme::toast_fill())
            .border_1()
            .border_color(theme::accent_alpha(0.6))
            .text_size(px(15.))
            .font_weight(theme::W_BOLD)
            .text_color(theme::text_bright())
            .child(crate::ui::icon(Icon::Add, px(18.), theme::accent()))
            .child(heronite_i18n::tr!("Drop to add a game"));
        Some(
            div()
                .id("drop-target")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .opacity(0.)
                .when(debug_drop(), lit)
                .drag_over::<ExternalPaths>(|s, _, _, _| lit(s))
                .child(label),
        )
    }
}

/// The drop target while a file is over it.
fn lit<S: Styled>(s: S) -> S {
    s.opacity(1.).bg(theme::accent_alpha(0.12)).border_2().border_color(theme::accent_alpha(0.7))
}

/// `HERONITE_DEBUG_DROP=1` shows the drop target as if a file were dragged
/// over the window (screenshots). Read once: the target is built every frame.
fn debug_drop() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| crate::env::flag("HERONITE_DEBUG_DROP"))
}
