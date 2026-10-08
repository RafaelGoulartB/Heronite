//! Add game / Edit game glue of the Collection: opens the dialog from the ☰
//! menu, the game context menu and the debug hook. The work is in
//! `dialogs::add_game`.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::CollectionView;
use crate::dialogs::add_game::{AddGameDialog, Mode};

impl CollectionView {
    pub(super) fn open_add_game(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<AddGameDialog>> {
        self.open_add_game_mode(Mode::Add, window, cx)
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
}
