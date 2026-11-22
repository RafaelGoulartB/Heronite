//! Drawing of the PIN dialog: the prompt, one dot per digit and the pad, as a
//! glass dialog or, in console mode, a console modal with the cursor's ring.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hidden_lock::PIN_LEN;
use heronite_i18n::tr;

use super::{HiddenPinDialog, DELETE_KEY, EMPTY_KEY, KEYS, ZERO_KEY};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::icon;

/// The digit key `i` of the pad shows (the delete key draws an icon).
fn key_label(i: usize) -> SharedString {
    match i {
        0..=8 => SharedString::from((i + 1).to_string()),
        ZERO_KEY => "0".into(),
        _ => "".into(),
    }
}

impl HiddenPinDialog {
    /// A dot per digit typed, and empty ones up to the shortest PIN.
    fn dots(&self, size: Pixels, gap: Pixels) -> Div {
        let slots = self.entry.len().max(*PIN_LEN.start());
        div().flex().justify_center().gap(gap).h(size).children((0..slots).map(|i| {
            let filled = i < self.entry.len();
            div()
                .size(size)
                .rounded_full()
                .border_2()
                .border_color(if filled { theme::accent().into() } else { theme::ink_a(0.35) })
                .when(filled, |d| d.bg(theme::accent()))
        }))
    }

    /// The error, the wait after wrong PINs, or a line kept empty so the pad
    /// does not move.
    fn status_line(&self, size: Pixels) -> Div {
        let (text, error): (SharedString, bool) = match (self.wait_text(), &self.error) {
            _ if self.forgot_asking => (tr!("Enter this computer's password in the window that opened.").into(), false),
            _ if self.forgot_help.is_some() => (self.forgot_help.clone().unwrap_or_default(), false),
            (Some(wait), _) => (wait, true),
            (None, Some(e)) => (e.clone(), true),
            (None, None) if self.checking => (tr!("Checking\u{2026}").into(), false),
            (None, None) => ("".into(), false),
        };
        div()
            .min_h(size * 1.4)
            .text_size(size)
            .line_height(size * 1.4)
            .text_center()
            .text_color(if error { theme::danger_text() } else { theme::text_muted() })
            .child(text)
    }

    fn pad(&self, side: Pixels, gap: Pixels, text: Pixels, focus: Option<usize>, cx: &mut Context<Self>) -> Div {
        let waiting = self.waiting();
        let rows = (0..KEYS / 3).map(|row| {
            div().flex().justify_center().gap(gap).children((0..3).map(|col| {
                let i = row * 3 + col;
                let mut key = div().id(("pin-key", i)).relative().flex().items_center().justify_center().size(side).rounded_full();
                if i == EMPTY_KEY {
                    return key.into_any_element();
                }
                key = key.bg(theme::ink_a(0.08)).text_size(text).font_family(theme::font_heading()).text_color(theme::text_primary());
                key = if i == DELETE_KEY { key.child(icon(Icon::Backspace, text, theme::text_primary())) } else { key.child(key_label(i)) };
                if waiting {
                    key = key.opacity(0.45);
                } else {
                    key = key
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::ink_a(0.16)))
                        .active(|s| s.bg(theme::ink_a(0.24)))
                        .on_click(cx.listener(move |this, _, _, cx| this.press(i, cx)));
                }
                if focus == Some(i) {
                    key = key.child(crate::console::kit::focus_ring(side.into()));
                }
                key.into_any_element()
            }))
        });
        div().flex().flex_col().gap(gap).children(rows)
    }

    /// Digits, Backspace and Enter; false for any other key.
    pub fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        match ev.keystroke.key.as_str() {
            "backspace" => self.delete(cx),
            "enter" => self.submit(cx),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => self.type_digit(k.as_bytes()[0] as char, cx),
            _ => return false,
        }
        true
    }

    fn console_render(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use crate::console::kit::{self, u};
        let body = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(u(16.))
            .child(kit::body_text(self.prompt()).text_center())
            .child(self.dots(u(18.), u(16.)))
            .child(self.status_line(u(15.)))
            .child(self.pad(u(76.), u(14.), u(28.), Some(self.key), cx));
        kit::modal(self.title(), 520., body).into_any_element()
    }
}

impl Render for HiddenPinDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.console_render(cx);
        }
        let body = div()
            .id("hidden-pin")
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .pb(px(4.))
            .child(div().flex().items_center().justify_center().size(px(48.)).rounded_full().bg(theme::accent_alpha(0.14)).child(icon(
                Icon::Lock,
                px(24.),
                theme::accent(),
            )))
            .child(
                div()
                    .max_w(px(320.))
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .text_center()
                    .text_color(theme::text_muted())
                    .child(self.prompt()),
            )
            .child(div().pt(px(4.)).child(self.dots(px(14.), px(12.))))
            .child(self.status_line(px(13.)))
            .child(self.pad(px(60.), px(12.), px(22.), None, cx))
            .children(self.can_forget().then(|| {
                div()
                    .id("hidden-pin-forgot")
                    .pt(px(2.))
                    .text_size(px(13.))
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .on_click(cx.listener(|this, _, _, cx| this.forgot(cx)))
                    .child(tr!("Forgot the PIN?"))
            }));
        let ready = !self.waiting() && heronite_data::hidden_lock::valid_pin(&self.entry);
        let kind = if self.mode == super::PinMode::Remove && ready { ButtonKind::Danger } else { ButtonKind::Primary };
        let mut go = button("hidden-pin-go", kind, self.primary_label());
        go = if ready { go.on_click(cx.listener(|this, _, _, cx| this.submit(cx))) } else { go.opacity(0.45) };
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(
                button("hidden-pin-cancel", ButtonKind::Ghost, tr!("Cancel"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
            )
            .child(go);
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "hidden-pin-dialog".into(), title: self.title(), width: 400., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
