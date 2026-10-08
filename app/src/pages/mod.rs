//! Pages reachable from the ☰ menu in the header. The Collection is the
//! main surface; every other page is its own view (`Entity<P>` with
//! `P: Render + EventEmitter<PageEvent>`) drawn by `CollectionView` below a
//! slim header (nav button + title) over the same stage background.
//!
//! Conventions: same as dialogs (`docs/UI-CONVENTIONS.md`) — no I/O on the
//! UI thread, shared files only through the backend crates. Lay the content
//! out with [`page_body`] so pages share width and padding.

pub mod downloads;
pub mod epic;
pub mod wine_manager;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{AppPaths, HeroicPaths};

use crate::icons::Icon;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Collection,
    Downloads,
    WineManager,
    Epic,
}

impl Page {
    pub const ALL: [Page; 4] = [Page::Collection, Page::Downloads, Page::WineManager, Page::Epic];

    pub fn title(self) -> &'static str {
        match self {
            Page::Collection => "Collection",
            Page::Downloads => "Downloads",
            Page::WineManager => "Wine Manager",
            Page::Epic => "Epic Games",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Page::Collection => Icon::GridView,
            Page::Downloads => Icon::Download,
            Page::WineManager => Icon::WineBar,
            Page::Epic => Icon::AccountCircle,
        }
    }

    /// `HERONITE_DEBUG_PAGE` values.
    pub fn from_debug(s: &str) -> Option<Page> {
        match s {
            "collection" => Some(Page::Collection),
            "downloads" => Some(Page::Downloads),
            "wine" => Some(Page::WineManager),
            "epic" => Some(Page::Epic),
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
}

/// Centred column (max 1480px like the focus panel) with the page padding;
/// scrolls vertically.
pub fn page_body(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .px(px(36.))
        .pt(px(12.))
        .pb(px(36.))
}

/// Inner column of [`page_body`]: `page_body(..).child(page_column().child(..))`.
pub fn page_column() -> Div {
    div().flex().flex_col().gap(px(16.)).w_full().max_w(px(1480.)).mx_auto()
}
