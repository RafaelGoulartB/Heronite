//! The console menu (Start / Menu, or B on a top screen): every page, Add
//! game, Customize, Settings, the way back to desktop mode and Power, in the
//! order and with the items the theme's `menu` says (Settings and Desktop
//! mode always stay), as a full-height sidebar over the dimmed screen.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::super::CollectionView;
use super::Screen;
use crate::console::kit::{self, u};
use crate::console::Pad;
use crate::console::{look, motion};
use crate::icons::Icon;
use crate::pages::Page;
use crate::theme;
use crate::ui::icon;
use heronite_data::console_theme::{MenuStyle, Side};
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

/// Tiles per row of the Grid menu.
const GRID_COLS: usize = 4;

/// What a press does to the menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Move {
    /// The focus goes to this item (an index in `items`).
    To(usize),
    Close,
    Keep,
}

/// The press `pad` with the focus on `cur`. The Sidebar moves with ▲ ▼ and
/// closes with the arrow that pushes it back towards its side; the Grid moves
/// in 2D and the Dock with ◀ ▶ (separators only exist in the Sidebar).
fn navigate(style: MenuStyle, side: Side, items: &[Item], cur: usize, pad: Pad) -> Move {
    if matches!(pad, Pad::B | Pad::Start) {
        return Move::Close;
    }
    if style == MenuStyle::Sidebar {
        return match pad {
            Pad::Up | Pad::Down => Move::To(step(items, cur, pad == Pad::Down)),
            Pad::Right if side == Side::Left => Move::Close,
            Pad::Left if side == Side::Right => Move::Close,
            _ => Move::Keep,
        };
    }
    let shown: Vec<usize> = (0..items.len()).filter(|&i| items[i] != Item::Separator).collect();
    let (Some(last), Some(at)) = (shown.len().checked_sub(1), shown.iter().position(|&i| i >= cur).or(shown.len().checked_sub(1))) else {
        return Move::Keep;
    };
    let to = match (pad, style) {
        (Pad::Left, _) => at.saturating_sub(1),
        (Pad::Right, _) => (at + 1).min(last),
        (Pad::Up, MenuStyle::Grid) => at.saturating_sub(GRID_COLS),
        // The last row may be short: Down reaches its last tile.
        (Pad::Down, MenuStyle::Grid) if at + GRID_COLS <= last => at + GRID_COLS,
        (Pad::Down, MenuStyle::Grid) if at / GRID_COLS < last / GRID_COLS => last,
        _ => return Move::Keep,
    };
    Move::To(shown[to])
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
        let theme = look::theme();
        match navigate(theme.menu.style, theme.menu.side, &items, cur, pad) {
            Move::To(i) => self.console.sidebar = Some(i),
            Move::Close => self.console.sidebar = None,
            Move::Keep => {}
        }
        if pad == Pad::A {
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
        cx.notify();
    }

    pub(super) fn render_sidebar(&self, cx: &App) -> Option<AnyElement> {
        let focused = self.console.sidebar?;
        let downloads = crate::services::try_downloads(cx).and_then(|s| super::super::nav::downloads_meta(Some(s.read(cx))));
        let menu = look::theme().menu.clone();
        let items = items();
        let focused = focused.min(items.len().saturating_sub(1));
        Some(match menu.style {
            MenuStyle::Sidebar => self.render_menu_sidebar(&items, focused, menu.side, downloads),
            MenuStyle::Grid => self.render_menu_grid(&items, focused, downloads),
            MenuStyle::Dock => self.render_menu_dock(&items, focused, downloads),
        })
    }

    /// Whether `item` is the page shown (drawn bold).
    fn menu_current(&self, item: Item) -> bool {
        item.page() == Some(self.page) || (item == Item::Stores && crate::pages::stores::is_store_page(self.page))
    }

    /// A full-height panel on the theme's side, over the dimmed screen.
    fn render_menu_sidebar(&self, items: &[Item], focused: usize, side: Side, downloads: Option<String>) -> AnyElement {
        // Sized so every item fits the 800-unit console height (the list does not scroll).
        let mut list = div().flex().flex_col().gap(u(4.));
        for (i, item) in items.iter().enumerate() {
            if *item == Item::Separator {
                list = list.child(div().my(u(10.)).mx(u(18.)).h(px(1.)).bg(theme::white_a(0.1)));
                continue;
            }
            let on = i == focused;
            let current = self.menu_current(*item);
            let fg: Hsla = if on {
                theme::console_on_focus().into()
            } else if current {
                white()
            } else {
                theme::text_primary().into()
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
                    .children(menu_badge(*item, &downloads)),
            );
        }
        let right = side == Side::Right;
        let panel = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(u(360.))
            .flex()
            .flex_col()
            .px(u(16.))
            .pt(u(36.))
            .bg(theme::console_sidebar())
            .border_color(theme::white_a(0.08))
            .shadow(kit::shadow(0.5, 0., 40.))
            .child(list);
        let panel = if right { panel.right_0().border_l_1() } else { panel.left_0().border_r_1() };
        div()
            .absolute()
            .inset_0()
            .child(kit::scrim())
            .child(motion::enter("console-sidebar", panel, 280, if right { 40. } else { -40. }, 0.))
            .into_any_element()
    }

    /// Big icon tiles, four to a row, centred over a strong dim.
    fn render_menu_grid(&self, items: &[Item], focused: usize, downloads: Option<String>) -> AnyElement {
        let tiles: Vec<(usize, Item)> = items.iter().copied().enumerate().filter(|(_, i)| *i != Item::Separator).collect();
        let mut rows = div().flex().flex_col().gap(u(16.));
        for chunk in tiles.chunks(GRID_COLS) {
            rows = rows.child(div().flex().gap(u(16.)).children(chunk.iter().map(|(i, item)| {
                let on = *i == focused;
                let fg: Hsla = if on { theme::console_on_focus().into() } else { theme::text_primary().into() };
                let tile = div()
                    .relative()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(u(12.))
                    .w(u(180.))
                    .h(u(130.))
                    .rounded(u(18.))
                    .border_1()
                    .border_color(theme::white_a(0.08))
                    .bg(theme::white_a(0.07))
                    .text_color(fg);
                let tile = if on { tile.child(kit::focus_pill(18.)) } else { tile };
                tile.child(icon(item.icon(), u(38.), if on { fg } else { theme::text_muted().into() }))
                    .child(
                        div().text_size(u(16.)).when(self.menu_current(*item) && !on, |d| d.font_weight(theme::W_BOLD)).child(item.label()),
                    )
                    .children(menu_badge(*item, &downloads).map(|b| div().absolute().top(u(10.)).right(u(10.)).child(b)))
            })));
        }
        div()
            .absolute()
            .inset_0()
            .child(motion::fade_in("console-menu-dim", div().absolute().inset_0().bg(theme::black_a(0.9)), 180))
            .child(div().absolute().inset_0().flex().items_center().justify_center().child(motion::enter(
                "console-menu-grid",
                div().relative().child(rows),
                260,
                0.,
                18.,
            )))
            .into_any_element()
    }

    /// A row of round icons at the bottom with the focused item's name above it.
    fn render_menu_dock(&self, items: &[Item], focused: usize, downloads: Option<String>) -> AnyElement {
        let dock = div()
            .flex()
            .items_center()
            .gap(u(14.))
            .px(u(18.))
            .py(u(10.))
            .rounded(u(26.))
            .bg(theme::console_sidebar())
            .border_1()
            .border_color(theme::white_a(0.1))
            .shadow(kit::shadow(0.6, 20., 50.))
            .children(items.iter().enumerate().filter(|(_, i)| **i != Item::Separator).map(|(i, item)| {
                let on = i == focused;
                let size = if on { 76. } else { 64. };
                let fg: Hsla = if on { theme::console_on_focus().into() } else { theme::text_primary().into() };
                let round = div()
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(u(size))
                    .h(u(size))
                    .rounded_full()
                    .bg(theme::white_a(0.08));
                let round = if on { round.child(kit::focus_pill(size / 2.)) } else { round };
                // The focused one sits a little higher than its neighbours.
                div().flex().items_center().h(u(92.)).pb(u(if on { 14. } else { 0. })).child(
                    round
                        .child(icon(item.icon(), u(30.), fg))
                        .children(menu_badge(*item, &downloads).map(|b| div().absolute().top(u(-2.)).right(u(-2.)).child(b))),
                )
            }));
        let label = items.get(focused).map(|i| i.label()).unwrap_or_default();
        div()
            .absolute()
            .inset_0()
            .child(motion::fade_in(
                "console-menu-dim",
                div().absolute().inset_0().bg(linear_gradient(
                    0.,
                    linear_color_stop(Rgba { a: 0.88, ..theme::console_bg() }, 0.),
                    linear_color_stop(Rgba { a: 0.25, ..theme::console_bg() }, 0.7),
                )),
                180,
            ))
            .child(motion::enter(
                "console-menu-dock",
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(u(84.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(u(10.))
                    .child(kit::title(label, 24.))
                    .child(dock),
                260,
                0.,
                30.,
            ))
            .into_any_element()
    }
}

/// The count on the Downloads item.
fn menu_badge(item: Item, downloads: &Option<String>) -> Option<Div> {
    match item {
        Item::Go(Page::Downloads) => downloads.clone().map(|b| kit::badge(b, theme::console_btn_a())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{items, navigate, step, Item, Move};
    use crate::console::Pad;
    use heronite_data::console_theme::{MenuStyle, Side};

    #[test]
    fn steps_skip_separators_and_stop_at_the_ends() {
        let items = items();
        let add = items.iter().position(|i| *i == Item::AddGame).unwrap();
        assert_eq!(step(&items, add - 2, true), add);
        assert_eq!(step(&items, add, false), add - 2);
        assert_eq!(step(&items, 0, false), 0);
        assert_eq!(step(&items, items.len() - 1, true), items.len() - 1);
    }

    #[test]
    fn the_sidebar_closes_towards_its_side() {
        let items = items();
        let nav = |side, pad| navigate(MenuStyle::Sidebar, side, &items, 0, pad);
        assert_eq!(nav(Side::Left, Pad::Right), Move::Close);
        assert_eq!(nav(Side::Left, Pad::Left), Move::Keep);
        assert_eq!(nav(Side::Right, Pad::Left), Move::Close);
        assert_eq!(nav(Side::Right, Pad::Right), Move::Keep);
        assert_eq!(nav(Side::Left, Pad::B), Move::Close);
    }

    #[test]
    fn grid_and_dock_skip_separators_and_grid_moves_in_2d() {
        // Six items with separators between: positions 0, 2, 3, 5, 6, 8.
        let s = Item::Separator;
        let items = [Item::AddGame, s, Item::Customize, Item::Desktop, s, Item::Power, Item::Stores, s, Item::Customize];
        let grid = |cur, pad| navigate(MenuStyle::Grid, Side::Left, &items, cur, pad);
        let dock = |cur, pad| navigate(MenuStyle::Dock, Side::Left, &items, cur, pad);
        assert_eq!(dock(0, Pad::Right), Move::To(2));
        assert_eq!(dock(3, Pad::Left), Move::To(2));
        assert_eq!(dock(0, Pad::Left), Move::To(0));
        assert_eq!(dock(8, Pad::Right), Move::To(8));
        assert_eq!(dock(0, Pad::Down), Move::Keep);
        // Shown: [0, 2, 3, 5 | 6, 8]: a short second row.
        assert_eq!(grid(2, Pad::Down), Move::To(8));
        assert_eq!(grid(0, Pad::Down), Move::To(6));
        assert_eq!(grid(6, Pad::Up), Move::To(0));
        assert_eq!(grid(6, Pad::Down), Move::Keep);
        assert_eq!(grid(0, Pad::Up), Move::To(0));
    }
}
