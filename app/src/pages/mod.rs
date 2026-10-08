//! Pages reachable from the ☰ menu in the header. The Collection is the
//! main surface; every other page is its own view (`Entity<P>` with
//! `P: Render + EventEmitter<PageEvent>`) drawn by `CollectionView` below a
//! slim header (nav button + title) over the same stage background.
//!
//! Conventions: same as dialogs (`docs/UI-CONVENTIONS.md`) — no I/O on the
//! UI thread, shared files only through the backend crates. Lay the content
//! out with [`page_body`] so pages share width and padding.

pub mod downloads;
pub mod emulators;
pub mod epic;
pub mod settings;
pub mod wine_manager;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{AppPaths, HeroicPaths};

use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{close_button, help_text, page_heading};
use crate::ui::{glass, icon};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Collection,
    Downloads,
    WineManager,
    Epic,
    Emulators,
    /// Global settings (☰ → Settings); not in [`Page::ALL`], the menu lists it last.
    Settings,
}

impl Page {
    pub const ALL: [Page; 5] =
        [Page::Collection, Page::Downloads, Page::WineManager, Page::Epic, Page::Emulators];

    pub fn title(self) -> &'static str {
        match self {
            Page::Collection => "Collection",
            Page::Downloads => "Downloads",
            Page::WineManager => "Wine Manager",
            Page::Epic => "Epic Games",
            Page::Emulators => "Emulators",
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
            Page::Settings => Icon::Settings,
        }
    }

    /// `HERONITE_DEBUG_PAGE` values.
    pub fn from_debug(s: &str) -> Option<Page> {
        match s {
            "collection" => Some(Page::Collection),
            "downloads" => Some(Page::Downloads),
            "wine" => Some(Page::WineManager),
            "epic" => Some(Page::Epic),
            "emulators" => Some(Page::Emulators),
            // `settings[:section[:tab]]`, see `settings::SettingsPage`.
            s if s == "settings" || s.starts_with("settings:") => Some(Page::Settings),
            _ => None,
        }
    }
}

/// What every page may need.
#[derive(Clone)]
pub struct PageCtx {
    pub heroic: HeroicPaths,
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
}

// ---------------------------------------------------------------------------
// Glass sheet chrome (Downloads, Wine Manager, Epic Games, Emulators)
// ---------------------------------------------------------------------------

/// Round close button of a [`sheet`]: goes back to the Collection.
pub fn page_close<V: EventEmitter<PageEvent> + 'static>(cx: &mut Context<V>) -> Stateful<Div> {
    close_button("page-close").on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::Navigate(Page::Collection))))
}

/// Glass sheet floating over the (dimmed) Collection, like the Settings sheet
/// of `o1-1.jpg`: title (Rubik 26) + lead on the left, `actions` and the round
/// close button on the right, then a scrolling body. `column` is the content
/// column (a `Div` whose children are captions and cards); it is centred and
/// limited to `max_w`. Pages render this as their whole view.
pub fn sheet(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    lead: Option<SharedString>,
    actions: Option<AnyElement>,
    max_w: f32,
    close: Stateful<Div>,
    column: impl IntoElement,
) -> Div {
    let id: ElementId = id.into();
    let header = div()
        .flex()
        .flex_none()
        .items_start()
        .justify_between()
        .gap(px(16.))
        .pl(px(32.))
        .pr(px(24.))
        .pt(px(24.))
        .pb(px(8.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .min_w_0()
                .child(page_heading(title))
                .children(lead.map(help_text)),
        )
        .child(div().flex().flex_none().items_center().gap(px(10.)).children(actions).child(close));
    let body = div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(32.))
        .pt(px(10.))
        .pb(px(28.))
        .child(div().flex().flex_col().w_full().max_w(px(max_w)).child(column));
    div().size_full().p(px(12.)).pt(px(2.)).child(
        glass::glass_panel(theme::R_SHEET)
            .size_full()
            .flex()
            .flex_col()
            .text_color(theme::text_primary())
            .font_family(theme::FONT_BODY)
            .child(header)
            .child(body),
    )
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

/// Caption with an action on the right ("Completed" + "Clear list").
pub fn caption_with(text: impl Into<SharedString>, action: impl IntoElement) -> Div {
    // Same gap above as a plain caption, a small one before the card, text and button centred.
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .mt(px(14.))
        .mb(px(6.))
        .child(div().mx(px(6.)).text_size(px(12.)).text_color(theme::text_muted()).child(text.into()))
        .child(action)
}

/// Empty / info row inside a card.
pub fn empty_row(text: impl Into<SharedString>) -> Div {
    div().py(px(14.)).text_size(px(13.)).text_color(theme::text_dim()).child(text.into())
}
