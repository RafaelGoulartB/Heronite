//! The bars of a picture: the tabs, the clock and the status icons at the
//! top (Spotlight's tabs as pills centred over the screen, its clock
//! larger), the tab bar along the bottom, and the button hints (monochrome
//! glyphs, the menu and the tabs on the left, the actions on the right).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Clock, Glyphs, Hints, Layout, TabBar};
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

    /// A keycap (LB, RB) as the console draws it: outlined in the text colour.
    pub fn key(&self, text: &'static str) -> Div {
        div()
            .flex_none()
            .h(self.u(9.))
            .min_w(self.u(12.))
            .px(self.u(2.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(self.u(3.))
            .border(self.u(0.7))
            .border_color(Rgba { a: 0.75, ..self.text })
            .text_size(self.u(4.5))
            .font_weight(theme::W_BOLD)
            .text_color(self.text)
            .child(text)
    }

    /// A face button as the hints draw it: a disc in the text colour with
    /// its mark cut out (a keycap for the keyboard).
    fn mono_glyph(&self, text: &'static str) -> Div {
        if text.chars().count() > 1 {
            return self.key(text);
        }
        div()
            .flex_none()
            .size(self.u(9.5))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(self.text)
            .text_size(self.u(5.5))
            .font_weight(theme::W_BOLD)
            .text_color(self.bg)
            .child(text)
    }

    /// Whether the tab shown draws its tabs as a centred pill (Spotlight, Cinema).
    fn spotlight_tab(&self) -> bool {
        self.t.tabs.get(self.tab).is_some_and(|t| matches!(t.layout, Layout::Spotlight | Layout::Cinema))
    }

    /// Spotlight's tabs: the names between LB and RB, the current one in a
    /// pill of the focus colour, centred.
    fn spotlight_tabs(&self) -> Div {
        let mut tabs = div().flex().items_center().gap(self.u(2.)).child(self.key("LB").mr(self.u(4.)));
        for (n, name) in self.tab_names().into_iter().enumerate() {
            let on = n == self.tab;
            tabs = tabs.child(div().px(self.u(6.5)).py(self.u(2.5)).rounded_full().when(on, |d| d.bg(self.accent)).child(self.label(
                name,
                7.,
                if on { self.on_fill(self.accent) } else { self.muted },
            )));
        }
        let tabs = tabs.child(self.key("RB").ml(self.u(4.)));
        div().absolute().left_0().right_0().top(self.u(10.)).flex().justify_center().child(tabs)
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
            TabBar::Top if self.spotlight_tab() => {}
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
        let spot = self.spotlight_tab() && i.tab_bar == TabBar::Top;
        // Spotlight's clock is larger.
        let clock = if spot { 12. } else { 10. };
        match i.clock {
            Clock::H24 => right = right.child(self.label("21:30", clock, self.text)),
            Clock::H12 => right = right.child(self.label("9:30", clock, self.text)),
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
            .when(spot && names.len() > 1, |d| d.child(self.spotlight_tabs()))
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
        // The controller's buttons (not the theme's): south and east, the
        // one that confirms first, and X.
        let (south, east, west, north): (&'static str, &'static str, &'static str, &'static str) = match crate::console::look::glyphs() {
            Glyphs::Xbox | Glyphs::Auto | Glyphs::Deck => ("A", "B", "X", "Y"),
            Glyphs::PlayStation => ("\u{2715}", "\u{25cb}", "\u{25a1}", "\u{25b3}"),
            Glyphs::Nintendo => ("B", "A", "Y", "X"),
            Glyphs::Keyboard => ("Enter", "Esc", "X", "Y"),
        };
        let keys = crate::console::look::glyphs() == Glyphs::Keyboard;
        let (a, b) = if crate::console::look::confirm_east() && !keys { (east, south) } else { (south, east) };
        let bottom = if self.t.interface.tab_bar == TabBar::Bottom { 38. } else { 8. };
        let hint =
            |g: Div, text: &str| div().flex().items_center().gap(self.u(3.5)).child(g).child(self.label(text.to_string(), 6.5, self.text));
        let menu =
            div().flex_none().size(self.u(9.5)).flex().items_center().justify_center().rounded_full().bg(self.text).child(crate::ui::icon(
                crate::icons::Icon::Menu,
                self.u(6.5),
                self.bg,
            ));
        let tabs =
            (self.t.tabs.len() > 1).then(|| hint(div().flex().gap(self.u(2.)).child(self.key("LB")).child(self.key("RB")), tr!("Tabs")));
        let left = div().flex().items_center().gap(self.u(12.)).child(hint(menu, tr!("Menu"))).children(tabs);
        let cinema = self.t.tabs.get(self.tab).is_some_and(|t| t.layout == Layout::Cinema);
        // A and X as the theme's `nav.aButton` says, as on the console.
        let details_on_a = self.t.nav.a_button == heronite_data::console_theme::AButton::Details;
        // Cinema: Y the search, A / X the page and Play as the setting says;
        // Spotlight's row: A opens the actions, X plays; elsewhere A and B.
        let right = if cinema {
            div()
                .flex()
                .items_center()
                .gap(self.u(12.))
                .child(hint(self.mono_glyph(north), tr!("Search")))
                .child(hint(self.mono_glyph(west), if details_on_a { tr!("Play") } else { tr!("Details") }))
                .child(hint(self.mono_glyph(a), if details_on_a { tr!("Details") } else { tr!("Play") }))
        } else if self.spotlight_tab() {
            div()
                .flex()
                .items_center()
                .gap(self.u(12.))
                .child(hint(self.mono_glyph(a), tr!("Actions")))
                .child(hint(self.mono_glyph(west), tr!("Play")))
        } else {
            div()
                .flex()
                .items_center()
                .gap(self.u(12.))
                .child(hint(self.mono_glyph(a), tr!("Select")))
                .child(hint(self.mono_glyph(b), tr!("Back")))
        };
        // Hints that fade by themselves show faint.
        let faint = c.hints == Hints::Auto;
        Some(
            div()
                .absolute()
                .left(self.u(18. + self.inset.0))
                .right(self.u(18. + self.inset.1))
                .bottom(self.u(bottom))
                .flex()
                .items_center()
                .justify_between()
                .when(faint, |d| d.opacity(0.6))
                .child(left)
                .child(right),
        )
    }
}
