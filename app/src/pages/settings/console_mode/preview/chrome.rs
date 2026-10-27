//! The bars of a picture: the tabs, the clock and the status icons at the
//! top, the tab bar along the bottom, and the hint bar.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Clock, Glyphs, Hints, TabBar};
use heronite_i18n::tr;

use super::Pic;
use crate::theme;

impl Pic<'_, '_> {
    pub fn tab_names(&self) -> Vec<SharedString> {
        self.t.tabs.iter().map(|t| SharedString::from(tr(&t.name).to_string())).collect()
    }

    /// A controller button: `text` in a small circle (a key on the keyboard).
    pub fn glyph(&self, text: &'static str, fill: Rgba) -> Div {
        let wide = text.chars().count() > 1;
        div()
            .flex_none()
            .h(self.u(11.))
            .min_w(self.u(11.))
            .when(wide, |d| d.px(self.u(3.)))
            .flex()
            .items_center()
            .justify_center()
            .rounded(self.u(if wide { 3. } else { 6. }))
            .bg(fill)
            .text_size(self.u(6.5))
            .text_color(theme::white_a(0.95))
            .font_weight(theme::W_BOLD)
            .child(text)
    }

    pub fn top_bar(&self) -> Div {
        let i = &self.t.interface;
        let names = self.tab_names();
        let mut left = div().flex().items_center().gap(self.u(10.));
        if !i.label.is_empty() {
            left = left.child(self.heading(i.label.clone(), 10.));
        }
        let grey = Rgba { a: 0.5, ..self.text };
        match i.tab_bar {
            TabBar::Top => {
                let mut tabs = div().flex().items_center().gap(self.u(4.)).child(self.glyph("LB", grey));
                for (n, name) in names.iter().enumerate() {
                    let on = n == self.tab;
                    tabs = tabs.child(
                        div()
                            .px(self.u(6.))
                            .py(self.u(2.))
                            .rounded(self.u(8.))
                            .when(on, |d| d.bg(Rgba { a: 0.16, ..self.text }))
                            .child(self.label(name.clone(), 9., if on { self.text } else { self.muted })),
                    );
                }
                left = left.child(tabs.child(self.glyph("RB", grey)));
            }
            TabBar::Names => {
                for (n, name) in names.iter().enumerate() {
                    let on = n == self.tab;
                    left = left.child(
                        self.label(name.clone(), 11., if on { self.text } else { self.muted }).when(on, |d| d.font_weight(theme::W_BOLD)),
                    );
                }
            }
            TabBar::Title | TabBar::Bottom => {}
        }
        let mut right = div().flex().items_center().gap(self.u(6.));
        if i.status_icons {
            right = right.child(div().size(self.u(7. * self.s)).rounded_full().border(self.u(1.)).border_color(self.muted)).child(
                div().w(self.u(10. * self.s)).h(self.u(5. * self.s)).rounded(self.u(1.5)).border(self.u(1.)).border_color(self.muted),
            );
        }
        if i.date && i.clock != Clock::Off {
            right = right.child(self.line(18., 4., 0.7));
        }
        match i.clock {
            Clock::H24 => right = right.child(self.label("21:30", 10., self.text)),
            Clock::H12 => right = right.child(self.label("9:30", 10., self.text)),
            Clock::Off => {}
        }
        let title = (i.tab_bar == TabBar::Title).then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(self.u(14.))
                .flex()
                .justify_center()
                .child(self.heading(names.get(self.tab).cloned().unwrap_or_default(), 11.))
        });
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .child(div().flex().items_center().justify_between().px(self.u(22.)).pt(self.u(14.)).child(left).child(right))
            .children(title)
    }

    pub fn bottom_bar(&self) -> Option<Div> {
        (self.t.interface.tab_bar == TabBar::Bottom).then(|| {
            let mut bar = div().flex().items_center().justify_center().gap(self.u(20.));
            for (n, name) in self.tab_names().into_iter().enumerate() {
                let on = n == self.tab;
                bar = bar.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(self.u(4.))
                        .px(self.u(7.))
                        .py(self.u(3.))
                        .rounded(self.u(8.))
                        .when(on, |d| d.bg(Rgba { a: 0.16, ..self.text }))
                        .child(div().size(self.u(7.)).rounded(self.u(2.)).bg(if on { self.text } else { self.muted }))
                        .child(self.label(name, 8.5, if on { self.text } else { self.muted })),
                );
            }
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(self.u(32.))
                .flex()
                .items_center()
                .justify_center()
                .when(self.inset.0 == 0., |d| d.rounded_bl(px(self.frame_r)))
                .when(self.inset.1 == 0., |d| d.rounded_br(px(self.frame_r)))
                .bg(self.panel)
                .child(bar)
        })
    }

    pub fn hint_bar(&self) -> Option<Div> {
        let c = &self.t.controls;
        if c.hints == Hints::Off {
            return None;
        }
        let (a, b): ((&'static str, Rgba), (&'static str, Rgba)) = match c.glyphs {
            Glyphs::Xbox | Glyphs::Auto => (("A", rgb(0x4f9f45)), ("B", rgb(0xc8453c))),
            Glyphs::PlayStation => (("\u{2715}", rgb(0x5a7bd6)), ("\u{25cb}", rgb(0xd0545f))),
            Glyphs::Nintendo => (("A", rgb(0x4b4f58)), ("B", rgb(0x4b4f58))),
            Glyphs::Deck => (("A", rgb(0x3a3f48)), ("B", rgb(0x3a3f48))),
            Glyphs::Keyboard => (("Enter", rgb(0x3a3f48)), ("Esc", rgb(0x3a3f48))),
        };
        let floating = c.hints == Hints::Auto;
        let bottom = if self.t.interface.tab_bar == TabBar::Bottom { 38. } else { 9. };
        let hint = |g: (&'static str, Rgba), text: &'static str| {
            div().flex().items_center().gap(self.u(4.)).child(self.glyph(g.0, g.1)).child(self.label(text, 8., self.muted))
        };
        Some(
            div().absolute().right(self.u(18.)).bottom(self.u(bottom)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(self.u(10.))
                    .when(floating, |d| d.px(self.u(8.)).py(self.u(4.)).rounded(self.u(10.)).bg(self.panel).opacity(0.45))
                    .child(hint(a, tr!("Select")))
                    .child(hint(b, tr!("Back"))),
            ),
        )
    }
}
