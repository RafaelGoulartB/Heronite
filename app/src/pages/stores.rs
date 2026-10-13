//! Epic Games, GOG, Amazon Games and Steam share one entry ("Stores") in the
//! ☰ menu, the console sidebar and the welcome card: each store keeps its own
//! page, and they switch with these tabs (LB / RB in console mode).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{Page, PageEvent};
use crate::console::Pad;

/// The store pages, in tab order.
pub const PAGES: [Page; 4] = [Page::Epic, Page::Gog, Page::Amazon, Page::Steam];

pub fn is_store_page(page: Page) -> bool {
    PAGES.contains(&page)
}

/// Title of the store pages (the store is the active tab).
pub fn title() -> &'static str {
    tr!("Stores")
}

/// Segmented tabs over a store page's content; picking one shows that page.
pub fn tabs<V: EventEmitter<PageEvent> + 'static>(active: Page, cx: &mut Context<V>) -> Div {
    let items: Vec<(Page, &'static str)> = PAGES.iter().map(|p| (*p, p.title())).collect();
    let page = cx.entity().downgrade();
    div().flex().mb(px(18.)).child(crate::ui::controls::tabs("stores", &items, active, move |p, _, cx| {
        if p != active {
            page.update(cx, |_, cx| cx.emit(PageEvent::Navigate(p))).ok();
        }
    }))
}

/// The console top bar's tabs (switched with LB / RB, see [`switch`]).
pub fn console_tabs(active: Page) -> Div {
    let names: Vec<SharedString> = PAGES.iter().map(|p| SharedString::from(p.title())).collect();
    crate::console::kit::tabs(&names, PAGES.iter().position(|p| *p == active).unwrap_or(0))
}

/// The store LB / RB moves to from `active`, if `pad` is one of them.
pub fn switch(active: Page, pad: Pad) -> Option<Page> {
    let i = PAGES.iter().position(|p| *p == active)?;
    let n = PAGES.len();
    match pad {
        Pad::LB => Some(PAGES[(i + n - 1) % n]),
        Pad::RB => Some(PAGES[(i + 1) % n]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_store_page, switch, Pad, Page};

    #[test]
    fn lb_and_rb_cycle_through_the_stores() {
        assert_eq!(switch(Page::Epic, Pad::RB), Some(Page::Gog));
        assert_eq!(switch(Page::Amazon, Pad::RB), Some(Page::Steam));
        assert_eq!(switch(Page::Steam, Pad::RB), Some(Page::Epic));
        assert_eq!(switch(Page::Epic, Pad::LB), Some(Page::Steam));
        assert_eq!(switch(Page::Gog, Pad::A), None);
        assert_eq!(switch(Page::Downloads, Pad::RB), None);
        assert!(is_store_page(Page::Gog) && !is_store_page(Page::Emulators));
    }
}
