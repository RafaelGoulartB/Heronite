//! "Remove from Collection" confirmation (`CollectionCard.handleRemoveFromCollection`).
//! The game is not uninstalled; its Collection data, playtime and art are
//! deleted (see `heronite_metadata::remove`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_metadata::remove::{remove_collection_game, ShortcutDirs};

use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;

pub const TITLE: &str = "Remove from Collection";

pub struct RemoveGameDialog {
    ctx: DialogCtx,
    app_name: String,
    runner: String,
    title: String,
    working: bool,
    error: Option<SharedString>,
}

impl EventEmitter<DialogEvent> for RemoveGameDialog {}

/// The confirmation text.
pub fn confirm_message(title: &str) -> String {
    format!(
        "Remove {title} from Collection? This does not uninstall the game. Playtime, status, and Collection images for it are deleted."
    )
}

impl RemoveGameDialog {
    pub fn new(ctx: DialogCtx, app_name: String, runner: String, title: String) -> Self {
        Self { ctx, app_name, runner, title, working: false, error: None }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        self.working = true;
        self.error = None;
        let (paths, app_name, runner) = (self.ctx.heroic.clone(), self.app_name.clone(), self.runner.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    // `removeShortcuts` + `removeNonSteamGame` first: they look the game up
                    // in the library, which the removal below empties. Failures only log.
                    let outcome = crate::system::shortcuts::remove_shortcuts(&paths, &runner, &app_name);
                    log::info!("shortcuts of {app_name}: {outcome:?}");
                    if crate::system::shortcuts::is_added_to_steam(&paths, &runner, &app_name) {
                        let outcome = crate::system::shortcuts::remove_from_steam(&paths, &runner, &app_name);
                        log::info!("steam shortcut of {app_name}: {outcome:?}");
                    }
                    remove_collection_game(&paths, &ShortcutDirs::none(), &app_name, &runner).map_err(|e| format!("{e:#}"))
                })
                .await;
            this.update(cx, |this, cx| {
                this.working = false;
                match result {
                    Ok(_) => cx.emit(DialogEvent::ChangedAndClose),
                    Err(message) => {
                        this.error = Some(
                            if message.is_empty() { "Could not remove this game from Collection.".to_string() } else { message }
                                .into(),
                        );
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

impl Render for RemoveGameDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(paragraph(confirm_message(&self.title)))
            .children(self.error.clone().map(|e| div().text_color(theme::danger_text()).child(e)));
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("remove-no", ButtonKind::Ghost, "Cancel").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                button("remove-yes", ButtonKind::Danger, if self.working { "Removing…" } else { "Remove" })
                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
            );
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "remove-game".into(), title: TITLE.into(), width: 520., height: None },
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
    use super::confirm_message;

    #[test]
    fn message_matches_heronite() {
        assert_eq!(
            confirm_message("Portal 2"),
            "Remove Portal 2 from Collection? This does not uninstall the game. Playtime, status, and Collection images for it are deleted."
        );
    }
}
