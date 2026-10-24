//! The console menu (Start / Menu, or B on a top screen): every page, Add
//! game, Customize, Settings, the way back to desktop mode and Power, in the
//! order and with the items the theme's `menu` says (Settings and Desktop
//! mode always stay), as a full-height sidebar over the dimmed screen.

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
    /// Epic Games, GOG and Amazon Games: the store page shown last.
    Stores,
    AddGame,
    /// ☰ → Customize: the console theme over the live screen.
    Customize,
    Desktop,
    /// Suspend, restart, shut down (`power.rs`).
    Power,
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
            Item::Stores => crate::pages::stores::title(),
            Item::AddGame => tr!("Add game"),
            Item::Customize => tr!("Customize"),
            Item::Desktop => tr!("Desktop mode"),
            Item::Power => tr!("Power"),
            Item::Separator => "",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Item::Go(p) => p.icon(),
            Item::Stores => Icon::Storefront,
            Item::AddGame => Icon::Add,
            Item::Customize => Icon::Tune,
            Item::Desktop => Icon::Logout,
            Item::Power => Icon::PowerSettingsNew,
            Item::Separator => Icon::Menu,
        }
    }
}

impl Item {
    /// A `MenuItemSpec::item` name.
    pub fn parse(name: &str) -> Option<Item> {
        Some(match name {
            "collection" => Item::Go(Page::Collection),
            "downloads" => Item::Go(Page::Downloads),
            "wine-manager" => Item::Go(Page::WineManager),
            "stores" => Item::Stores,
            "emulators" => Item::Go(Page::Emulators),
            "statistics" => Item::Go(Page::Statistics),
            "separator" => Item::Separator,
            "add-game" => Item::AddGame,
            "customize" => Item::Customize,
            "settings" => Item::Go(Page::Settings),
            "desktop" => Item::Desktop,
            "power" => Item::Power,
            _ => return None,
        })
    }
}

/// The menu of the theme: hidden items left out, no separator at an end or
/// next to another.
pub(super) fn items() -> Vec<Item> {
    let theme = crate::console::look::theme();
    let mut out: Vec<Item> = Vec::new();
    for spec in &theme.menu.items {
        let Some(item) = Item::parse(&spec.item) else { continue };
        if !spec.on && !heronite_data::console_theme::MENU_LOCKED.contains(&spec.item.as_str()) {
            continue;
        }
        if item == Item::Separator && out.last().is_none_or(|l| *l == Item::Separator) {
            continue;
        }
        if item != Item::Separator && out.contains(&item) {
            continue;
        }
        out.push(item);
    }
    while out.last() == Some(&Item::Separator) {
        out.pop();
    }
    out
}

fn step(items: &[Item], from: usize, down: bool) -> usize {
    let mut i = from;
    loop {
        let next = if down { i + 1 } else { i.wrapping_sub(1) };
        if next >= items.len() {
            return from;
        }
        i = next;
        if items[i] != Item::Separator {
            return i;
        }
    }
}

impl CollectionView {
    /// The menu item of the page shown (where the menu opens).
    pub(super) fn sidebar_current(&self) -> usize {
        let items = items();
        items.iter().position(|i| i.page() == Some(self.page)).or_else(|| items.iter().position(|i| *i != Item::Separator)).unwrap_or(0)
    }

    pub(super) fn sidebar_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cur) = self.console.sidebar else { return };
        let items = items();
        let cur = cur.min(items.len().saturating_sub(1));
        match pad {
            Pad::Up | Pad::Down => self.console.sidebar = Some(step(&items, cur, pad == Pad::Down)),
            Pad::B | Pad::Start | Pad::Right => self.console.sidebar = None,
            Pad::A => {
                self.console.sidebar = None;
                match items[cur] {
                    Item::Go(Page::Collection) => {
                        self.navigate(Page::Collection, window, cx);
                        if matches!(self.console.screen, Screen::Game(_)) {
                            self.console.screen = Screen::Tabs;
                        }
                    }
                    Item::Go(page) => self.navigate(page, window, cx),
                    Item::Stores => {
                        let page = self.pages.stores_page();
                        self.navigate(page, window, cx);
                    }
                    Item::AddGame => self.console_add_game(window, cx),
                    Item::Customize => self.open_console_customize(),
                    Item::Desktop => self.set_console(false, window, cx),
                    Item::Power => self.console_power_menu(cx),
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
        // Sized so every item fits the 800-unit console height (the list does not scroll).
        let mut list = div().flex().flex_col().gap(u(4.));
        for (i, item) in items().iter().enumerate() {
            if *item == Item::Separator {
                list = list.child(div().my(u(10.)).mx(u(18.)).h(px(1.)).bg(theme::white_a(0.1)));
                continue;
            }
            let on = i == focused;
            let current = item.page() == Some(self.page) || (*item == Item::Stores && crate::pages::stores::is_store_page(self.page));
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
                    .h(u(54.))
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
                        .pt(u(36.))
                        .bg(theme::console_sidebar())
                        .border_r_1()
                        .border_color(theme::white_a(0.08))
                        .shadow(kit::shadow(0.5, 0., 40.))
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
    use super::{items, step, Item};

    #[test]
    fn steps_skip_separators_and_stop_at_the_ends() {
        let items = items();
        let add = items.iter().position(|i| *i == Item::AddGame).unwrap();
        assert_eq!(step(&items, add - 2, true), add);
        assert_eq!(step(&items, add, false), add - 2);
        assert_eq!(step(&items, 0, false), 0);
        assert_eq!(step(&items, items.len() - 1, true), items.len() - 1);
    }
}
