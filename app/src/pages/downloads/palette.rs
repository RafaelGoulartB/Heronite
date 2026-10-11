//! Command palette commands of the Downloads page (see `pages/commands.rs`).

use gpui_kit::*;
use heronite_epic::manager::DmState;

use super::DownloadsPage;
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

impl PaletteCommands for DownloadsPage {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let Some(svc) = crate::services::downloads_started(cx) else { return Vec::new() };
        let s = svc.read(cx);
        let snap = s.snapshot();
        let current = snap.queue.first().map(|e| e.params.title());
        let idle = current.is_none().then_some("Nothing is downloading");
        let what = current.clone().unwrap_or_default();
        let mut v = Vec::new();
        v.push(
            match (current.is_some(), snap.state) {
                (true, DmState::Running) | (false, _) => {
                    PageCommand::new("pause", "Pause downloads", what.clone(), Icon::Pause).keywords("hold queue epic").disabled(idle)
                }
                (true, _) => {
                    PageCommand::new("resume", "Resume downloads", what.clone(), Icon::PlayArrow).keywords("continue start queue epic")
                }
            }
            .stay(),
        );
        v.push(
            PageCommand::new("stop", "Stop the current download\u{2026}", what, Icon::Stop)
                .keywords("cancel abort queue epic")
                .disabled(idle),
        );
        let updates = s.updates_count();
        v.push(
            PageCommand::new("update-all", "Update all Epic games", format!("{updates} update(s) available"), Icon::Update)
                .keywords("upgrade patch queue")
                .disabled((updates == 0).then_some("No Epic updates available"))
                .stay(),
        );
        v.push(
            PageCommand::new("clear", "Clear completed downloads", "Empty the download history", Icon::DeleteOutline)
                .keywords("history finished done")
                .disabled(snap.finished.is_empty().then_some("No completed downloads"))
                .stay(),
        );
        v
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "pause" => self.svc.update(cx, |s, _| s.pause()),
            "resume" => self.svc.update(cx, |s, _| s.resume()),
            // The page's own question: keep the files or delete them.
            "stop" => self.confirm_stop = true,
            "update-all" => self.svc.update(cx, |s, cx| s.update_all(cx)),
            "clear" => self.svc.update(cx, |s, _| s.clear_finished()),
            _ => {}
        }
        cx.notify();
    }
}
