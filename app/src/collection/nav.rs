//! ☰ navigation: the hover menu listing the pages, and the chrome of the
//! non-Collection pages (slim header + the page view).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::header::Menu;
use super::CollectionView;
use crate::icons::Icon;
use crate::pages::{self, Page, PageCtx, PageEvent};
use crate::theme;
use crate::ui::dropdown::Align;
use crate::ui::{glass, icon};

/// Lazily created page views.
#[derive(Default)]
pub struct Pages {
    downloads: Option<AnyView>,
    wine: Option<AnyView>,
    epic: Option<AnyView>,
    emulators: Option<AnyView>,
    stats: Option<AnyView>,
    settings: Option<AnyView>,
    subs: Vec<Subscription>,
    meta: NavMeta,
    /// Controller hooks of the pages (console mode).
    hooks: std::collections::HashMap<Page, crate::console::PadHook>,
}

impl Pages {
    pub(super) fn hook(&self, page: Page) -> Option<crate::console::PadHook> {
        self.hooks.get(&page).cloned()
    }
}

/// Live one-line facts the ☰ menu tiles show. Read off the UI thread
/// (`sync_nav_meta`) when the app starts and whenever the menu opens.
#[derive(Default)]
struct NavMeta {
    wine: Option<String>,
    emulators: Option<usize>,
    /// Play time this month ("12h 5m this month").
    month: Option<String>,
    loading: bool,
    loaded_at: Option<std::time::Instant>,
}

/// This month's play time from the loaded library's sessions.
fn month_meta(lib: &heronite_data::Library) -> String {
    use heronite_data::stats;
    let history = stats::history(lib.games.iter().map(|g| stats::GameInput {
        sessions: g.sessions.as_deref().map_or(&[], Vec::as_slice),
        lifetime_minutes: 0,
        first_played: None,
    }));
    let today = chrono::Local::now().date_naive();
    let secs = stats::compute(&history, stats::Period::of(stats::Span::Month, today), today).total_secs;
    if secs > 0. {
        format!("{} this month", stats::fmt_secs(secs))
    } else {
        "Nothing this month".into()
    }
}

/// Default Wine/Proton name (`config.json` > `defaultSettings.wineVersion`)
/// and the number of configured emulators.
fn read_nav_meta(heroic: &heronite_data::HeroicPaths) -> (Option<String>, Option<usize>) {
    let wine = std::fs::read_to_string(heroic.config())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.pointer("/defaultSettings/wineVersion/name").and_then(|n| n.as_str()).map(str::to_owned))
        .map(|n| crate::dialogs::heroic_settings::logic::display_wine_name(&n))
        .or_else(|| Some(crate::dialogs::heroic_settings::logic::default_wine().name));
    let emulators = heronite_emulation::store::list_user_emulators(heroic).ok().map(|l| l.len());
    (wine, emulators)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "1 active · 42%", "Paused · 42%", "3 queued" or "Idle".
pub(super) fn downloads_meta(svc: Option<&crate::services::downloads::Downloads>) -> String {
    use heronite_epic::manager::DmState;
    let Some(svc) = svc else { return "Idle".into() };
    let snap = svc.snapshot();
    if snap.queue.is_empty() {
        return "Idle".into();
    }
    let percent = snap.current.as_ref().and_then(|(_, p)| p.as_ref()).map(|p| format!(" \u{b7} {}%", p.percent.round() as u32));
    match snap.state {
        DmState::Running => format!("{} active{}", snap.queue.len(), percent.unwrap_or_default()),
        DmState::Paused => format!("Paused{}", percent.unwrap_or_default()),
        DmState::Idle => format!("{} queued", snap.queue.len()),
    }
}

/// One tile of the ☰ grid: icon, title, one-line meta.
fn tile(id: SharedString, ic: Icon, title: &str, meta: String, active: bool, dashed: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .p(px(12.))
        .rounded(px(theme::R_TILE))
        .border_1()
        .border_color(if active {
            theme::accent_tint_border()
        } else if dashed {
            theme::white_a(0.2)
        } else {
            theme::white_a(0.)
        })
        .when(dashed, |d| d.border_dashed())
        .bg(if active {
            theme::accent_tint()
        } else if dashed {
            theme::white_a(0.)
        } else {
            theme::card_fill()
        })
        .cursor_pointer()
        .hover(|s| s.bg(if active { theme::accent_tint() } else { theme::white_a(0.09) }))
        .child(div().mb(px(6.)).child(icon(ic, px(20.), theme::accent())))
        .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).truncate().child(title.to_string()))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(meta))
}

/// Full-width row under the tiles (Settings, Command palette) with its kbd hint.
fn bottom_row(id: &'static str, ic: Icon, label: &str, hint: &str, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .mt(px(8.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(theme::R_INPUT))
        .bg(if active { theme::accent_tint() } else { theme::card_fill() })
        .cursor_pointer()
        .hover(|s| s.bg(if active { theme::accent_tint() } else { theme::white_a(0.09) }))
        .text_size(px(14.))
        .text_color(if active { white() } else { theme::text_primary().into() })
        .child(icon(ic, px(16.), if active { theme::accent() } else { theme::text_muted() }))
        .child(div().flex_1().child(label.to_string()))
        .when(!hint.is_empty(), |d| d.child(crate::ui::controls::kbd(hint.to_string())))
}

