//! "How do you want to play?" in console mode: the emulator's save states
//! (with their thumbnails) and "Start from the beginning" as large rows; A
//! launches the focused one.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::LaunchStateDialog;
use crate::console::kit::{self, u};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

impl LaunchStateDialog {
    /// States, then "Start from the beginning".
    fn console_count(&self) -> usize {
        self.rows.len() + 1
    }

    fn console_focus(&self) -> usize {
        self.selected.unwrap_or(self.rows.len())
    }

    fn console_move(&mut self, down: bool) {
        let at = self.console_focus();
        let to = if down { (at + 1).min(self.console_count() - 1) } else { at.saturating_sub(1) };
        self.selected = (to < self.rows.len()).then_some(to);
        self.console_scroll.scroll_to_item(to);
    }

    pub(super) fn render_console(&mut self) -> AnyElement {
        let focus = self.console_focus();
        let thumb = |content: AnyElement| {
            div()
                .w(u(112.))
                .h(u(84.))
                .flex_none()
                .rounded(u(10.))
                .overflow_hidden()
                .bg(theme::black_a(0.35))
                .flex()
                .items_center()
                .justify_center()
                .text_size(u(22.))
                .child(content)
        };
        let mut list =
            div().id("console-launch-states").flex().flex_col().gap(u(6.)).overflow_y_scroll().track_scroll(&self.console_scroll);
        for (i, row) in self.rows.iter().enumerate() {
            let on = focus == i;
            let fg: Hsla = if on { theme::console_on_focus().into() } else { theme::text_primary().into() };
            let content: AnyElement = match &row.thumb {
                Some(img) => gpui_kit::img(ImageSource::Render(img.clone())).size_full().object_fit(ObjectFit::Cover).into_any_element(),
                None if row.resume => icon(Icon::PlayArrow, u(32.), white()).into_any_element(),
                None => div().text_color(white()).child(row.glyph.clone()).into_any_element(),
            };
            list = list.child(
                kit::row_base(on).py(u(8.)).child(thumb(content)).child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().text_size(u(19.)).text_color(fg).child(row.label.clone()))
                        .child(div().text_size(u(14.)).text_color(Hsla { a: 0.7, ..fg }).child(row.when.clone())),
                ),
            );
        }
        let on = focus == self.rows.len();
        let fg: Hsla = if on { theme::console_on_focus().into() } else { theme::text_primary().into() };
        list = list.child(
            kit::row_base(on).py(u(8.)).child(thumb(icon(Icon::Replay, u(32.), white()).into_any_element())).child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().text_size(u(19.)).text_color(fg).child(tr!("Start from the beginning")))
                    .child(
                        div()
                            .text_size(u(14.))
                            .text_color(Hsla { a: 0.7, ..fg })
                            .child(tr!("Boot the game normally. In-game saves on memory cards are kept.")),
                    ),
            ),
        );
        let body =
            div().flex().flex_col().flex_1().min_h_0().gap(u(12.)).child(kit::body_text(self.intro())).child(list.flex_1().min_h_0());
        kit::modal(tr!("How do you want to play?"), 760., body).into_any_element()
    }
}

impl PadTarget for LaunchStateDialog {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        match pad {
            Pad::Up | Pad::Down => self.console_move(pad == Pad::Down),
            Pad::A => self.play(cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![hint(Btn::A, if self.selected.is_none() { tr!("Start") } else { tr!("Continue") }), hint(Btn::B, tr!("Cancel"))]
    }
}
