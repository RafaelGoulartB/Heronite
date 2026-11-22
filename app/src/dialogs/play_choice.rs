//! `PlayChoiceDialog` ("Choose how to start the game"): Play of a game that has
//! actions counting as playing (`heronite_data::game_actions`) asks which
//! way to start it: the game's own Play or one of those actions (an emulator
//! or a recompiled port, online or offline...).
//!
//! A click, Enter or the row's number plays; ↑ ↓ move. The row picked last
//! time for the game starts focused, so Enter repeats it.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::DialogEvent;
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, kbd, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::icon;

/// One way to start the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The action's id; `None`: the game's own Play.
    pub action: Option<String>,
    pub label: String,
    /// What it starts ("Steam", "mGBA", "mkrecomp.exe").
    pub note: String,
    pub icon: Icon,
}

pub struct PlayChoiceDialog {
    title: String,
    choices: Vec<Choice>,
    focus: usize,
    on_play: Rc<dyn Fn(Option<String>, &mut App)>,
    on_edit: Rc<dyn Fn(&mut Window, &mut App)>,
    console_list: ListNav,
}

impl EventEmitter<DialogEvent> for PlayChoiceDialog {}

impl PlayChoiceDialog {
    /// `last`: the action picked last time (`Some(None)`: Play), focused first.
    pub fn new(
        title: String,
        choices: Vec<Choice>,
        last: Option<Option<String>>,
        on_play: Rc<dyn Fn(Option<String>, &mut App)>,
        on_edit: Rc<dyn Fn(&mut Window, &mut App)>,
    ) -> Self {
        let focus = last.and_then(|l| choices.iter().position(|c| c.action == l)).unwrap_or(0);
        let mut console_list = ListNav::default();
        console_list.focus = focus;
        Self { title, choices, focus, on_play, on_edit, console_list }
    }

    /// Runs `on_play` once the current update is over: a key reaches the
    /// dialog while the Collection handles it, and `on_play` updates the
    /// Collection.
    fn play(&self, i: usize, cx: &mut App) {
        if let Some(c) = self.choices.get(i) {
            let (f, action) = (self.on_play.clone(), c.action.clone());
            cx.defer(move |cx| f(action, cx));
        }
    }

    /// ↑ ↓ (and Tab) move, Enter or Space plays, 1–9 play that row.
    pub fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let n = self.choices.len();
        if n == 0 {
            return false;
        }
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        // The Enter that opened the dialog, held, does not pick a row.
        if ev.is_held || m.control || m.alt || m.platform {
            return false;
        }
        match ks.key.as_str() {
            "up" => self.focus = (self.focus + n - 1) % n,
            "tab" if ks.modifiers.shift => self.focus = (self.focus + n - 1) % n,
            "down" | "tab" => self.focus = (self.focus + 1) % n,
            "home" => self.focus = 0,
            "end" => self.focus = n - 1,
            "enter" | "space" => self.play(self.focus, cx),
            k if k.len() == 1 && (b'1'..=b'9').contains(&k.as_bytes()[0]) => {
                let i = (k.as_bytes()[0] - b'1') as usize;
                if i >= n {
                    return false;
                }
                self.focus = i;
                self.play(i, cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn row(&self, i: usize, c: &Choice, cx: &mut Context<Self>) -> Stateful<Div> {
        let on = i == self.focus;
        div()
            .id(SharedString::from(format!("play-choice-{i}")))
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(12.))
            .rounded(px(theme::R_TILE))
            .border_1()
            .border_color(if on { theme::accent_tint_border() } else { theme::card_border() })
            .bg(if on { theme::accent_tint() } else { theme::card_fill() })
            .cursor_pointer()
            .hover(|s| s.bg(if on { theme::accent_tint() } else { theme::ink_a(0.08) }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.focus = i;
                this.play(i, cx);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(36.))
                    .rounded_full()
                    .bg(if on { Hsla::from(theme::accent()) } else { theme::ink_a(0.08) })
                    .child(icon(c.icon, px(20.), if on { theme::play_text() } else { theme::text_primary() })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_weight(theme::W_BOLD)
                            .text_size(px(15.))
                            .text_color(theme::text_primary())
                            .child(c.label.clone()),
                    )
                    .when(!c.note.is_empty(), |d| {
                        d.child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(c.note.clone()))
                    }),
            )
            .when(i < 9, |d| d.child(div().flex_none().child(kbd((i + 1).to_string()))))
    }

    fn console_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .choices
            .iter()
            .enumerate()
            .map(|(i, c)| Row::action(format!("c:{i}"), c.label.clone()).desc(c.note.clone()).icon(c.icon))
            .collect();
        rows.push(Row::action("cancel", tr!("Cancel")));
        rows
    }
}

impl Render for PlayChoiceDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            let rows = self.console_rows();
            let body = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(u(12.))
                .child(kit::body_text(tr!("Choose how to start the game.")))
                .child(self.console_list.render("console-play-choice", &rows, true).flex_1().min_h_0());
            return kit::modal(self.title.clone(), 760., body).into_any_element();
        }
        let mut list = div().flex().flex_col().gap(px(8.));
        for (i, c) in self.choices.iter().enumerate() {
            list = list.child(self.row(i, c, cx));
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().text_size(px(14.)).text_color(theme::text_muted()).child(tr!("Choose how to start the game.")))
            .child(list);
        let on_edit = self.on_edit.clone();
        let footer = div()
            .flex()
            .flex_1()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(
                button("play-choice-edit", ButtonKind::Ghost, tr!("Edit actions\u{2026}"))
                    .on_click(cx.listener(move |_, _, window, cx| (on_edit)(window, cx))),
            )
            .child(
                button("play-choice-cancel", ButtonKind::Ghost, tr!("Cancel"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
            )
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "play-choice".into(),
                title: self.title.clone().into(),
                width: 480f32.min(f32::from(crate::ui::content_size(window).width) * 0.9) + 64.,
                height: None,
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

impl PadTarget for PlayChoiceDialog {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                if id.as_ref() == "cancel" {
                    cx.emit(DialogEvent::Close);
                } else if let Some(i) = id.strip_prefix("c:").and_then(|n| n.parse::<usize>().ok()) {
                    self.focus = i;
                    self.play(i, cx);
                }
            }
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![hint(Btn::A, tr!("Play")), hint(Btn::B, tr!("Cancel"))]
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::{Choice, PlayChoiceDialog};
    use crate::icons::Icon;

    fn choice(action: Option<&str>) -> Choice {
        Choice { action: action.map(str::to_owned), label: "x".into(), note: String::new(), icon: Icon::PlayArrow }
    }

    fn dialog_with(last: Option<Option<String>>) -> PlayChoiceDialog {
        let choices = vec![choice(None), choice(Some("a")), choice(Some("b"))];
        PlayChoiceDialog::new("G".into(), choices, last, Rc::new(|_, _| {}), Rc::new(|_, _| {}))
    }

    #[test]
    fn the_last_pick_starts_focused() {
        assert_eq!(dialog_with(None).focus, 0);
        assert_eq!(dialog_with(Some(Some("b".into()))).focus, 2);
        assert_eq!(dialog_with(Some(None)).focus, 0);
        // An action removed since: back to Play.
        assert_eq!(dialog_with(Some(Some("gone".into()))).focus, 0);
        assert_eq!(dialog_with(Some(Some("b".into()))).console_list.focus, 2);
    }
}
