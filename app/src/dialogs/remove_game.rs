//! "Remove from Collection" confirmation (`CollectionCard.handleRemoveFromCollection`).
//! The game is not uninstalled; its Collection data, playtime and art are
//! deleted (see `heronite_metadata::remove`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::remove::{remove_collection_game, ShortcutDirs};

use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;

pub struct RemoveGameDialog {
    ctx: DialogCtx,
    app_name: String,
    runner: String,
    title: String,
    working: bool,
    error: Option<SharedString>,
    console_list: crate::console::list::ListNav,
}

impl EventEmitter<DialogEvent> for RemoveGameDialog {}

pub fn title() -> &'static str {
    tr!("Remove from Collection")
}

/// The confirmation text.
pub fn confirm_message(title: &str) -> String {
    tr!(
        "Remove {title} from Collection? This does not uninstall the game. Playtime, status, and Collection images for it are deleted.",
        title
    )
}

impl RemoveGameDialog {
    pub fn new(ctx: DialogCtx, app_name: String, runner: String, title: String) -> Self {
        Self { ctx, app_name, runner, title, working: false, error: None, console_list: Default::default() }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        self.working = true;
        self.error = None;
        let (paths, app_name, runner) = (self.ctx.data_paths.clone(), self.app_name.clone(), self.runner.clone());
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
                            if message.is_empty() { tr!("Could not remove this game from Collection.").to_string() } else { message }
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
        if crate::console::active() {
            let rows = self.console_rows();
            return crate::console::list::simple_dialog(
                &mut self.console_list,
                "console-remove-game",
                title(),
                &confirm_message(&self.title),
                self.error.clone(),
                &rows,
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(paragraph(confirm_message(&self.title)))
            .children(self.error.clone().map(|e| div().text_color(theme::danger_text()).child(e)));
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("remove-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                button("remove-yes", ButtonKind::Danger, if self.working { tr!("Removing…") } else { tr!("Remove") })
                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
            );
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "remove-game".into(), title: title().into(), width: 520., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}

impl RemoveGameDialog {
    fn console_rows(&self) -> Vec<crate::console::list::Row> {
        use crate::console::list::Row;
        vec![
            Row::action("cancel", tr!("Cancel")),
            Row::action("remove", if self.working { tr!("Removing…") } else { tr!("Remove") }).danger().enabled(!self.working),
        ]
    }
}

impl crate::console::PadTarget for RemoveGameDialog {
    fn on_pad(&mut self, pad: crate::console::Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::console::list::Outcome;
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) if id.as_ref() == "remove" => self.confirm(cx),
            Outcome::Activate(_) => cx.emit(DialogEvent::Close),
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<crate::console::Hint> {
        use crate::console::{hint, Btn};
        vec![hint(Btn::A, tr!("Select")), hint(Btn::B, tr!("Cancel"))]
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
