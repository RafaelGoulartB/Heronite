//! Command palette commands of the Downloads page (see `pages/commands.rs`).

use gpui_kit::*;
use heronite_epic::manager::DmState;
use heronite_i18n::{tr, trn};

use super::DownloadsPage;
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

impl PaletteCommands for DownloadsPage {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let Some(svc) = crate::services::downloads_started(cx) else { return Vec::new() };
        let s = svc.read(cx);
        let snap = s.snapshot();
        let current = snap.queue.first().map(|e| e.params.title());
        let idle = current.is_none().then_some(tr!("Nothing is downloading"));
        let what = current.clone().unwrap_or_default();
        let mut v = Vec::new();
        v.push(
            match (current.is_some(), snap.state) {
                (true, DmState::Running) | (false, _) => PageCommand::new("pause", tr!("Pause downloads"), what.clone(), Icon::Pause)
                    .keywords(tr!("hold queue epic"))
                    .disabled(idle),
                (true, _) => PageCommand::new("resume", tr!("Resume downloads"), what.clone(), Icon::PlayArrow)
                    .keywords(tr!("continue start queue epic")),
            }
            .stay(),
        );
        v.push(
            PageCommand::new("stop", tr!("Stop the current download\u{2026}"), what, Icon::Stop)
                .keywords(tr!("cancel abort queue epic"))
                .disabled(idle),
        );
        let updates = s.updates_count();
        v.push(
            PageCommand::new(
                "update-all",
                tr!("Update all games"),
                trn!(updates, "{n} update available", "{n} updates available"),
                Icon::Update,
            )
            .keywords(tr!("upgrade patch queue"))
            .disabled((updates == 0).then_some(tr!("No updates available")))
            .stay(),
        );
        v.push(
            PageCommand::new("clear", tr!("Clear completed downloads"), tr!("Empty the download history"), Icon::DeleteOutline)
                .keywords(tr!("history finished done"))
                .disabled(snap.finished.is_empty().then_some(tr!("No completed downloads")))
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
