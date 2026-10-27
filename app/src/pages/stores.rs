//! Epic Games, GOG, Amazon Games, Steam and the stores that run their own
//! program under Wine (Battle.net, the EA app, Ubisoft Connect, Rockstar
//! Games) share one entry ("Stores") in the ☰ menu, the console sidebar and
//! the welcome card: each store keeps its own page, laid out by [`sheet`]
//! with every store listed on its left (LB / RB in console mode), and a link
//! to Settings → Other launchers under them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_clients::Account;
use heronite_data::clients::Client;
use heronite_i18n::{n_, tr, trn};

use super::settings::Section;
use super::{Page, PageEvent};
use crate::console::Pad;
use crate::icons::Icon;
use crate::services::clients::{Activity, ClientState};
use crate::services::downloads::Store;
use crate::services::steam_library::{SteamState, Summary};
use crate::theme;
use crate::ui::controls::{close_button, help_text, page_heading};
use crate::ui::{glass, icon};

/// The store pages, in the order of the list (and of LB / RB).
pub const PAGES: [Page; 8] = [Page::Epic, Page::Gog, Page::Amazon, Page::Steam, Page::BattleNet, Page::Ea, Page::Ubisoft, Page::Rockstar];

/// The list's groups: the stores Heronite signs in to or reads, then the
/// ones whose own program runs under Wine.
const GROUPS: [(&str, &[Page]); 2] = [
    (n_!("Stores"), &[Page::Epic, Page::Gog, Page::Amazon, Page::Steam]),
    (n_!("Store apps"), &[Page::BattleNet, Page::Ea, Page::Ubisoft, Page::Rockstar]),
];

/// Width of the list on the left of a store page.
const NAV_W: f32 = 250.;

pub fn is_store_page(page: Page) -> bool {
    PAGES.contains(&page)
}

/// Title of the store pages' list.
pub fn title() -> &'static str {
    tr!("Stores")
}

/// The dot beside a store in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dot {
    /// Signed in, found, set up.
    On,
    /// Something to do: log in again, sign in, restart.
    Attention,
    /// Working on it: setting up, opening, reading.
    Busy,
    /// Not used yet.
    Off,
}

/// What the list says of a store: its dot and a line under its name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Glance {
    dot: Dot,
    line: String,
}

fn glance(dot: Dot, line: impl Into<String>) -> Glance {
    Glance { dot, line: line.into() }
}

/// "80 games · 1 installed".
fn counts(games: usize, installed: usize) -> String {
    format!("{} \u{b7} {}", trn!(games, "{n} game", "{n} games"), super::store::installed_count(installed))
}

/// A store Heronite signs in to (Epic Games, GOG, Amazon Games).
fn account_glance(logged_in: bool, expired: bool, owned: usize, installed: usize) -> Glance {
    if logged_in {
        glance(Dot::On, counts(owned, installed))
    } else if expired {
        glance(Dot::Attention, tr!("Log in again"))
    } else {
        glance(Dot::Off, tr!("Not logged in"))
    }
}

/// Steam: whether it is on this PC and what its library holds.
fn steam_glance(state: SteamState, summary: &Summary) -> Glance {
    let library = || counts(summary.games, summary.installed);
    match state {
        SteamState::Checking => glance(Dot::Off, tr!("Checking")),
        SteamState::NotInstalled => glance(Dot::Off, tr!("Not found")),
        SteamState::NeedsRestart => glance(Dot::Attention, tr!("Restart once")),
        SteamState::Failed => glance(Dot::Attention, tr!("Could not connect")),
        SteamState::Opening | SteamState::Restarting | SteamState::Reading => glance(Dot::Busy, library()),
        SteamState::Closed if summary.games == 0 => glance(Dot::Off, tr!("Closed")),
        SteamState::Closed | SteamState::Connected => glance(Dot::On, library()),
    }
}

/// A store whose program runs under Wine.
fn client_glance(st: &ClientState) -> Glance {
    if matches!(st.activity, Activity::SettingUp(_)) {
        return glance(Dot::Busy, tr!("Setting up\u{2026}"));
    }
    if !st.installed() {
        return glance(Dot::Off, tr!("Not set up"));
    }
    match st.account() {
        Account::SignedOut => glance(Dot::Attention, tr!("Not signed in")),
        _ if st.in_collection > 0 => glance(Dot::On, trn!(st.in_collection, "{n} game", "{n} games")),
        _ => glance(Dot::On, tr!("Ready")),
    }
}

