//! The ☰ menu of a picture, in the theme's style (sidebar, grid, dock or
//! rail) on its side with the items it lists, and the rail's column that
//! stays on screen while the menu is closed.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{ConsoleTheme, MenuStyle, Side, MENU_LOCKED};

use super::Pic;
use crate::console::theme_rows::menu_item_name;
use crate::icons::Icon;
use crate::pages::Page;
use crate::theme;
use crate::ui::icon;

/// Width of the rail while the menu is closed, canvas units (the console's
/// 74 design px, half).
pub(super) const RAIL_W: f32 = 37.;
/// Width of the sidebar.
const PANEL_W: f32 = 180.;
/// Tiles per row of the grid style.
const GRID_COLS: usize = 4;
const SEPARATOR: &str = "separator";

/// The items the menu lists: the ones switched on in the theme's order, the
/// locked ones always, no separator at an end or next to another.
fn items(t: &ConsoleTheme) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for spec in &t.menu.items {
        let item = spec.item.as_str();
        if menu_item_name(item).is_empty() || (!spec.on && !MENU_LOCKED.contains(&item)) {
            continue;
        }
        if item == SEPARATOR && out.last().is_none_or(|l| *l == SEPARATOR) {
            continue;
        }
        if item != SEPARATOR && out.contains(&item) {
            continue;
        }
        out.push(item);
    }
    while out.last() == Some(&SEPARATOR) {
        out.pop();
    }
    out
}

fn icon_of(item: &str) -> Icon {
    match item {
        "collection" => Page::Collection.icon(),
        "downloads" => Page::Downloads.icon(),
        "wine-manager" => Page::WineManager.icon(),
        "stores" => Icon::Storefront,
        "emulators" => Page::Emulators.icon(),
        "statistics" => Page::Statistics.icon(),
        "add-game" => Icon::Add,
        "customize" => Icon::Tune,
        "settings" => Page::Settings.icon(),
        "desktop" => Icon::Logout,
        "power" => Icon::PowerSettingsNew,
        _ => Icon::Menu,
    }
}

impl Pic<'_, '_> {
    /// The row of `item`: an icon and its name; the focused one on the focus
    /// colour with its text colour.
    fn menu_row(&self, item: &str, focused: bool, h: f32) -> Div {
        let (fg, ic) = if focused { (self.focus_text, self.focus_text) } else { (self.text, self.muted) };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(self.u(7.))
            .h(self.u(h))
            .px(self.u(10.))
            .rounded(self.u(7.))
            .when(focused, |d| d.bg(self.accent).border_1().border_color(theme::white_a(0.5)))
            .child(icon(icon_of(item), self.u(11.), ic))
            .child(self.label(menu_item_name(item), 9.5, fg).flex_1().min_w_0())
    }

    /// The sidebar's rows (the rail's when `compact`: a ☰ on top, gaps for
    /// separators), the first item focused.
    fn menu_list(&self, items: &[&str], compact: bool) -> Div {
        let h = if compact { 19. } else { 22. };
        let mut list = div().flex().flex_col().gap(self.u(1.));
        if compact {
            list = list.child(div().h(self.u(h)).px(self.u(10.)).flex().items_center().child(icon(Icon::Menu, self.u(12.), self.muted)));
        }
        for (i, item) in items.iter().enumerate() {
            list = list.child(if *item == SEPARATOR {
                if compact {
                    div().h(self.u(7.))
                } else {
                    div().my(self.u(4.)).mx(self.u(8.)).h(px(1.)).bg(Rgba { a: 0.14, ..self.text })
                }
            } else {
                self.menu_row(item, i == 0, h)
            });
        }
        list
    }

    /// A solid card on the theme's panel colour.
    fn menu_surface(&self) -> Div {
        div().bg(self.sidebar).border_1().border_color(Rgba { a: 0.1, ..self.text })
    }
}

/// The rail's column while the menu is closed (only the Rail style has one):
/// muted icons over a soft shade, the page shown in the text colour.
pub(super) fn rail(p: &Pic) -> Option<Div> {
    let t = p.t;
    if t.menu.style != MenuStyle::Rail {
        return None;
    }
    let right = t.menu.side == Side::Right;
    let items = items(t);
    let mut col = div().flex().flex_col().items_center().gap(p.u(1.)).pt(p.u(14.));
    col = col.child(div().h(p.u(19.)).flex().items_center().child(icon(Icon::Menu, p.u(12.), p.muted)));
    for (i, item) in items.iter().enumerate() {
        col = col.child(if *item == SEPARATOR {
            div().h(p.u(7.))
        } else {
            div().h(p.u(19.)).flex().items_center().child(icon(icon_of(item), p.u(11.), if i == 0 { p.text } else { p.muted }))
        });
    }
    let edge = if right { 270. } else { 90. };
    let shade = linear_gradient(edge, linear_color_stop(p.veil(0.4), 0.), linear_color_stop(p.veil(0.), 1.));
    let column = div().absolute().top_0().bottom_0().w(p.u(RAIL_W)).bg(shade).child(col.opacity(0.7));
    let r = px(p.frame_r);
    Some(if right { column.right_0().rounded_tr(r).rounded_br(r) } else { column.left_0().rounded_tl(r).rounded_bl(r) })
}

