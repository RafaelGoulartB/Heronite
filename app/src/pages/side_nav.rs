//! The layout of the pages that list where games come from (the stores,
//! the emulators): a glass sheet with a list on its left, as in Settings,
//! and the page of what is picked in it on the right. Each item of the list
//! has an icon, a name, a line on where it stands and a dot; both sides
//! scroll with a scrollbar.

use std::collections::HashMap;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{Page, PageEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{close_button, help_text, page_heading};
use crate::ui::{glass, icon};

/// Width of the list.
pub const NAV_W: f32 = 250.;

/// Horizontal room for the page beside the list: [`super::content_width`]
/// minus the list.
pub fn content_width(window: &Window) -> f32 {
    super::content_width(window) - NAV_W - 8.
}

/// The dot beside an item of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    /// Signed in, found, set up, ready.
    On,
    /// Something to do: log in again, sign in, add a folder.
    Attention,
    /// Working on it: setting up, opening, reading.
    Busy,
    /// Not used yet.
    Off,
}

fn dot(d: Dot) -> Div {
    let base = div().flex_none().size(px(8.)).rounded_full();
    match d {
        Dot::On => base.bg(theme::success()),
        Dot::Attention => base.bg(theme::warning()),
        Dot::Busy => base.bg(theme::accent()),
        Dot::Off => base.border_1().border_color(theme::text_dim()),
    }
}

/// The colour of the line under an item: the dot's when it asks for
/// something, else muted.
fn line_color(d: Dot) -> Rgba {
    match d {
        Dot::Attention => theme::warning(),
        _ => theme::text_muted(),
    }
}

/// The 30px square an item's icon sits in.
fn icon_box(ic: Icon, tint: impl Into<Hsla>) -> Div {
    div().flex().flex_none().items_center().justify_center().size(px(30.)).rounded(px(9.)).bg(theme::ink_a(0.08)).child(icon(
        ic,
        px(16.),
        tint,
    ))
}

fn item_base(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).flex().items_center().gap(px(10.)).px(px(10.)).py(px(7.)).mb(px(2.)).rounded(px(12.)).cursor_pointer()
}

/// An item of the list: its icon, `title`, `line` (when not empty) and
/// its dot. The caller adds `.on_click(..)`.
pub fn item(
    id: impl Into<ElementId>,
    ic: Icon,
    title: impl Into<SharedString>,
    line: impl Into<SharedString>,
    d: Dot,
    active: bool,
) -> Stateful<Div> {
    let line: SharedString = line.into();
    item_base(id)
        .when(active, |el| el.bg(theme::accent_tint()))
        .when(!active, |el| el.hover(|s| s.bg(theme::ink_a(0.05))))
        .child(icon_box(ic, if active { theme::accent() } else { theme::nav_text() }))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(14.))
                        .truncate()
                        .text_color(if active { theme::text_bright() } else { theme::nav_text().into() })
                        .child(title.into()),
                )
                .when(!line.is_empty(), |el| el.child(div().text_size(px(11.5)).truncate().text_color(line_color(d)).child(line))),
        )
        .child(dot(d))
}

/// The list's title: an icon, the name, and `trailing` on the right.
pub fn heading(ic: Icon, title: impl Into<SharedString>, trailing: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .pb(px(10.))
        .child(icon(ic, px(18.), theme::text_primary()))
        .child(
            div()
                .flex_1()
                .font_family(theme::font_heading())
                .text_size(px(19.))
                .font_weight(theme::W_BOLD)
                .text_color(theme::text_primary())
                .child(title.into()),
        )
        .children(trailing)
}

/// A group's caption in the list, with how many it holds on the right.
pub fn group(label: impl Into<SharedString>, count: Option<usize>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(12.))
        .pt(px(12.))
        .pb(px(5.))
        .text_size(px(12.))
        .text_color(theme::text_muted())
        .child(label.into())
        .children(count.map(|n| div().text_size(px(11.)).text_color(theme::text_dim()).child(n.to_string())))
}

/// A dashed link under the list to another page: an icon, its name, a line
/// on what is there, and the "opens elsewhere" arrow. The caller adds
/// `.on_click(..)`.
pub fn link(id: impl Into<ElementId>, ic: Icon, title: impl Into<SharedString>, line: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(9.))
        .rounded(px(12.))
        .border_1()
        .border_dashed()
        .border_color(theme::ink_a(0.16))
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.05)))
        .child(icon(ic, px(16.), theme::text_muted()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(13.5)).text_color(theme::nav_text()).child(title.into()))
                .child(div().text_size(px(11.5)).truncate().text_color(theme::text_dim()).child(line.into())),
        )
        .child(icon(Icon::OpenInNew, px(14.), theme::text_dim()))
}

