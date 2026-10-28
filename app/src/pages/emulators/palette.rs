//! Command palette commands of the Emulators page (see `pages/commands.rs`).

use gpui_kit::*;
use heronite_i18n::tr;

use super::EmulatorsPage;
use crate::icons::Icon;
use crate::pages::commands::{close_dialog, CommandCtx, PageCommand, PaletteCommands};

impl EmulatorsPage {
    /// Runs `id` once the emulators are read (the dialogs need them).
    pub(super) fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.loaded {
            self.palette_pending = Some(id.to_string());
            return;
        }
        match id {
            "add" => self.open_add_emulator(window, cx),
            "detect" => self.open_detect(window, cx),
            "folder" => self.open_folder_dialog(None, None, window, cx),
            _ => {}
        }
    }
}

impl PaletteCommands for EmulatorsPage {
    fn palette_commands(page: Option<&Self>, _ctx: &CommandCtx, _cx: &App) -> Vec<PageCommand> {
        let none = page.is_some_and(|p| p.loaded && p.summaries.is_empty());
        vec![
            PageCommand::new("add", tr!("Add emulator\u{2026}"), "RetroArch, Dolphin, PCSX2\u{2026}", Icon::Add)
                .keywords("new emulation retro console"),
            PageCommand::new(
                "detect",
                tr!("Detect emulators\u{2026}"),
                tr!("Find the emulators installed on this computer"),
                Icon::TravelExplore,
            )
            .keywords("scan find search emulation"),
            PageCommand::new("folder", tr!("Add ROM folder\u{2026}"), tr!("A folder of games for an emulator"), Icon::FolderOpen)
                .keywords("roms directory scan emulation")
                .disabled(none.then_some(tr!("Add an emulator first"))),
        ]
    }

    fn run_palette_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(id, window, cx);
    }

    fn page_modal(&self) -> bool {
        self.dialog.is_some()
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let closed = close_dialog(&mut self.dialog, &mut self.dialog_sub, window, cx);
        cx.notify();
        closed
    }
}
