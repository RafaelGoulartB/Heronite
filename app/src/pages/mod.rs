//! Pages reachable from the ☰ menu in the header. The Collection is the
//! main surface; every other page is its own view (`Entity<P>` with
//! `P: Render + EventEmitter<PageEvent>`) drawn by `CollectionView` below a
//! slim header (nav button + title) over the same stage background.
//!
//! Conventions: same as dialogs (`docs/UI-CONVENTIONS.md`) — no I/O on the
//! UI thread, shared files only through the backend crates. Lay the content
//! out with [`sheet`] so pages share the chrome, width and padding.

pub mod downloads;
pub mod emulators;
pub mod epic;
pub mod settings;
pub mod stats;
pub mod wine_manager;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{AppPaths, DataPaths};

use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{close_button, help_text, page_heading, CAPTION_H};
use crate::ui::{glass, icon};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Page {
    Collection,
    Downloads,
    WineManager,
    Epic,
    Emulators,
    Statistics,
    /// Global settings (☰ → Settings, listed last).
    Settings,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Collection => "Collection",
            Page::Downloads => "Downloads",
            Page::WineManager => "Wine Manager",
            Page::Epic => "Epic Games",
            Page::Emulators => "Emulators",
            Page::Statistics => "Statistics",
            Page::Settings => "Settings",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Page::Collection => Icon::GridView,
            Page::Downloads => Icon::Download,
            Page::WineManager => Icon::WineBar,
            Page::Epic => Icon::AccountCircle,
            Page::Emulators => Icon::SportsEsports,
            Page::Statistics => Icon::BarChart,
            Page::Settings => Icon::Settings,
        }
    }

    /// Parses a `HERONITE_DEBUG_PAGE` value.
    pub fn from_debug(s: &str) -> Option<Page> {
        match s {
            "collection" => Some(Page::Collection),
            "downloads" => Some(Page::Downloads),
            "wine" => Some(Page::WineManager),
            "epic" => Some(Page::Epic),
            "emulators" => Some(Page::Emulators),
            // `stats[:week|month|year|all]`, see `stats::StatsPage`.
            s if s == "stats" || s.starts_with("stats:") => Some(Page::Statistics),
            // `settings[:section[:tab]]`, see `settings::SettingsPage`.
            s if s == "settings" || s.starts_with("settings:") => Some(Page::Settings),
            _ => None,
        }
    }
}

/// Paths every page needs.
#[derive(Clone)]
pub struct PageCtx {
    pub data_paths: DataPaths,
    pub app: AppPaths,
}

#[derive(Clone, Debug)]
pub enum PageEvent {
    /// Data the Collection shows changed on disk (it reloads the library).
    LibraryChanged,
    /// Show a toast (`true` = error).
    Toast(String, bool),
    Navigate(Page),
    /// Open the "Add emulated games" dialog (preview and import) on the Collection.
    ScanRoms(crate::dialogs::scan_roms::ScanInitial),
    /// "Show hidden games" changed in Settings: the Collection saves the flag
    /// and reloads its library.
    ShowHidden(bool),
    /// Select this game (`CollectionGame::key`) on the Collection.
    ShowGame(String),
}

/// Round close button of a [`sheet`]: goes back to the Collection.
pub fn page_close<V: EventEmitter<PageEvent> + 'static>(cx: &mut Context<V>) -> Stateful<Div> {
    close_button("page-close").on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::Navigate(Page::Collection))))
}

/// Horizontal room for a [`sheet`]'s content: the window minus the sheet's
/// inset (12px each side) and the body padding (32px each side).
pub fn content_width(window: &Window) -> f32 {
    f32::from(window.viewport_size().width) - 24. - 64.
}