/// Where each list and page is scrolled, by key. The pages are separate
/// views drawn one at a time: a list shared by several pages keeps its
/// place from one to the next.
#[derive(Default)]
struct Scrolls(HashMap<SharedString, ScrollHandle>);

impl Global for Scrolls {}

fn scroll_of(key: SharedString, cx: &mut App) -> ScrollHandle {
    cx.default_global::<Scrolls>().0.entry(key).or_default().clone()
}

/// `area` (which holds the element scrolled by `scroll`) with its
/// scrollbar `right` px from its edge, when the content is taller than it.
fn with_scrollbar<V: 'static>(area: Div, id: &'static str, scroll: &ScrollHandle, right: f32, cx: &mut Context<V>) -> Div {
    let (offset, max, height) = (scroll.offset().y, scroll.max_offset().y, scroll.bounds().size.height);
    // The sizes are known only once laid out, after this draw: when they
    // differ from the ones drawn (the first time, the window resized, the
    // page grew), draw once more so the bar follows.
    let (laid_out, view) = (scroll.clone(), cx.entity_id());
    let watch = canvas(
        move |_, _, cx| {
            if laid_out.max_offset().y != max || laid_out.bounds().size.height != height {
                cx.notify(view);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0();
    let (to, weak) = (scroll.clone(), cx.entity().downgrade());
    let bar = crate::ui::scrollbar_draggable(
        id,
        offset,
        max,
        height,
        move |distance, _, cx| {
            to.set_offset(point(px(0.), px(-distance)));
            weak.update(cx, |_, cx| cx.notify()).ok();
        },
        None,
    );
    area.child(watch).children(bar.map(|b| b.right(px(right))))
}

/// The list on the left: `heading`, then `items` scrolling (kept under
/// `key`), then `footer` pinned at the bottom.
pub fn nav<V: 'static>(
    key: &'static str,
    heading: Div,
    items: impl IntoIterator<Item = AnyElement>,
    footer: Option<AnyElement>,
    cx: &mut Context<V>,
) -> Div {
    let scroll = scroll_of(key.into(), cx);
    let list = div()
        .id(key)
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
        .flex()
        .flex_col()
        .child(heading)
        .children(items);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(NAV_W))
        .h_full()
        .px(px(12.))
        .pt(px(26.))
        .pb(px(14.))
        .border_r_1()
        .border_color(theme::row_separator())
        .child(with_scrollbar(div().relative().flex_1().min_h_0().child(list), "side-nav-scrollbar", &scroll, -11., cx))
        .children(footer.map(|f| div().pt(px(12.)).child(f)))
}

/// The top of the page on the right: `title` with `tags` beside it,
/// `lead` under it, and `actions` on the right (kept clear of the close
/// button).
pub fn header(
    title: impl Into<SharedString>,
    tags: impl IntoIterator<Item = AnyElement>,
    lead: impl Into<SharedString>,
    actions: Option<AnyElement>,
) -> Div {
    let lead: SharedString = lead.into();
    div()
        .flex()
        .items_start()
        .gap(px(16.))
        .pr(px(48.))
        .pb(px(18.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(4.))
                .child(div().flex().flex_wrap().items_center().gap(px(10.)).child(page_heading(title)).children(tags))
                .when(!lead.is_empty(), |d| d.child(help_text(lead))),
        )
        .children(actions.map(|a| div().flex().flex_none().items_center().gap(px(8.)).child(a)))
}

/// The page: `nav` on the left; `header` and `body` (at most 1320px wide)
/// on the right, scrolling (kept under `key`). The round close button goes
/// back to the Collection.
pub fn sheet<V: EventEmitter<PageEvent> + 'static>(
    key: impl Into<SharedString>,
    nav: Div,
    header: Div,
    cx: &mut Context<V>,
    body: impl IntoElement,
) -> Div {
    let key: SharedString = key.into();
    let scroll = scroll_of(key.clone(), cx);
    let content = div()
        .id(ElementId::Name(key))
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
        .px(px(32.))
        .pt(px(26.))
        .pb(px(28.))
        .child(div().flex().flex_col().w_full().max_w(px(1320.)).mx_auto().child(header).child(body));
    let close = close_button("page-close")
        .absolute()
        .top(px(20.))
        .right(px(20.))
        .on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::Navigate(Page::Collection))));
    div().size_full().p(px(12.)).pt(px(2.)).child(
        glass::glass_panel(theme::R_SHEET)
            .relative()
            .size_full()
            .flex()
            .text_color(theme::text_primary())
            .font_family(theme::font_body())
            .child(nav)
            .child(with_scrollbar(div().relative().flex_1().min_w_0().h_full().child(content), "side-page-scrollbar", &scroll, 4., cx))
            .child(close),
    )
}
