//! "A game is running": asked before the app quits while games are running or
//! post-game work (backups, cloud saves) is in flight. The launcher is the one
//! that records the playtime and runs the post-game tasks when a game ends, so
//! quitting now keeps the playtime so far but skips those tasks.
//!
//! Buttons: with a tray icon "Keep running in tray" hides the window and keeps
//! the app (and the games' bookkeeping) alive; without one "Keep open" just
//! closes the question. "Quit anyway" runs the graceful quit of the hub.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::DialogEvent;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;

pub struct QuitConfirmDialog {
    games: Vec<String>,
    post_game: bool,
    /// The game being installed from a script.
    install: Option<String>,
    tray: bool,
    console_list: crate::console::list::ListNav,
}

impl EventEmitter<DialogEvent> for QuitConfirmDialog {}

/// `"A"`, `"A" and "B"`, `"A", "B" and "C"`.
pub fn name_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => {
            let init = init.join(", ");
            tr!("{init} and {last}", init, last)
        }
    }
}

/// The body of the question.
pub fn message(games: &[String], post_game: bool) -> String {
    if games.is_empty() {
        return tr!("Post-game tasks (backups, cloud saves) are still running. If you quit now they are interrupted.").into();
    }
    let subject = name_list(games);
    let mut text = if games.len() == 1 {
        tr!(
            "{subject} is still running. Heronite records the playtime and runs the post-game tasks (backups, cloud saves) when it ends. If you quit now, the playtime so far is saved and the game keeps running, but those tasks will not run.",
            subject
        )
    } else {
        tr!(
            "{subject} are still running. Heronite records the playtime and runs the post-game tasks (backups, cloud saves) when they end. If you quit now, the playtime so far is saved and the game keeps running, but those tasks will not run.",
            subject
        )
    };
    if post_game {
        text.push(' ');
        text.push_str(tr!("Post-game tasks of another game are also still running and would be interrupted."));
    }
    text
}

/// [`message`], with the install from a script that quitting would stop.
pub fn full_message(games: &[String], post_game: bool, install: Option<&str>) -> String {
    let Some(title) = install else { return message(games, post_game) };
    if games.is_empty() && !post_game {
        return tr!("{title} is still being installed. If you quit now, the install stops and the game is not added.", title);
    }
    let mut text = message(games, post_game);
    text.push(' ');
    text.push_str(&tr!("The install of {title} would also be stopped.", title));
    text
}

impl QuitConfirmDialog {
    pub fn new(games: Vec<String>, post_game: bool, install: Option<String>, tray: bool) -> Self {
        Self { games, post_game, install, tray, console_list: Default::default() }
    }

    fn title(&self) -> &'static str {
        if self.games.is_empty() && !self.post_game && self.install.is_some() {
            tr!("A game is being installed")
        } else {
            tr!("A game is running")
        }
    }

    fn text(&self) -> String {
        full_message(&self.games, self.post_game, self.install.as_deref())
    }
}

impl Render for QuitConfirmDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            let rows = self.console_rows();
            let text = self.text();
            let title = self.title();
            return crate::console::list::simple_dialog(&mut self.console_list, "console-quit", title, &text, None, &rows);
        }
        let body = div().flex().flex_col().gap(px(14.)).child(paragraph(self.text()));
        let mut footer = div().flex().gap(px(10.));
        if self.tray {
            footer = footer
                .child(
                    button("quit-cancel", ButtonKind::Ghost, tr!("Cancel"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
                )
                .child(button("quit-tray", ButtonKind::Primary, tr!("Keep running in tray")).on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(DialogEvent::Close);
                    crate::system::keep_in_tray(cx);
                })));
        } else {
            footer = footer.child(
                button("quit-keep", ButtonKind::Primary, tr!("Keep open")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
            );
        }
        footer = footer.child(button("quit-anyway", ButtonKind::Danger, tr!("Quit anyway")).on_click(cx.listener(|_, _, _, cx| {
            cx.emit(DialogEvent::Close);
            crate::system::quit_anyway(cx);
        })));
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "quit-confirm".into(), title: self.title().into(), width: 560., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}

impl QuitConfirmDialog {
    fn console_rows(&self) -> Vec<crate::console::list::Row> {
        use crate::console::list::Row;
        let mut rows = Vec::new();
        if self.tray {
            rows.push(Row::action("tray", tr!("Keep running in tray")));
            rows.push(Row::action("cancel", tr!("Cancel")));
        } else {
            rows.push(Row::action("cancel", tr!("Keep open")));
        }
        rows.push(Row::action("quit", tr!("Quit anyway")).danger());
        rows
    }
}

impl crate::console::PadTarget for QuitConfirmDialog {
    fn on_pad(&mut self, pad: crate::console::Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::console::list::Outcome;
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                cx.emit(DialogEvent::Close);
                match id.as_ref() {
                    "tray" => crate::system::keep_in_tray(cx),
                    "quit" => crate::system::quit_anyway(cx),
                    _ => {}
                }
            }
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
    use super::{full_message, message, name_list};

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn names_are_listed_in_plain_english() {
        assert_eq!(name_list(&v(&["Control"])), "\"Control\"");
        assert_eq!(name_list(&v(&["A", "B"])), "\"A\" and \"B\"");
        assert_eq!(name_list(&v(&["A", "B", "C"])), "\"A\", \"B\" and \"C\"");
    }

    #[test]
    fn the_message_names_the_games_and_what_is_skipped() {
        let one = message(&v(&["Control"]), false);
        assert!(one.starts_with("\"Control\" is still running."));
        assert!(one.contains("playtime") && one.contains("backups, cloud saves") && one.contains("when it ends"));
        let two = message(&v(&["A", "B"]), false);
        assert!(two.starts_with("\"A\" and \"B\" are still running.") && two.contains("when they end"));
        assert!(message(&v(&["A"]), true).contains("another game"));
        assert!(message(&[], true).starts_with("Post-game tasks"));
    }

    #[test]
    fn a_running_install_is_mentioned() {
        assert!(full_message(&[], false, Some("Doom")).starts_with("Doom is still being installed."));
        let both = full_message(&v(&["A"]), false, Some("Doom"));
        assert!(both.starts_with("\"A\" is still running.") && both.ends_with("The install of Doom would also be stopped."));
        assert_eq!(full_message(&v(&["A"]), false, None), message(&v(&["A"]), false));
    }
}
