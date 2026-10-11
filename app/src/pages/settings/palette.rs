//! Command palette commands of Settings (see `pages/commands.rs`): the
//! Appearance toggles, run on the Appearance section so the change shows.

use gpui_kit::*;

use super::{appearance, Section, SettingsPage};
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

impl PaletteCommands for SettingsPage {
    fn palette_commands(page: Option<&Self>, ctx: &CommandCtx, _cx: &App) -> Vec<PageCommand> {
        appearance::palette_commands(page, ctx.settings)
    }

    fn run_palette_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.select(Section::Appearance, window, cx);
        appearance::run_palette(self, id, cx);
        cx.notify();
    }
}
