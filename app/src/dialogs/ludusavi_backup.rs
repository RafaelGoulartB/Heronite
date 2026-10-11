//! Manual "Backup save (Ludusavi)" of one game (`CollectionCard.handleLudusaviBackup`):
//! runs the same backup as the after-play one (`heronite_emulation::ludusavi`)
//! and tells how it went. The runner is injected so tests and screenshots use
//! a fake binary.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::ludusavi::{
    backup_for_game, BackupReason, CommandRunner, LudusaviBackupResult, LudusaviEnv, LudusaviGame,
};
use heronite_launch::SystemProbe;

use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;

pub const TITLE: &str = "Ludusavi save backup";
pub const RUNNING: &str = "Backing up saves with Ludusavi…";

/// What the dialog says when the backup finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Running,
    /// Plain information (busy, no saves matched).
    Message(String),
    Error(String),
    /// Backup saved; `path` enables "Show in folder".
    Done { message: String, path: Option<String> },
}

/// Port of the branches of `handleLudusaviBackup`.
pub fn describe(result: &LudusaviBackupResult, title: &str) -> Outcome {
    if result.skipped_reason.as_deref() == Some("already-running") {
        return Outcome::Message("Ludusavi is already backing up a save. Wait a moment and try again.".into());
    }
    if let (Some(error), false) = (&result.error, result.ran) {
        return if result.skipped_reason.as_deref() == Some("no-match") {
            Outcome::Message(error.clone())
        } else {
            Outcome::Error(error.clone())
        };
    }
    let name = result.game_name.as_deref().filter(|n| !n.is_empty()).unwrap_or(title);
    match result.backup_path.as_deref().filter(|p| !p.is_empty()) {
        Some(path) => Outcome::Done {
            message: format!("Saved a Ludusavi backup for {name} in {path}."),
            path: Some(path.to_string()),
        },
        None => Outcome::Done { message: format!("Saved a Ludusavi backup for {name}."), path: None },
    }
}

pub struct LudusaviBackupDialog {
    outcome: Outcome,
    console_list: crate::console::list::ListNav,
}

impl EventEmitter<DialogEvent> for LudusaviBackupDialog {}

impl LudusaviBackupDialog {
    /// A dialog that already shows `outcome` (screenshots).
    pub fn showing(outcome: Outcome) -> Self {
        Self { outcome, console_list: Default::default() }
    }

    /// Starts the backup right away on a worker thread.
    pub fn new(ctx: DialogCtx, game: LudusaviGame, runner: Arc<dyn CommandRunner>, cx: &mut Context<Self>) -> Self {
        let title = game.title.clone();
        let paths = ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let env = LudusaviEnv { paths: &paths, runner: runner.as_ref(), probe: &SystemProbe };
                    backup_for_game(&env, &game, BackupReason::Manual)
                })
                .await;
            this.update(cx, |this, cx| {
                this.outcome = describe(&result, &title);
                // The backup is recorded in settings.json (last backup/error).
                cx.emit(DialogEvent::Changed);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self { outcome: Outcome::Running, console_list: Default::default() }
    }
}

/// Opens `path` (or its parent when it does not exist) in the file manager.
fn show_in_folder(path: &str) {
    let path = std::path::Path::new(path);
    let target = if path.is_dir() { Some(path) } else { path.parent() };
    if let Some(target) = target {
        if let Err(err) = std::process::Command::new("xdg-open").arg(target).spawn() {
            log::warn!("xdg-open {}: {err}", target.display());
        }
    }
}

impl Render for LudusaviBackupDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (text, is_error, path) = match &self.outcome {
            Outcome::Running => (RUNNING.to_string(), false, None),
            Outcome::Message(m) => (m.clone(), false, None),
            Outcome::Error(m) => (m.clone(), true, None),
            Outcome::Done { message, path } => (message.clone(), false, path.clone()),
        };
        if crate::console::active() {
            let rows = vec![crate::console::list::Row::action("close", "Close")];
            let error = is_error.then(|| SharedString::from(text.clone()));
            let message = if is_error { String::new() } else { text };
            return crate::console::list::simple_dialog(&mut self.console_list, "console-ludusavi", TITLE, &message, error, &rows);
        }
        let body = div()
            .flex()
            .flex_col()
            .child(paragraph(text).when(is_error, |d| d.text_color(theme::danger_text())));
        let mut footer = div().flex().gap(px(10.));
        if let Some(path) = path {
            footer = footer
                .child(button("ludusavi-show", ButtonKind::Glass, "Show in folder").on_click(move |_, _, _| show_in_folder(&path)));
        }
        footer = footer
            .child(button("ludusavi-close", ButtonKind::Primary, "Close").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))));
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "ludusavi-backup".into(), title: TITLE.into(), width: 560., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}

impl crate::console::PadTarget for LudusaviBackupDialog {
    fn on_pad(&mut self, pad: crate::console::Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        if pad == crate::console::Pad::A {
            cx.emit(DialogEvent::Close);
            return true;
        }
        false
    }

    fn hints(&self, _: &App) -> Vec<crate::console::Hint> {
        vec![crate::console::hint(crate::console::Btn::A, "Close")]
    }
}

#[cfg(test)]
mod tests {
    use super::{describe, LudusaviBackupResult, Outcome};

    fn result(f: impl FnOnce(&mut LudusaviBackupResult)) -> LudusaviBackupResult {
        let mut r = LudusaviBackupResult::default();
        f(&mut r);
        r
    }

    #[test]
    fn busy_no_match_and_errors() {
        let busy = result(|r| r.skipped_reason = Some("already-running".into()));
        assert_eq!(
            describe(&busy, "G"),
            Outcome::Message("Ludusavi is already backing up a save. Wait a moment and try again.".into())
        );
        let no_match = result(|r| {
            r.skipped_reason = Some("no-match".into());
            r.error = Some("nothing".into());
        });
        assert_eq!(describe(&no_match, "G"), Outcome::Message("nothing".into()));
        let failed = result(|r| r.error = Some("Ludusavi was not found".into()));
        assert_eq!(describe(&failed, "G"), Outcome::Error("Ludusavi was not found".into()));
    }

    #[test]
    fn saved_with_and_without_path() {
        let with = result(|r| {
            r.ran = true;
            r.game_name = Some("Portal 2".into());
            r.backup_path = Some("/b/Portal 2".into());
        });
        assert_eq!(
            describe(&with, "Fallback"),
            Outcome::Done {
                message: "Saved a Ludusavi backup for Portal 2 in /b/Portal 2.".into(),
                path: Some("/b/Portal 2".into())
            }
        );
        let without = result(|r| r.ran = true);
        assert_eq!(
            describe(&without, "Fallback"),
            Outcome::Done { message: "Saved a Ludusavi backup for Fallback.".into(), path: None }
        );
    }
}