impl CollectionView {
    /// Refreshes [`NavMeta`] in the background (at start and when the menu opens).
    pub(super) fn sync_nav_meta(&mut self, cx: &mut Context<Self>) {
        let meta = &mut self.pages.meta;
        let wanted = meta.loaded_at.is_none() || self.open_menu == Some(Menu::Nav);
        if meta.loading || !wanted || meta.loaded_at.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(5)) {
            return;
        }
        meta.loading = true;
        let heroic = self.heroic.clone();
        let lib = self.lib.clone();
        cx.spawn(async move |this, cx| {
            let (wine, emulators, month) = cx
                .background_spawn(async move {
                    let (wine, emulators) = read_nav_meta(&heroic);
                    (wine, emulators, lib.map(|l| month_meta(&l)))
                })
                .await;
            this.update(cx, |this, cx| {
                let m = &mut this.pages.meta;
                m.loading = false;
                m.loaded_at = Some(std::time::Instant::now());
                if m.wine != wine || m.emulators != emulators || m.month != month {
                    m.wine = wine;
                    m.emulators = emulators;
                    m.month = month;
                    cx.notify();
                }
                // The library arrived while this ran without it.
                if m.month.is_none() && this.lib.is_some() {
                    m.loaded_at = None;
                    this.sync_nav_meta(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn nav_dropdown(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        self.hover_dropdown(
            Menu::Nav,
            Icon::Menu,
            Align::Left,
            |this, cx| {
                let games = this.lib.as_ref().map_or(0, |l| l.games.len());
                let svc = crate::services::try_downloads(cx);
                let svc = svc.as_ref().map(|s| s.read(cx));
                let signed_in = svc.is_some_and(|s| s.account().logged_in);
                let downloads = downloads_meta(svc);
                let meta = &this.pages.meta;
                let wine = meta.wine.clone().unwrap_or_else(|| "\u{2026}".into());
                let emulators = match meta.emulators {
                    Some(0) => "No emulators".to_string(),
                    Some(n) => plural(n, "emulator", "emulators"),
                    None => "\u{2026}".into(),
                };
                let month = meta.month.clone().unwrap_or_else(|| "\u{2026}".into());
                let metas = [
                    plural(games, "game", "games"),
                    downloads,
                    wine,
                    if signed_in { "Signed in".into() } else { "Not signed in".into() },
                    emulators,
                    month,
                ];
                let mut tiles: Vec<Stateful<Div>> = Page::ALL
                    .iter()
                    .zip(metas)
                    .map(|(&page, meta)| {
                        tile(SharedString::from(format!("nav-{page:?}")), page.icon(), page.title(), meta, this.page == page, false)
                            .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx)))
                    })
                    .collect();
                tiles.push(
                    tile("nav-add-game".into(), Icon::Add, "Add game", "Sideload or ROM".into(), false, true)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_add_game(window, cx);
                        })),
                );
                let mut p = glass::glass_panel(theme::R_POPOVER).w(px(330.)).p(px(10.)).flex().flex_col().gap(px(8.));
                while !tiles.is_empty() {
                    let rest = tiles.split_off(2.min(tiles.len()));
                    p = p.child(div().flex().gap(px(8.)).children(std::mem::replace(&mut tiles, rest)));
                }
                p.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            bottom_row("nav-console", Icon::Tv, "Console mode", "", false)
                                .on_click(cx.listener(|this, _, window, cx| this.set_console(true, window, cx))),
                        )
                        .child(
                            bottom_row("nav-settings", Icon::Settings, "Settings", "Ctrl ,", this.page == Page::Settings)
                                .on_click(cx.listener(|this, _, window, cx| this.navigate(Page::Settings, window, cx))),
                        )
                        .child(
                            bottom_row("nav-palette", Icon::FaSearch, "Command palette", "Ctrl K", false)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_menu = None;
                                    this.open_palette(window, cx);
                                })),
                        ),
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
        if page == Page::Settings {
            // Coming back: lists and flags may have changed in the Collection.
            self.offer_settings_data(cx);
            if let Some(v) = self.pages.settings.clone().and_then(|v| v.downcast::<pages::settings::SettingsPage>().ok()) {
                v.update(cx, |p, cx| p.refresh(window, cx));
            }
        }
        if page == Page::Statistics {
            // Hand the page the library after the slot below is filled.
            cx.defer_in(window, |this, _, cx| this.offer_stats_data(cx));
        }
        let slot_empty = match page {
            Page::Collection => return,
            Page::Downloads => self.pages.downloads.is_none(),
            Page::WineManager => self.pages.wine.is_none(),
            Page::Epic => self.pages.epic.is_none(),
            Page::Emulators => self.pages.emulators.is_none(),
            Page::Statistics => self.pages.stats.is_none(),
            Page::Settings => self.pages.settings.is_none(),
        };
        if !slot_empty {
            return;
        }
        match page {
            Page::Collection => {}
            Page::Downloads => {
                let v = cx.new(|cx| pages::downloads::DownloadsPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.hooks.insert(Page::Downloads, crate::console::PadHook::new(&v));
                self.pages.downloads = Some(v.into());
            }
            Page::WineManager => {
                let v = cx.new(|cx| pages::wine_manager::WineManagerPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.hooks.insert(Page::WineManager, crate::console::PadHook::new(&v));
                self.pages.wine = Some(v.into());
            }
            Page::Epic => {
                let v = cx.new(|cx| pages::epic::EpicPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.hooks.insert(Page::Epic, crate::console::PadHook::new(&v));
                self.pages.epic = Some(v.into());
            }
            Page::Emulators => {
                let v = cx.new(|cx| pages::emulators::EmulatorsPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.hooks.insert(Page::Emulators, crate::console::PadHook::new(&v));
                self.pages.emulators = Some(v.into());
            }
            Page::Statistics => {
                let v = cx.new(|cx| pages::stats::StatsPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.hooks.insert(Page::Statistics, crate::console::PadHook::new(&v));
                self.pages.stats = Some(v.into());
            }
            Page::Settings => {
                let v = cx.new(|cx| pages::settings::SettingsPage::new(ctx, window, cx));
                self.pages.subs.push(cx.subscribe_in(&v, window, Self::on_page_event));
                self.pages.hooks.insert(Page::Settings, crate::console::PadHook::new(&v));
                self.pages.settings = Some(v.into());
            }
        }
    }

    /// A (re)loaded library: refresh the ☰ tiles that read it and the
    /// Statistics page.
    pub(super) fn nav_library_loaded(&mut self, cx: &mut Context<Self>) {
        self.pages.meta.loaded_at = None;
        self.sync_nav_meta(cx);
        self.offer_stats_data(cx);
    }

    /// Gives the Statistics page the current library (it skips the work
    /// when the library did not change).
    pub(super) fn offer_stats_data(&self, cx: &mut Context<Self>) {
        let (Some(lib), Some(view)) = (self.lib.clone(), self.pages.stats.clone()) else { return };
        let Ok(view) = view.downcast::<pages::stats::StatsPage>() else { return };
        view.update(cx, |p, cx| p.set_library(lib, &self.cover_chains, cx));
    }

    fn on_page_event<E>(&mut self, _: &Entity<E>, ev: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PageEvent::LibraryChanged => self.reload(cx),
            PageEvent::Toast(text, error) => self.show_toast(text.clone(), *error, cx),
            PageEvent::Navigate(page) => self.navigate(*page, window, cx),
            PageEvent::ScanRoms(initial) => self.open_scan_roms(initial.clone(), window, cx),
            PageEvent::ShowHidden(on) => self.set_show_hidden(*on, cx),
            PageEvent::ShowGame(key) => {
                self.navigate(Page::Collection, window, cx);
                if self.lib.as_ref().is_some_and(|l| l.get(key).is_some()) {
                    self.select(key.clone().into(), true);
                    self.reveal_selected();
                }
            }
        }
    }

    /// The current page's view alone (console mode draws its own chrome).
    pub(super) fn render_page_view(&self) -> Option<AnyElement> {
        let view = match self.page {
            Page::Collection => return None,
            Page::Downloads => self.pages.downloads.clone(),
            Page::WineManager => self.pages.wine.clone(),
            Page::Epic => self.pages.epic.clone(),
            Page::Emulators => self.pages.emulators.clone(),
            Page::Statistics => self.pages.stats.clone(),
            Page::Settings => self.pages.settings.clone(),
        }?;
        Some(view.into_any_element())
    }

    /// Header + body of the current non-Collection page.
    pub(super) fn render_page(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = match self.page {
            Page::Collection => return None,
            Page::Downloads => self.pages.downloads.clone(),
            Page::WineManager => self.pages.wine.clone(),
            Page::Epic => self.pages.epic.clone(),
            Page::Emulators => self.pages.emulators.clone(),
            Page::Statistics => self.pages.stats.clone(),
            Page::Settings => self.pages.settings.clone(),
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
                // Darker veil than the Collection: pages are mostly text
                // (follows the Interface appearance: opaque when Solid).
                .bg(theme::page_backdrop())
                .child(header)
                .child(div().flex_1().min_h_0().child(view))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{downloads_meta, plural};

    #[test]
    fn tile_meta_texts() {
        assert_eq!(plural(1, "emulator", "emulators"), "1 emulator");
        assert_eq!(plural(6, "emulator", "emulators"), "6 emulators");
        assert_eq!(plural(207, "game", "games"), "207 games");
        // No download service yet: nothing is queued.
        assert_eq!(downloads_meta(None), "Idle");
    }
}