/// What the list says of `page`, from the services already running (a
/// store whose service has not started says nothing yet).
fn glance_of(page: Page, cx: &App) -> Glance {
    let unknown = || glance(Dot::Off, "");
    let downloads = crate::services::downloads_started(cx);
    let store = |store: Store| {
        downloads.as_ref().map_or_else(unknown, |d| {
            let s = d.read(cx).store(store);
            account_glance(s.account.logged_in, s.account.expired, s.owned, s.installed)
        })
    };
    let client =
        |client: Client| crate::services::clients_started(cx).map_or_else(unknown, |svc| client_glance(svc.read(cx).state(client)));
    match page {
        Page::Epic => downloads.as_ref().map_or_else(unknown, |d| {
            let d = d.read(cx);
            let (account, stats) = (d.account(), d.stats());
            account_glance(account.logged_in, account.expired, stats.owned, stats.installed)
        }),
        Page::Gog => store(Store::Gog),
        Page::Amazon => store(Store::Amazon),
        Page::Steam => crate::services::steam_library_started(cx).map_or_else(unknown, |s| {
            let s = s.read(cx);
            steam_glance(s.state(), &s.summary)
        }),
        Page::BattleNet => client(Client::BattleNet),
        Page::Ea => client(Client::Ea),
        Page::Ubisoft => client(Client::Ubisoft),
        Page::Rockstar => client(Client::Rockstar),
        _ => unknown(),
    }
}

fn glances(cx: &App) -> Vec<Glance> {
    PAGES.iter().map(|p| glance_of(*p, cx)).collect()
}

/// Repaints a store page when what its list shows of the other stores
/// changes (a login, a setup, Steam opening), and only then: downloads and
/// polls change the services far more often. Call from the page's `new`.
pub fn watch<V: 'static>(cx: &mut Context<V>) {
    let shown = Rc::new(RefCell::new(glances(cx)));
    observe(crate::services::downloads_started(cx), &shown, cx);
    observe(crate::services::steam_library_started(cx), &shown, cx);
    observe(crate::services::clients_started(cx), &shown, cx);
}

fn observe<V: 'static, T: 'static>(entity: Option<Entity<T>>, shown: &Rc<RefCell<Vec<Glance>>>, cx: &mut Context<V>) {
    let Some(entity) = entity else { return };
    let shown = shown.clone();
    cx.observe(&entity, move |_, _, cx| {
        let now = glances(cx);
        if *shown.borrow() != now {
            *shown.borrow_mut() = now;
            cx.notify();
        }
    })
    .detach();
}

/// Horizontal room for a store page's columns: [`super::content_width`]
/// minus the list.
pub fn content_width(window: &Window) -> f32 {
    super::content_width(window) - NAV_W - 8.
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

/// A store in the list: its icon, name, a line on where it stands, and
/// its dot.
fn nav_item(page: Page, active: bool, g: Glance) -> Stateful<Div> {
    let tint = if active { theme::accent() } else { theme::nav_text() };
    div()
        .id(SharedString::from(format!("stores-nav-{page:?}")))
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(7.))
        .mb(px(2.))
        .rounded(px(12.))
        .cursor_pointer()
        .when(active, |d| d.bg(theme::accent_tint()))
        .when(!active, |d| d.hover(|s| s.bg(theme::ink_a(0.05))))
        .child(div().flex().flex_none().items_center().justify_center().size(px(30.)).rounded(px(9.)).bg(theme::ink_a(0.08)).child(icon(
            page.icon(),
            px(16.),
            tint,
        )))
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
                        .child(page.title()),
                )
                .when(!g.line.is_empty(), |d| d.child(div().text_size(px(11.5)).truncate().text_color(theme::text_muted()).child(g.line))),
        )
        .child(dot(g.dot))
}

/// Settings → Other launchers, under the stores: the games of Heroic,
/// Lutris, Bottles and Faugus come in from there.
fn other_launchers<V: EventEmitter<PageEvent> + 'static>(cx: &mut Context<V>) -> Stateful<Div> {
    div()
        .id("stores-nav-launchers")
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
        .child(icon(Icon::CallMerge, px(16.), theme::text_muted()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(13.5)).text_color(theme::nav_text()).child(tr!("Other launchers")))
                // Names: not translated.
                .child(div().text_size(px(11.5)).truncate().text_color(theme::text_dim()).child("Heroic, Lutris, Bottles, Faugus")),
        )
        .child(icon(Icon::OpenInNew, px(14.), theme::text_dim()))
        .on_click(cx.listener(|_, _, _, cx| {
            super::settings::request_section(Section::Launchers, cx);
            cx.emit(PageEvent::Navigate(Page::Settings));
        }))
}

/// Where the list and each store page are scrolled. The store pages are
/// separate views drawn one at a time, so the list keeps its place from one
/// store to the next; each page keeps its own.
struct Scrolls {
    nav: ScrollHandle,
    pages: HashMap<Page, ScrollHandle>,
}

impl Global for Scrolls {}