/// Glass sheet floating over the (dimmed) Collection: title (Rubik 26) + lead
/// on the left, `actions` on the right, then a scrolling body. Title and body
/// share one column of at most `max_w`, centred in the sheet, so a wide
/// window gets even margins instead of an empty right side. The round close
/// button sits in the sheet's top-right corner, as in Settings. Pages render
/// this as their whole view (Downloads, Wine Manager, Epic Games, Emulators).
pub fn sheet(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    lead: Option<SharedString>,
    actions: Option<AnyElement>,
    max_w: f32,
    window: &Window,
    close: Stateful<Div>,
    column: impl IntoElement,
) -> Div {
    let id: ElementId = id.into();
    // Keep the actions clear of the close button once the column reaches it.
    let room = content_width(window);
    let clear_close = (room - max_w) / 2. < 40.;
    let header = div().flex_none().px(px(32.)).pt(px(24.)).pb(px(8.)).child(
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(16.))
            .w_full()
            .max_w(px(max_w))
            .mx_auto()
            .when(clear_close, |d| d.pr(px(44.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .min_w_0()
                    .child(page_heading(title))
                    .children(lead.map(help_text)),
            )
            .child(div().flex().flex_none().items_center().gap(px(10.)).children(actions)),
    );
    let body = div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(32.))
        .pt(px(10.))
        .pb(px(28.))
        .child(div().flex().flex_col().w_full().max_w(px(max_w)).mx_auto().child(column));
    div().size_full().p(px(12.)).pt(px(2.)).child(
        glass::glass_panel(theme::R_SHEET)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .text_color(theme::text_primary())
            .font_family(theme::FONT_BODY)
            .child(header)
            .child(body)
            .child(close.absolute().top(px(20.)).right(px(20.))),
    )
}

/// Columns of equal width side by side, top-aligned, when `wide`; otherwise
/// the same blocks stacked in order. Each block is a flex column.
pub fn columns(wide: bool, blocks: impl IntoIterator<Item = Div>) -> Div {
    let mut row = div().flex().w_full();
    row = if wide { row.items_start().gap(px(24.)) } else { row.flex_col() };
    for b in blocks {
        row = row.child(b.flex().flex_col().min_w_0().when(wide, |b| b.flex_basis(px(0.)).flex_grow(1.)));
    }
    row
}

/// One row of a list card: optional leading element (cover, icon), the
/// content (grows) and trailing controls. Padding 12px 0, like [`crate::ui::controls::row`].
pub fn list_item(leading: Option<AnyElement>, content: impl IntoElement, trailing: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .py(px(12.))
        .children(leading)
        .child(div().flex().flex_col().flex_1().min_w_0().gap(px(3.)).child(content))
        .child(div().flex().flex_none().items_center().gap(px(6.)).child(trailing))
}

/// Round 32px icon button for list rows (pause, remove, move up...).
/// Disabled ones are dimmed and the caller must not attach a click handler.
pub fn icon_action(id: impl Into<ElementId>, ic: Icon, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded_full()
        .bg(theme::white_a(0.08))
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::white_a(0.14))))
        .when(!enabled, |d| d.opacity(0.35))
        .child(icon(ic, px(16.), theme::text_primary()))
}

/// Small tinted status pill ("Ready", "Installed", "Native"): `color` text on
/// a 16% tint of the same colour.
pub fn status_pill(text: impl Into<SharedString>, color: Rgba) -> Div {
    let c: Hsla = color.into();
    div()
        .flex_none()
        .px(px(9.))
        .py(px(2.))
        .rounded_full()
        .bg(Hsla { a: 0.16, ..c })
        .text_size(px(12.))
        .text_color(color)
        .whitespace_nowrap()
        .child(text.into())
}

/// Neutral pill (platform names, launch kind).
pub fn tag_pill(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px(px(9.))
        .py(px(2.))
        .rounded_full()
        .bg(theme::white_a(0.08))
        .text_size(px(12.))
        .text_color(theme::text_muted())
        .whitespace_nowrap()
        .child(text.into())
}

/// Caption with an action on the right ("Completed" + "Clear list"). As
/// tall as a plain caption (the action overflows it), so the cards below
/// line up with those of neighbouring columns.
pub fn caption_with(text: impl Into<SharedString>, action: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .h(px(CAPTION_H))
        .mt(px(14.))
        .mb(px(6.))
        .child(div().mx(px(6.)).text_size(px(12.)).line_height(px(CAPTION_H)).text_color(theme::text_muted()).child(text.into()))
        .child(action)
}

/// Empty / info row inside a card.
pub fn empty_row(text: impl Into<SharedString>) -> Div {
    div().py(px(14.)).text_size(px(13.)).text_color(theme::text_dim()).child(text.into())
}
