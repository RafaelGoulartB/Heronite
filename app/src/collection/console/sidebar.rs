//! The console menu (Start / Menu, or B on a top screen): a full-height
//! sidebar over the dimmed screen with every page, Add game,
//! Settings and the way back to desktop mode.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::super::CollectionView;
use super::Screen;
use crate::console::kit::{self, u};
use crate::console::motion;
use crate::console::Pad;
use crate::icons::Icon;
use crate::pages::Page;
use crate::theme;
use crate::ui::icon;
use heronite_i18n::tr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Item {
    Go(Page),
    AddGame,
    Desktop,
    Separator,
}

impl Item {
    pub fn page(self) -> Option<Page> {
        match self {
            Item::Go(p) => Some(p),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Item::Go(p) => p.title(),
            Item::AddGame => tr!("Add game"),
            Item::Desktop => tr!("Desktop mode"),
            Item::Separator => "",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Item::Go(p) => p.icon(),
            Item::AddGame => Icon::Add,
            Item::Desktop => Icon::Logout,
            Item::Separator => Icon::Menu,
        }
    }
}

pub(super) const ITEMS: [Item; 13] = [
    Item::Go(Page::Collection),
    Item::Go(Page::Downloads),
    Item::Go(Page::WineManager),
    Item::Go(Page::Epic),
    Item::Go(Page::Gog),
    Item::Go(Page::Amazon),
    Item::Go(Page::Emulators),
    Item::Go(Page::Statistics),
    Item::Separator,
    Item::AddGame,
    Item::Go(Page::Settings),
    Item::Separator,
    Item::Desktop,
];

fn step(from: usize, down: bool) -> usize {
    let mut i = from;
    loop {
        let next = if down { i + 1 } else { i.wrapping_sub(1) };
        if next >= ITEMS.len() {
            return from;
        }
        i = next;
        if ITEMS[i] != Item::Separator {
            return i;
        }
    }
}

impl CollectionView {
    pub(super) fn sidebar_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cur) = self.console.sidebar else { return };
        match pad {
            Pad::Up | Pad::Down => self.console.sidebar = Some(step(cur, pad == Pad::Down)),
            Pad::B | Pad::Start | Pad::Right => self.console.sidebar = None,
            Pad::A => {
                self.console.sidebar = None;
                match ITEMS[cur] {
                    Item::Go(Page::Collection) => {
                        self.navigate(Page::Collection, window, cx);
                        if matches!(self.console.screen, Screen::Game(_)) {
                            self.console.screen = Screen::Home;
                        }
                    }
                    Item::Go(page) => self.navigate(page, window, cx),
                    Item::AddGame => self.console_add_game(window, cx),
                    Item::Desktop => self.set_console(false, window, cx),
                    Item::Separator => {}
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn render_sidebar(&self, cx: &App) -> Option<AnyElement> {
        let focused = self.console.sidebar?;
        let downloads = crate::services::try_downloads(cx).and_then(|s| super::super::nav::downloads_meta(Some(s.read(cx))));
        let mut list = div().flex().flex_col().gap(u(2.));
        for (i, item) in ITEMS.iter().enumerate() {
            if *item == Item::Separator {
                list = list.child(div().my(u(6.)).mx(u(18.)).h(px(1.)).bg(theme::white_a(0.1)));
                continue;
            }
            let on = i == focused;
            let current = item.page() == Some(self.page);
            let fg: Hsla = if on {
                theme::console_on_focus().into()
            } else if current {
                white()
            } else {
                theme::text_primary().into()
            };
            let badge = match item {
                Item::Go(Page::Downloads) => downloads.clone(),
                _ => None,
            };
            list = list.child(
                kit::row_base(on)
                    .h(u(46.))
                    .child(icon(item.icon(), u(26.), if on { fg } else { theme::text_muted().into() }))
                    .child(
                        div()
                            .flex_1()
                            .text_size(u(21.))
                            .text_color(fg)
                            .when(current && !on, |d| d.font_weight(theme::W_BOLD))
                            .child(item.label()),
                    )
                    .children(badge.map(|b| kit::badge(b, theme::console_btn_a()))),
            );
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .child(kit::scrim())
                .child(motion::enter(
                    "console-sidebar",
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(u(360.))
                        .flex()
                        .flex_col()
                        .px(u(16.))
                        .pt(u(30.))
                        .bg(theme::console_sidebar())
                        .border_r_1()
                        .border_color(theme::white_a(0.08))
                        .shadow(kit::shadow(0.5, 0., 40.))
                        .child(div().px(u(18.)).pb(u(26.)).child(kit::wordmark("HERONITE")))
                        .child(list),
                    280,
                    -40.,
                    0.,
                ))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{step, Item, ITEMS};

    #[test]
    fn steps_skip_separators_and_stop_at_the_ends() {
        let add = ITEMS.iter().position(|i| *i == Item::AddGame).unwrap();
        assert_eq!(step(add - 2, true), add);
        assert_eq!(step(add, false), add - 2);
        assert_eq!(step(0, false), 0);
        assert_eq!(step(ITEMS.len() - 1, true), ITEMS.len() - 1);
    }
}
