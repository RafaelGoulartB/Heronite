//! ☰ navigation: the hover menu listing the pages, and the chrome of the
//! non-Collection pages (slim header + the page view).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::header::Menu;
use super::CollectionView;
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};
use crate::pages::{self, Page, PageCtx, PageEvent};
use crate::theme;
use crate::ui::dropdown::{menu_separator, Align};
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
fn read_nav_meta(data_paths: &heronite_data::DataPaths) -> (Option<String>, Option<usize>) {
    let wine = std::fs::read_to_string(data_paths.store_config())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.pointer("/defaultSettings/wineVersion/name").and_then(|n| n.as_str()).map(str::to_owned))
        .map(|n| crate::dialogs::settings_panel::logic::display_wine_name(&n))
        .or_else(|| Some(crate::dialogs::settings_panel::logic::default_wine().name));
    let emulators = heronite_emulation::store::list_user_emulators(data_paths).ok().map(|l| l.len());
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

/// One row of the ☰ menu: icon in a small rounded square, title, and on
/// the right a one-line meta (accent when `live`) or a key hint.
fn nav_row(id: SharedString, ic: Icon, title: &str, trailing: Trailing, active: bool) -> Stateful<Div> {
    let trailing = match trailing {
        Trailing::None => None,
        Trailing::Meta(text, live) => Some(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(if live { theme::accent() } else { theme::text_dim() })
                .child(text)
                .into_any_element(),
        ),
        Trailing::Key(k) => Some(crate::ui::controls::kbd(k).into_any_element()),
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(40.))
        .px(px(6.))
        .rounded(px(theme::R_ITEM))
        .when(active, |d| d.bg(theme::accent_tint()))
        .cursor_pointer()
        .hover(|s| s.bg(if active { theme::accent_tint() } else { theme::menu_item_hover() }))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(8.))
                .bg(if active { theme::accent_tint() } else { theme::white_a(0.06) })
                .child(icon(ic, px(16.), if active { theme::accent() } else { theme::text_muted() })),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(14.))
                .text_color(if active { white() } else { theme::text_primary().into() })
                .child(title.to_string()),
        )
        .child(div().flex().flex_1().min_w_0().justify_end().children(trailing))
}