/// The list's scroll (`None`) or a page's.
fn scroll_of(page: Option<Page>, cx: &mut App) -> ScrollHandle {
    if !cx.has_global::<Scrolls>() {
        cx.set_global(Scrolls { nav: ScrollHandle::new(), pages: HashMap::new() });
    }
    let s = cx.global_mut::<Scrolls>();
    match page {
        None => s.nav.clone(),
        Some(p) => s.pages.entry(p).or_default().clone(),
    }
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

/// The list on the left of a store page: "Stores", the stores in their
/// groups, and Other launchers at the bottom.
fn nav<V: EventEmitter<PageEvent> + 'static>(active: Page, cx: &mut Context<V>) -> Div {
    let scroll = scroll_of(None, cx);
    let mut list = div()
        .id("stores-nav")
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .pb(px(10.))
                .child(icon(Icon::Storefront, px(18.), theme::text_primary()))
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .text_size(px(19.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .child(title()),
                ),
        );
    let page = cx.entity().downgrade();
    for (group, pages) in GROUPS {
        list = list.child(
            div().px(px(12.)).pt(px(12.)).pb(px(5.)).text_size(px(12.)).text_color(theme::text_muted()).child(heronite_i18n::tr(group)),
        );
        for p in pages.iter().copied() {
            let page = page.clone();
            list = list.child(nav_item(p, p == active, glance_of(p, cx)).on_click(move |_, _, cx| {
                if p != active {
                    page.update(cx, |_, cx| cx.emit(PageEvent::Navigate(p))).ok();
                }
            }));
        }
    }
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
        .child(with_scrollbar(div().relative().flex_1().min_h_0().child(list), "stores-nav-scrollbar", &scroll, -11., cx))
        .child(div().pt(px(12.)).child(other_launchers(cx)))
}

/// A store page: the list of stores on the left; the store's name, `lead`
/// and `body` (at most 1320px wide) on the right, scrolling. Both have a
/// scrollbar when the window is too short for them. The round
/// close button goes back to the Collection.
pub fn sheet<V: EventEmitter<PageEvent> + 'static>(
    id: impl Into<ElementId>,
    active: Page,
    lead: impl Into<SharedString>,
    cx: &mut Context<V>,
    body: impl IntoElement,
) -> Div {
    let header = div().flex().flex_col().gap(px(4.)).pr(px(48.)).pb(px(18.)).child(page_heading(active.title())).child(help_text(lead));
    let scroll = scroll_of(Some(active), cx);
    let content = div()
        .id(id.into())
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
            .child(nav(active, cx))
            .child(with_scrollbar(div().relative().flex_1().min_w_0().h_full().child(content), "stores-page-scrollbar", &scroll, 4., cx))
            .child(close),
    )
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
        assert_eq!(switch(Page::Steam, Pad::RB), Some(Page::BattleNet));
        assert_eq!(switch(Page::Rockstar, Pad::RB), Some(Page::Epic));
        assert_eq!(switch(Page::Epic, Pad::LB), Some(Page::Rockstar));
        assert_eq!(switch(Page::BattleNet, Pad::LB), Some(Page::Steam));
        assert_eq!(switch(Page::Gog, Pad::A), None);
        assert_eq!(switch(Page::Downloads, Pad::RB), None);
        assert!(is_store_page(Page::Gog) && is_store_page(Page::Ubisoft) && !is_store_page(Page::Emulators));
    }

    #[test]
    fn a_store_says_whether_it_is_signed_in_and_what_it_holds() {
        use super::{account_glance, Dot};
        let on = account_glance(true, false, 80, 1);
        assert_eq!(on.dot, Dot::On);
        assert!(on.line.contains("80") && on.line.contains('1'));
        assert_eq!(account_glance(false, true, 0, 0).dot, Dot::Attention);
        assert_eq!(account_glance(false, false, 0, 0).dot, Dot::Off);
    }

    #[test]
    fn steam_says_what_is_missing_before_its_library() {
        use super::{steam_glance, Dot, SteamState, Summary};
        let read = Summary { games: 36, installed: 8, updated_at: None };
        assert_eq!(steam_glance(SteamState::Connected, &read).dot, Dot::On);
        assert_eq!(steam_glance(SteamState::Closed, &read).dot, Dot::On);
        assert_eq!(steam_glance(SteamState::Closed, &Summary::default()).dot, Dot::Off);
        assert_eq!(steam_glance(SteamState::NotInstalled, &Summary::default()).dot, Dot::Off);
        assert_eq!(steam_glance(SteamState::NeedsRestart, &read).dot, Dot::Attention);
        assert_eq!(steam_glance(SteamState::Reading, &read).dot, Dot::Busy);
    }

    #[test]
    fn a_client_not_set_up_is_off() {
        use super::{client_glance, Dot};
        assert_eq!(client_glance(&Default::default()).dot, Dot::Off);
    }

    #[test]
    fn every_store_page_is_in_one_group_of_the_list() {
        use super::{GROUPS, PAGES};
        let listed: Vec<Page> = GROUPS.iter().flat_map(|(_, pages)| pages.iter().copied()).collect();
        assert_eq!(listed, PAGES);
    }

    #[test]
    fn every_client_is_listed_and_has_a_page_to_debug() {
        use heronite_data::clients::Client;
        for (client, page) in [
            (Client::BattleNet, Page::BattleNet),
            (Client::Ea, Page::Ea),
            (Client::Ubisoft, Page::Ubisoft),
            (Client::Rockstar, Page::Rockstar),
        ] {
            assert!(is_store_page(page));
            assert_eq!(Page::from_debug(client.id()), Some(page));
        }
    }
}
