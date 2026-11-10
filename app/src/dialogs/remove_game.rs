//! "Remove from Collection" confirmation (`CollectionCard.handleRemoveFromCollection`),
//! for one game or for the games of a selection. The games are not
//! uninstalled; their Collection data, playtime and art are deleted (see
//! `heronite_metadata::remove`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::clients::{self, Client};
use heronite_data::launchers::Launcher;
use heronite_data::{DataPaths, Library};
use heronite_i18n::{tr, trn};
use heronite_metadata::remove::{remove_collection_game, RemoveOutcome, ShortcutDirs};

use super::metadata_update::hub::{hub, HubGlobal};
use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{meta_text, paragraph};

/// How many titles the confirmation of several games lists by name.
const LISTED: usize = 6;

/// A game the dialog takes out of the Collection.
#[derive(Clone, Debug)]
pub struct Target {
    pub app_name: String,
    pub runner: String,
    pub title: String,
}

pub struct RemoveGameDialog {
    ctx: DialogCtx,
    targets: Vec<Target>,
    /// Selected games that cannot be removed (store games, running games).
    skipped: usize,
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

/// The confirmation text for several games: how many, the first titles by
/// name, and how many of the selected games stay.
pub fn batch_message(titles: &[&str], skipped: usize) -> String {
    let n = titles.len();
    let mut lines = vec![trn!(
        n,
        "Remove {n} game from Collection? This does not uninstall it. Playtime, status, and Collection images for it are deleted.",
        "Remove {n} games from Collection? This does not uninstall them. Playtime, status, and Collection images for them are deleted."
    )
    .to_string()];
    lines.extend(titles.iter().take(LISTED).map(|t| format!("\u{2022} {t}")));
    if n > LISTED {
        lines.push(trn!(n - LISTED, "\u{2022} and {n} more", "\u{2022} and {n} more").to_string());
    }
    if skipped > 0 {
        lines.push(
            trn!(
                skipped,
                "{n} other selected game stays: games from a store account, and running games, cannot be removed.",
                "{n} other selected games stay: games from a store account, and running games, cannot be removed."
            )
            .to_string(),
        );
    }
    lines.join("\n")
}

/// Takes a game out of the Collection: its desktop and menu shortcuts and
/// its Steam shortcut first (`removeShortcuts` + `removeNonSteamGame`: they
/// look the game up in the library the removal empties; failures only log),
/// then its Collection data. Blocking.
pub fn remove_from_collection(paths: &DataPaths, runner: &str, app_name: &str) -> anyhow::Result<RemoveOutcome> {
    let outcome = crate::system::shortcuts::remove_shortcuts(paths, runner, app_name);
    log::info!("shortcuts of {app_name}: {outcome:?}");
    if crate::system::shortcuts::is_added_to_steam(paths, runner, app_name) {
        let outcome = crate::system::shortcuts::remove_from_steam(paths, runner, app_name);
        log::info!("steam shortcut of {app_name}: {outcome:?}");
    }
    remove_collection_game(paths, &ShortcutDirs::none(), app_name, runner)
}

/// Who brought a game into the Collection and keeps it in sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Launcher(Launcher),
    Client(Client),
    /// The Xbox app (Microsoft Store and PC Game Pass games).
    Xbox,
}

/// The launcher (Heroic, Lutris, Bottles, Faugus, Playnite) or the store client a game
/// of `lib` was brought in from, whose import must leave it out once it is
/// removed.
pub fn launcher_of(lib: &Library, app_name: &str) -> Option<Origin> {
    let game = lib.games.iter().find(|g| g.app_name == app_name)?;
    let meta = game.meta.as_ref()?;
    if meta.launch_kind == heronite_data::xbox::LAUNCH_KIND {
        return Some(Origin::Xbox);
    }
    match Client::from_id(&meta.source).filter(|_| meta.launch_kind == clients::LAUNCH_KIND) {
        Some(client) => Some(Origin::Client(client)),
        None => Launcher::from_source(&meta.source).map(Origin::Launcher),
    }
}

/// Tells the import of the launcher, or the sync of the store client, that
/// the user removed `app_name`.
pub fn note_removed(origin: Option<Origin>, app_name: &str, cx: &mut App) {
    match origin {
        Some(Origin::Launcher(launcher)) => {
            let svc = crate::services::launcher_imports(cx);
            svc.update(cx, |s, cx| s.note_removed(launcher, app_name, cx));
        }
        Some(Origin::Client(client)) => {
            let svc = crate::services::clients(cx);
            svc.update(cx, |s, cx| s.note_removed(client, app_name, cx));
        }
        Some(Origin::Xbox) => {
            let svc = crate::services::xbox_library(cx);
            svc.update(cx, |s, cx| s.note_removed(app_name, cx));
        }
        None => {}
    }
}

impl RemoveGameDialog {
    pub fn new(ctx: DialogCtx, app_name: String, runner: String, title: String) -> Self {
        Self::many(ctx, vec![Target { app_name, runner, title }], 0)
    }

