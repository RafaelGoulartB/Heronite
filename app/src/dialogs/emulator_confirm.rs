//! Confirmations of the Emulators page: removing an emulator (and, as
//! Heronite's `removeUserEmulator` does, the ROM folders that use it; games
//! stay in the Collection) or a saved ROM folder.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::HeroicPaths;
use heronite_emulation::{remove_rom_scanner, remove_user_emulator};

use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmAction {
    RemoveEmulator { id: String },
    RemoveScanner { id: String },
}

pub struct EmulatorConfirmDialog {
    paths: HeroicPaths,
    action: ConfirmAction,
    title: String,
    message: String,
    working: bool,
    error: Option<SharedString>,
}

impl EventEmitter<DialogEvent> for EmulatorConfirmDialog {}

/// Text of the emulator removal confirmation.
pub fn remove_emulator_message(name: &str, games: usize, folders: usize) -> String {
    let mut text = format!("Remove {name}?");
    if folders > 0 {
        text.push_str(&format!(
            " Its {folders} ROM folder{} will be removed too.",
            if folders == 1 { "" } else { "s" }
        ));
    }
    match games {
        0 => {}
        1 => text.push_str(" 1 game uses it. It stays in your Collection and keeps the executable it was added with."),
        n => text.push_str(&format!(
            " {n} games use it. They stay in your Collection and keep the executable they were added with."
        )),
    }
    text
}

impl EmulatorConfirmDialog {
    pub fn remove_emulator(paths: HeroicPaths, id: String, name: &str, games: usize, folders: usize) -> Self {
        Self {
            paths,
            action: ConfirmAction::RemoveEmulator { id },
            title: "Remove emulator".into(),
            message: remove_emulator_message(name, games, folders),
            working: false,
            error: None,
        }
    }

    pub fn remove_scanner(paths: HeroicPaths, id: String, directory: &str) -> Self {
        Self {
            paths,
            action: ConfirmAction::RemoveScanner { id },
            title: "Remove ROM folder".into(),
            message: format!("Stop scanning {directory}? Games already added stay in the Collection."),
            working: false,
            error: None,
        }
    }

    /// Debug helper: answers "Yes".
    pub fn debug_yes(&mut self, cx: &mut Context<Self>) {
        self.confirm(cx);
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        self.working = true;
        let (paths, action) = (self.paths.clone(), self.action.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match action {
                        ConfirmAction::RemoveEmulator { id } => remove_user_emulator(&paths, &id).map(|_| ()),
                        ConfirmAction::RemoveScanner { id } => remove_rom_scanner(&paths, &id).map(|_| ()),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.working = false;
                match res {
                    Ok(()) => cx.emit(DialogEvent::ChangedAndClose),
                    Err(e) => {
                        this.error = Some(format!("{e:#}").into());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for EmulatorConfirmDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(paragraph(self.message.clone()))
            .children(self.error.clone().map(|e| div().text_color(theme::danger()).child(e)));
        let footer = div()
            .flex()
            .gap(px(16.))
            .child(button("emu-confirm-no", ButtonKind::Outline, "No").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                button("emu-confirm-yes", ButtonKind::Default, if self.working { "Removing\u{2026}" } else { "Yes" })
                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
            );
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "emulator-confirm".into(), title: self.title.clone().into(), width: 520., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::remove_emulator_message;

    #[test]
    fn message_mentions_folders_and_games() {
        assert_eq!(remove_emulator_message("PCSX2", 0, 0), "Remove PCSX2?");
        assert_eq!(
            remove_emulator_message("PCSX2", 1, 1),
            "Remove PCSX2? Its 1 ROM folder will be removed too. 1 game uses it. It stays in your Collection and keeps the executable it was added with."
        );
        assert!(remove_emulator_message("X", 12, 2).contains("12 games use it. They stay in your Collection"));
    }
}