enum Trailing {
    None,
    /// Text, and whether it reports something running.
    Meta(String, bool),
    Key(&'static str),
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
        let data_paths = self.data_paths.clone();
        let lib = self.lib.clone();
        cx.spawn(async move |this, cx| {
            let (wine, emulators, month) = cx
                .background_spawn(async move {
                    let (wine, emulators) = read_nav_meta(&data_paths);
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
        let open = self.open_menu == Some(Menu::Nav);
        self.hover_dropdown(
            Menu::Nav,
            crate::ui::tool_button("btn-Nav", Icon::Menu, open),
            Align::Left,
            |this, cx| {
                let games = this.lib.as_ref().map_or(0, |l| l.games.len());
                let svc = crate::services::try_downloads(cx);
                let svc = svc.as_ref().map(|s| s.read(cx));
                let signed_in = svc.is_some_and(|s| s.account().logged_in);
                let downloads = downloads_meta(svc);
                let meta = &this.pages.meta;
                let pending = || "\u{2026}".to_string();
                let emulators = match meta.emulators {
                    Some(0) => "None".to_string(),
                    Some(n) => plural(n, "emulator", "emulators"),
                    None => pending(),
                };
                let downloading = downloads != "Idle";
                let trailing = |page: Page| match page {
                    Page::Collection => Trailing::Meta(plural(games, "game", "games"), false),
                    Page::Downloads => Trailing::Meta(downloads.clone(), downloading),
                    Page::Statistics => Trailing::Meta(meta.month.clone().unwrap_or_else(pending), false),
                    Page::Epic => Trailing::Meta(if signed_in { "Signed in".into() } else { "Not signed in".into() }, false),
                    Page::WineManager => Trailing::Meta(meta.wine.clone().unwrap_or_else(pending), false),
                    Page::Emulators => Trailing::Meta(emulators.clone(), false),
                    Page::Settings => Trailing::Key("Ctrl ,"),
                };
                let mut p = glass::glass_panel(theme::R_POPOVER).w(px(300.)).p(px(6.)).flex().flex_col().gap(px(2.));
                // Where the games are, then what sets them up.
                for (i, group) in [
                    &[Page::Collection, Page::Downloads, Page::Statistics][..],
                    &[Page::Epic, Page::WineManager, Page::Emulators][..],
                ]
                .into_iter()
                .enumerate()
                {
                    if i > 0 {
                        p = p.child(menu_separator());
                    }
                    for &page in group {
                        p = p.child(
                            nav_row(SharedString::from(format!("nav-{page:?}")), page.icon(), page.title(), trailing(page), this.page == page)
                                .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx))),
                        );
                    }
                }
                p.child(menu_separator())
                    .child(
                        nav_row("nav-add-game".into(), Icon::Add, "Add game", Trailing::Meta("Sideload or ROM".into(), false), false)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_add_game(window, cx);
                            })),
                    )
                    .child(
                        nav_row("nav-console".into(), Icon::Tv, "Console mode", Trailing::None, false)
                            .on_click(cx.listener(|this, _, window, cx| this.set_console(true, window, cx))),
                    )
                    .child(menu_separator())
                    .child(
                        nav_row("nav-Settings".into(), Icon::Settings, "Settings", trailing(Page::Settings), this.page == Page::Settings)
                            .on_click(cx.listener(|this, _, window, cx| this.navigate(Page::Settings, window, cx))),
                    )
                    .child(
                        nav_row("nav-palette".into(), Icon::FaSearch, "Command palette", Trailing::Key("Ctrl K"), false)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_menu = None;
                                this.open_palette(window, cx);
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
        PageCtx { data_paths: self.data_paths.clone(), app: self.app_paths.clone() }
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

    /// Palette commands of every page, also of the ones not shown yet
    /// (`pages/commands.rs`).
    pub(super) fn page_commands(&self, cx: &App) -> Vec<(Page, PageCommand)> {
        fn list<P: PaletteCommands>(page: Page, view: &Option<AnyView>, ctx: &CommandCtx, cx: &App, out: &mut Vec<(Page, PageCommand)>) {
            let entity = view.clone().and_then(|v| v.downcast::<P>().ok());
            out.extend(P::palette_commands(entity.as_ref().map(|e| e.read(cx)), ctx, cx).into_iter().map(|c| (page, c)));
        }
        let ctx = CommandCtx { settings: self.lib.as_ref().map(|l| &l.settings) };
        let p = &self.pages;
        let mut out = Vec::new();
        list::<pages::downloads::DownloadsPage>(Page::Downloads, &p.downloads, &ctx, cx, &mut out);
        list::<pages::wine_manager::WineManagerPage>(Page::WineManager, &p.wine, &ctx, cx, &mut out);
        list::<pages::epic::EpicPage>(Page::Epic, &p.epic, &ctx, cx, &mut out);
        list::<pages::emulators::EmulatorsPage>(Page::Emulators, &p.emulators, &ctx, cx, &mut out);
        list::<pages::stats::StatsPage>(Page::Statistics, &p.stats, &ctx, cx, &mut out);
        list::<pages::settings::SettingsPage>(Page::Settings, &p.settings, &ctx, cx, &mut out);
        out
    }

    /// Runs a page's palette command, showing the page first when the
    /// command asks for it (and creating it in any case).
    pub(super) fn run_page_command(&mut self, page: Page, id: &str, navigate: bool, window: &mut Window, cx: &mut Context<Self>) {
        fn run<P: PaletteCommands>(view: &Option<AnyView>, id: &str, window: &mut Window, cx: &mut App) {
            if let Some(e) = view.clone().and_then(|v| v.downcast::<P>().ok()) {
                e.update(cx, |p, cx| p.run_palette_command(id, window, cx));
            }
        }
        if navigate {
            self.navigate(page, window, cx);
        } else {
            self.ensure_page(page, window, cx);
        }
        let p = &self.pages;
        match page {
            Page::Collection => {}
            Page::Downloads => run::<pages::downloads::DownloadsPage>(&p.downloads, id, window, cx),
            Page::WineManager => run::<pages::wine_manager::WineManagerPage>(&p.wine, id, window, cx),
            Page::Epic => run::<pages::epic::EpicPage>(&p.epic, id, window, cx),
            Page::Emulators => run::<pages::emulators::EmulatorsPage>(&p.emulators, id, window, cx),
            Page::Statistics => run::<pages::stats::StatsPage>(&p.stats, id, window, cx),
            Page::Settings => run::<pages::settings::SettingsPage>(&p.settings, id, window, cx),
        }
        cx.notify();
    }

    /// The page shown has a dialog of its own open (the global shortcuts
    /// and the palette wait for it).
    pub(super) fn page_modal(&self, cx: &App) -> bool {
        fn modal<P: PaletteCommands>(view: &Option<AnyView>, cx: &App) -> bool {
            view.clone().and_then(|v| v.downcast::<P>().ok()).is_some_and(|e| e.read(cx).page_modal())
        }
        let p = &self.pages;
        match self.page {
            Page::Collection => false,
            Page::Downloads => modal::<pages::downloads::DownloadsPage>(&p.downloads, cx),
            Page::WineManager => modal::<pages::wine_manager::WineManagerPage>(&p.wine, cx),
            Page::Epic => modal::<pages::epic::EpicPage>(&p.epic, cx),
            Page::Emulators => modal::<pages::emulators::EmulatorsPage>(&p.emulators, cx),
            Page::Statistics => modal::<pages::stats::StatsPage>(&p.stats, cx),
            Page::Settings => modal::<pages::settings::SettingsPage>(&p.settings, cx),
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
        view.update(cx, |p, cx| p.set_library(lib, &self.cover_chains, &self.stage_chains, cx));
    }

    /// Shows the Statistics page with the Year in review of this year.
    pub(super) fn open_year_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Page::Statistics, window, cx);
        let Some(view) = self.pages.stats.clone() else { return };
        let Ok(view) = view.downcast::<pages::stats::StatsPage>() else { return };
        let year = chrono::Datelike::year(&chrono::Local::now().date_naive());
        view.update(cx, |p, cx| p.open_review(year, cx));
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
