//! On-screen keyboard for console mode: a bottom sheet with a key grid
//! driven by the d-pad (A types, X deletes, Y space, LT shift, Start done,
//! B cancels). A physical keyboard types into it too
//! ([`Osk::type_key`]).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::kit::{self, u};
use super::{hint, Btn, Hint, Pad, PadTarget};
use crate::theme;

pub enum OskEvent {
    Done(String),
    Cancel,
}

const ROWS: [&str; 5] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm,.-", "/:?=&_%#+@"];
/// Bottom row: Shift, Space, Delete, Done.
const SPECIAL: [&str; 4] = ["Shift", "Space", "\u{232b}", "Done"];
const SPECIAL_ROW: usize = ROWS.len();

pub struct Osk {
    title: SharedString,
    text: String,
    row: usize,
    col: usize,
    shift: bool,
}

impl EventEmitter<OskEvent> for Osk {}

impl Osk {
    pub fn new(title: impl Into<SharedString>, initial: &str) -> Self {
        Self { title: title.into(), text: initial.to_string(), row: 1, col: 0, shift: false }
    }

    fn key_at(&self, row: usize, col: usize) -> char {
        let c = ROWS[row].chars().nth(col).unwrap_or(' ');
        if self.shift {
            c.to_ascii_uppercase()
        } else {
            c
        }
    }

    fn row_len(row: usize) -> usize {
        if row == SPECIAL_ROW {
            SPECIAL.len()
        } else {
            ROWS[row].chars().count()
        }
    }

    /// Moves the cursor; returns false at the edges.
    fn step(&mut self, pad: Pad) {
        let (r, c) = (self.row, self.col);
        match pad {
            Pad::Left => self.col = if c == 0 { Self::row_len(r) - 1 } else { c - 1 },
            Pad::Right => self.col = (c + 1) % Self::row_len(r),
            Pad::Up if r > 0 => {
                self.row = r - 1;
                if r == SPECIAL_ROW {
                    self.col = (c * 10 + 5) / SPECIAL.len();
                }
            }
            Pad::Down if r < SPECIAL_ROW => {
                self.row = r + 1;
                if self.row == SPECIAL_ROW {
                    self.col = (c * SPECIAL.len() / 10).min(SPECIAL.len() - 1);
                }
            }
            _ => {}
        }
        self.col = self.col.min(Self::row_len(self.row) - 1);
    }

    fn press(&mut self, cx: &mut Context<Self>) {
        if self.row == SPECIAL_ROW {
            match self.col {
                0 => self.shift = !self.shift,
                1 => self.text.push(' '),
                2 => {
                    self.text.pop();
                }
                _ => cx.emit(OskEvent::Done(self.text.clone())),
            }
        } else {
            let c = self.key_at(self.row, self.col);
            self.text.push(c);
            self.shift = false;
        }
    }

    /// A physical key while the keyboard is open; `true` when used.
    pub fn type_key(&mut self, ks: &Keystroke, cx: &mut Context<Self>) -> bool {
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform {
            return false;
        }
        match ks.key.as_str() {
            "backspace" => {
                self.text.pop();
            }
            "enter" => cx.emit(OskEvent::Done(self.text.clone())),
            "escape" => cx.emit(OskEvent::Cancel),
            "space" => self.text.push(' '),
            _ => match ks.key_char.as_deref() {
                Some(s) if !s.is_empty() && s.chars().all(|c| !c.is_control()) => self.text.push_str(s),
                _ => return false,
            },
        }
        cx.notify();
        true
    }
}

impl PadTarget for Osk {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        match pad {
            Pad::Up | Pad::Down | Pad::Left | Pad::Right => self.step(pad),
            Pad::A => self.press(cx),
            Pad::X => {
                self.text.pop();
            }
            Pad::Y => self.text.push(' '),
            Pad::LT | Pad::LB => self.shift = !self.shift,
            Pad::Start | Pad::RT => cx.emit(OskEvent::Done(self.text.clone())),
            Pad::B => cx.emit(OskEvent::Cancel),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![
            hint(Btn::A, "Type"),
            hint(Btn::X, "Delete"),
            hint(Btn::Y, "Space"),
            hint(Btn::LT, "Shift"),
            hint(Btn::Start, "Done"),
            hint(Btn::B, "Cancel"),
        ]
    }
}

fn key(label: String, focused: bool, wide: f32, on: bool) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_grow(1.)
        .flex_basis(u(wide))
        .h(u(52.))
        .rounded(u(12.))
        .text_size(u(19.))
        .when(focused, |d| d.relative().text_color(theme::console_on_focus()).font_weight(theme::W_BOLD).child(kit::focus_pill(12.)))
        .when(!focused, |d| d.bg(if on { theme::accent_tint() } else { theme::white_a(0.07) }).text_color(theme::text_primary()))
        .child(div().relative().child(label))
}

impl Render for Osk {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut grid = div().flex().flex_col().gap(u(8.));
        for (r, line) in ROWS.iter().enumerate() {
            grid = grid.child(div().flex().gap(u(8.)).children(
                line.chars().enumerate().map(|(c, _)| key(self.key_at(r, c).to_string(), self.row == r && self.col == c, 56., false)),
            ));
        }
        grid = grid.child(div().flex().gap(u(8.)).children(SPECIAL.iter().enumerate().map(|(i, label)| {
            let wide = if i == 1 { 300. } else { 110. };
            key(label.to_string(), self.row == SPECIAL_ROW && self.col == i, wide, i == 0 && self.shift)
        })));
        let field = div()
            .flex()
            .items_center()
            .h(u(56.))
            .px(u(20.))
            .rounded(u(14.))
            .bg(theme::black_a(0.35))
            .border_1()
            .border_color(theme::white_a(0.12))
            .text_size(u(20.))
            .text_color(theme::text_primary())
            .child(format!("{}\u{2502}", self.text));
        div().absolute().inset_0().child(kit::scrim()).child(
            div().absolute().left_0().right_0().bottom(u(70.)).flex().justify_center().child(kit::panel_in(
                "osk-panel",
                div()
                    .w(u(820.))
                    .flex()
                    .flex_col()
                    .gap(u(16.))
                    .p(u(24.))
                    .rounded(u(24.))
                    .bg(theme::console_sidebar())
                    .border_1()
                    .border_color(theme::white_a(0.1))
                    .child(kit::title(self.title.clone(), 22.))
                    .shadow(kit::shadow(0.5, 18., 48.))
                    .child(field)
                    .child(grid),
            )),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Osk, SPECIAL_ROW};
    use crate::console::Pad;

    #[test]
    fn cursor_moves_between_letters_and_the_bottom_row() {
        let mut k = Osk::new("Search", "");
        assert_eq!((k.row, k.col), (1, 0));
        k.step(Pad::Left);
        assert_eq!(k.col, 9, "wraps");
        k.step(Pad::Down);
        k.step(Pad::Down);
        k.step(Pad::Down);
        k.step(Pad::Down);
        assert_eq!((k.row, k.col), (SPECIAL_ROW, 3));
        k.step(Pad::Down);
        assert_eq!(k.row, SPECIAL_ROW);
        k.step(Pad::Up);
        assert_eq!(k.row, SPECIAL_ROW - 1);
        assert_eq!(k.key_at(1, 0), 'q');
        k.shift = true;
        assert_eq!(k.key_at(1, 0), 'Q');
    }
}
