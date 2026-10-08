//! ☰ navigation: the hover menu listing the pages, and the chrome of the
//! non-Collection pages (slim header + the page view).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::header::Menu;
use super::CollectionView;
use crate::icons::Icon;
use crate::pages::{self, Page, PageCtx, PageEvent};
use crate::theme;
use crate::ui::dropdown::{menu_panel, menu_item, menu_separator, Align};
use crate::ui::icon;

/// Lazily created page views.
#[derive(Default)]
pub struct Pages {
    downloads: Option<AnyView>,
    wine: Option<AnyView>,
    epic: Option<AnyView>,
    subs: Vec<Subscription>,
}

impl CollectionView {
    pub(super) fn nav_dropdown(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        self.hover_dropdown(
            Menu::Nav,
            Icon::Menu,
            Align::Left,
            |this, cx| {
                let mut p = menu_panel(220.);
                for page in Page::ALL {
                    let active = this.page == page;
                    p = p.child(
                        menu_item(SharedString::from(format!("nav-{page:?}")), active)
                            .child(icon(page.icon(), px(16.), if active { theme::accent() } else { rgb(0xcaf3fd) }))
                            .child(page.title())
                            .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx))),
                    );
                }
                p.child(menu_separator())
                    .child(
                        menu_item("nav-add-game", false)
                            .child(icon(Icon::Add, px(16.), rgb(0xcaf3fd)))
                            .child("Add game")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_add_game(window, cx);
                            })),
                    )
                    .child(
                        menu_item("nav-settings", false)
                            .child(icon(Icon::Settings, px(16.), rgb(0xcaf3fd)))
                            .child("Settings")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_menu = None;
                                this.open_settings(window, cx);
                            })),
                    )
                    .into_any_element()
            },
            cx,
        )
    }

    pub(super) fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.context_menu = None;
        self.page = page;
        self.ensure_page(page, window, cx);
        cx.notify();
    }

    fn page_ctx(&self) -> PageCtx {
        PageCtx { heroic: self.heroic.clone(), app: self.app_paths.clone() }
    }

    fn ensure_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let ctx = self.page_ctx();
        let slot_empty = match page {
            Page::Collection => return,
            Page::Downloads => self.pages.downloads.is_none(),
            Page::WineManager => self.pages.wine.is_none(),
            Page::Epic => self.pages.epic.is_none(),
        };
        if !slot_empty {
            return;
        }
        match page {
            Page::Collection => {}
            Page::Downloads => {
                let v = cx.new(|cx| pages::downloads::DownloadsPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.downloads = Some(v.into());
            }
            Page::WineManager => {
                let v = cx.new(|cx| pages::wine_manager::WineManagerPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.wine = Some(v.into());
            }
            Page::Epic => {
                let v = cx.new(|cx| pages::epic::EpicPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.epic = Some(v.into());
            }
        }
    }

    fn on_page_event<E>(&mut self, _: &Entity<E>, ev: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PageEvent::LibraryChanged => self.reload(cx),
            PageEvent::Toast(text, error) => self.show_toast(text.clone(), *error, cx),
            PageEvent::Navigate(page) => self.navigate(*page, window, cx),
        }
    }

    /// Header + body of the current non-Collection page.
    pub(super) fn render_page(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = match self.page {
            Page::Collection => return None,
            Page::Downloads => self.pages.downloads.clone(),
            Page::WineManager => self.pages.wine.clone(),
            Page::Epic => self.pages.epic.clone(),
        }?;
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .h(theme::HEADER_H)
            .px(px(12.))
            .gap(px(10.))
            .child(self.nav_dropdown(cx))
            .child(
                div()
                    .font_family(theme::FONT_HEADING)
                    .text_size(px(17.6))
                    .font_weight(theme::W_BOLD)
                    .text_color(rgb(0xcaf3fd))
                    .child(self.page.title()),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                // Darker veil than the Collection: pages are mostly text.
                .bg(theme::rgb_a(10, 12, 16, 0.55))
                .child(header)
                .child(div().flex_1().min_h_0().child(view))
                .into_any_element(),
        )
    }
}