    /// Removes every game of `targets`; `skipped` selected games stay.
    pub fn many(ctx: DialogCtx, targets: Vec<Target>, skipped: usize) -> Self {
        Self { ctx, targets, skipped, working: false, error: None, console_list: Default::default() }
    }

    /// Confirming several games (or a selection that leaves some out).
    fn is_batch(&self) -> bool {
        self.targets.len() > 1 || self.skipped > 0
    }

    fn message(&self) -> String {
        match self.targets.as_slice() {
            [one] if !self.is_batch() => confirm_message(&one.title),
            targets => batch_message(&targets.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), self.skipped),
        }
    }

    fn remove_label(&self) -> SharedString {
        if self.working {
            tr!("Removing…").into()
        } else if self.is_batch() {
            trn!(self.targets.len(), "Remove {n} game", "Remove {n} games").into()
        } else {
            tr!("Remove").into()
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.working || self.targets.is_empty() {
            return;
        }
        self.working = true;
        self.error = None;
        let paths = self.ctx.data_paths.clone();
        let targets = self.targets.clone();
        let origins: Vec<Option<Origin>> = targets.iter().map(|t| launcher_of(&self.ctx.lib, &t.app_name)).collect();
        cx.spawn(async move |this, cx| {
            let results: Vec<Result<(), String>> = cx
                .background_spawn(async move {
                    targets
                        .iter()
                        .map(|t| remove_from_collection(&paths, &t.runner, &t.app_name).map(|_| ()).map_err(|e| format!("{e:#}")))
                        .collect()
                })
                .await;
            this.update(cx, |this, cx| {
                this.working = false;
                // A removed game leaves the review queue (the header pill's).
                if cx.has_global::<HubGlobal>() {
                    hub(cx).update(cx, |hub, cx| hub.read_queue(cx));
                }
                let mut failed = Vec::new();
                let mut first_error = None;
                for ((target, origin), result) in this.targets.iter().zip(origins).zip(results) {
                    match result {
                        Ok(()) => note_removed(origin, &target.app_name, cx),
                        Err(message) => {
                            first_error.get_or_insert(message);
                            failed.push(target.clone());
                        }
                    }
                }
                let Some(message) = first_error else {
                    cx.emit(DialogEvent::ChangedAndClose);
                    return;
                };
                let removed = this.targets.len() - failed.len();
                this.error = Some(
                    if !this.is_batch() {
                        if message.is_empty() {
                            tr!("Could not remove this game from Collection.").to_string()
                        } else {
                            message
                        }
                    } else {
                        trn!(failed.len(), "Could not remove {n} game: {err}", "Could not remove {n} games: {err}", err = message)
                            .to_string()
                    }
                    .into(),
                );
                // What is left to try again; the removed ones leave the list.
                this.targets = failed;
                if removed > 0 {
                    cx.emit(DialogEvent::Changed);
                }
                cx.notify();
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
            let message = self.message();
            return crate::console::list::simple_dialog(
                &mut self.console_list,
                "console-remove-game",
                title(),
                &message,
                self.error.clone(),
                &rows,
            );
        }
        let message = self.message();
        let mut lines = message.lines();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .children(lines.next().map(|l| paragraph(l.to_string())))
            .when(self.is_batch(), |d| {
                d.child(
                    div()
                        .id("remove-list")
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .children(lines.map(|l| meta_text(l.to_string()).text_size(px(13.)).line_height(px(19.)))),
                )
            })
            .children(self.error.clone().map(|e| div().text_color(theme::danger_text()).child(e)));
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("remove-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(button("remove-yes", ButtonKind::Danger, self.remove_label()).on_click(cx.listener(|this, _, _, cx| this.confirm(cx))));
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
        vec![Row::action("cancel", tr!("Cancel")), Row::action("remove", self.remove_label()).danger().enabled(!self.working)]
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
    use super::{batch_message, confirm_message};

    #[test]
    fn message_matches_heronite() {
        assert_eq!(
            confirm_message("Portal 2"),
            "Remove Portal 2 from Collection? This does not uninstall the game. Playtime, status, and Collection images for it are deleted."
        );
    }

    #[test]
    fn batch_message_lists_the_first_titles_and_the_games_that_stay() {
        let titles = ["A", "B", "C", "D", "E", "F", "G", "H"];
        let m = batch_message(&titles, 2);
        let lines: Vec<&str> = m.lines().collect();
        assert!(lines[0].starts_with("Remove 8 games from Collection?"));
        assert_eq!(&lines[1..7], ["\u{2022} A", "\u{2022} B", "\u{2022} C", "\u{2022} D", "\u{2022} E", "\u{2022} F"]);
        assert_eq!(lines[7], "\u{2022} and 2 more");
        assert!(lines[8].starts_with("2 other selected games stay"));
        assert_eq!(lines.len(), 9);
    }

    #[test]
    fn batch_message_of_a_few_games_has_no_more_line() {
        let m = batch_message(&["Portal", "Celeste"], 0);
        assert_eq!(m.lines().count(), 3);
        assert!(m.starts_with("Remove 2 games from Collection?"));
    }
}
