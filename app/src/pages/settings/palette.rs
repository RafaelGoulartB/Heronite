//! Command palette commands of Settings (see `pages/commands.rs`): the
//! Appearance toggles and the app updates, run on their section so the
//! change shows.

use gpui_kit::*;

use super::{appearance, updates, Section, SettingsPage};
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

impl PaletteCommands for SettingsPage {
    fn palette_commands(page: Option<&Self>, ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let mut v = appearance::palette_commands(page, ctx.settings);
        v.extend(updates::palette_commands(cx));
        v
    }

    fn run_palette_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if id.starts_with("upd:") {
            self.select(Section::About, window, cx);
            updates::run_palette(id, cx);
            return;
        }
        self.select(Section::Appearance, window, cx);
        appearance::run_palette(self, id, cx);
        cx.notify();
    }

    fn close_modal(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let closed = self.dismiss_confirms();
        if closed {
            cx.notify();
        }
        closed
    }
}