/// The open menu over the whole picture: the screen dimmed and the menu in
/// the theme's style, listing its items with the first one focused.
pub(super) fn open(p: &mut Pic) -> Div {
    let t = p.t;
    let items = items(t);
    let right = t.menu.side == Side::Right;
    let layer = p.full();
    match t.menu.style {
        MenuStyle::Sidebar | MenuStyle::Rail => {
            let panel = p
                .menu_surface()
                .absolute()
                .top_0()
                .bottom_0()
                .w(p.u(PANEL_W))
                .overflow_hidden()
                .px(p.u(8.))
                .pt(p.u(if t.menu.style == MenuStyle::Rail { 14. } else { 18. }))
                .shadow(vec![BoxShadow {
                    color: theme::black_a(0.5),
                    offset: point(px(0.), px(0.)),
                    blur_radius: p.u(18.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .child(p.menu_list(&items, t.menu.style == MenuStyle::Rail));
            // It reaches the frame's edge: it rounds that corner itself.
            let r = px(p.frame_r);
            let panel = if right { panel.right_0().rounded_tr(r).rounded_br(r) } else { panel.left_0().rounded_tl(r).rounded_bl(r) };
            layer.child(p.full().bg(p.shade(0.62))).child(panel)
        }
        MenuStyle::Grid => {
            let tiles: Vec<&str> = items.iter().copied().filter(|i| *i != SEPARATOR).collect();
            let mut rows = div().flex().flex_col().gap(p.u(8.));
            for (r, chunk) in tiles.chunks(GRID_COLS).enumerate() {
                let mut row = div().flex().gap(p.u(8.));
                for (c, item) in chunk.iter().enumerate() {
                    let on = r == 0 && c == 0;
                    let (fg, ic) = if on { (p.focus_text, p.focus_text) } else { (p.text, p.muted) };
                    row = row.child(
                        p.menu_surface()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(p.u(6.))
                            .w(p.u(90.))
                            .h(p.u(62.))
                            .rounded(p.u(9.))
                            .when(on, |d| d.bg(p.accent).border_color(theme::white_a(0.5)))
                            .child(icon(icon_of(item), p.u(19.), ic))
                            .child(p.label(menu_item_name(item), 8., fg)),
                    );
                }
                rows = rows.child(row);
            }
            layer.child(p.full().bg(p.veil(0.9))).child(p.full().flex().items_center().justify_center().child(rows))
        }
        MenuStyle::Dock => {
            let icons: Vec<&str> = items.iter().copied().filter(|i| *i != SEPARATOR).collect();
            let mut dock = p.menu_surface().flex().items_center().gap(p.u(7.)).px(p.u(9.)).py(p.u(5.)).rounded(p.u(13.));
            for (i, item) in icons.iter().enumerate() {
                let on = i == 0;
                let size = if on { 38. } else { 32. };
                let round = div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(p.u(size))
                    .rounded_full()
                    .bg(if on { p.accent } else { Rgba { a: 0.08, ..p.text } })
                    .border_1()
                    .border_color(if on { theme::white_a(0.5) } else { theme::white_a(0.) })
                    .child(icon(icon_of(item), p.u(15.), if on { p.focus_text } else { p.text }));
                // The focused one sits a little higher than its neighbours.
                dock = dock.child(div().flex().items_center().h(p.u(46.)).pb(p.u(if on { 7. } else { 0. })).child(round));
            }
            let name = items.first().map(|i| menu_item_name(i)).unwrap_or_default();
            let wash =
                linear_gradient(0., linear_color_stop(Rgba { a: 0.88, ..p.bg }, 0.), linear_color_stop(Rgba { a: 0.25, ..p.bg }, 0.7));
            layer.child(p.full().bg(wash)).child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(p.u(42.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(p.u(5.))
                    .child(p.heading(name, 12.))
                    .child(dock),
            )
        }
    }
}
