//! Steam, Epic Games, GOG, Amazon Games and the stores that run their own
//! program under Wine (Battle.net, the EA app, Ubisoft Connect, Rockstar
//! Games) share one entry ("Stores") in the ☰ menu, the console sidebar and
//! the welcome card: each store keeps its own page, laid out by [`sheet`]
//! with every store listed on its left (LB / RB in console mode), and a link
//! to Settings → Other launchers under them.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_clients::Account;
use heronite_data::clients::Client;
use heronite_i18n::{n_, tr, trn};

use super::settings::Section;
use super::side_nav::{self, Dot};
use super::{Page, PageEvent};
use crate::console::Pad;
use crate::icons::Icon;
use crate::services::clients::{Activity, ClientState};
use crate::services::downloads::Store;
use crate::services::steam_library::{SteamState, Summary};

/// The store pages, in the order of the list (and of LB / RB).
pub const PAGES: [Page; 8] = [Page::Steam, Page::Epic, Page::Gog, Page::Amazon, Page::BattleNet, Page::Ea, Page::Ubisoft, Page::Rockstar];

/// The list's groups: the stores Heronite signs in to or reads, then the
/// ones whose own program runs under Wine.
const GROUPS: [(&str, &[Page]); 2] = [
    (n_!("Stores"), &[Page::Steam, Page::Epic, Page::Gog, Page::Amazon]),
    (n_!("Store apps"), &[Page::BattleNet, Page::Ea, Page::Ubisoft, Page::Rockstar]),
];

pub fn is_store_page(page: Page) -> bool {
    PAGES.contains(&page)
}

/// The store pages this system has, in [`PAGES`] order.
pub fn shown() -> Vec<Page> {
    PAGES.into_iter().filter(|p| p.available()).collect()
}

/// Title of the store pages' list.
pub fn title() -> &'static str {
    tr!("Stores")
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

/// Horizontal room for a store page's columns, beside the list.
pub fn content_width(window: &Window) -> f32 {
    side_nav::content_width(window)
}

/// Settings → Other launchers, under the stores: the games of Heroic,
/// Lutris, Bottles and Faugus come in from there.
fn other_launchers<V: EventEmitter<PageEvent> + 'static>(cx: &mut Context<V>) -> Stateful<Div> {
    // Names: not translated.
    side_nav::link("stores-nav-launchers", Icon::CallMerge, tr!("Other launchers"), "Heroic, Lutris, Bottles, Faugus").on_click(
        cx.listener(|_, _, _, cx| {
            super::settings::request_section(Section::Launchers, cx);
            cx.emit(PageEvent::Navigate(Page::Settings));
        }),
    )
}

/// The list on the left of a store page: "Stores", the stores in their
/// groups, and Other launchers at the bottom.
fn nav<V: EventEmitter<PageEvent> + 'static>(active: Page, cx: &mut Context<V>) -> Div {
    let page = cx.entity().downgrade();
    let mut items: Vec<AnyElement> = vec![];
    for (group, pages) in GROUPS {
        if !pages.iter().any(|p| p.available()) {
            continue;
        }
        items.push(side_nav::group(heronite_i18n::tr(group), None).into_any_element());
        for p in pages.iter().copied().filter(|p| p.available()) {
            let (page, g) = (page.clone(), glance_of(p, cx));
            let id = SharedString::from(format!("stores-nav-{p:?}"));
            items.push(
                side_nav::item(id, p.icon(), p.title(), g.line, g.dot, p == active)
                    .on_click(move |_, _, cx| {
                        if p != active {
                            page.update(cx, |_, cx| cx.emit(PageEvent::Navigate(p))).ok();
                        }
                    })
                    .into_any_element(),
            );
        }
    }
    let heading = side_nav::heading(Icon::Storefront, title(), None);
    let footer = heronite_platform::supported(heronite_platform::Feature::LauncherImports).then(|| other_launchers(cx).into_any_element());
    side_nav::nav("stores-nav", heading, items, footer, cx)
}

/// A store page: the list of stores on the left; the store's name, `lead`
/// and `body` on the right (see [`side_nav::sheet`]).
pub fn sheet<V: EventEmitter<PageEvent> + 'static>(
    id: impl Into<SharedString>,
    active: Page,
    lead: impl Into<SharedString>,
    cx: &mut Context<V>,
    body: impl IntoElement,
) -> Div {
    let header = side_nav::header(active.title(), [], lead, None);
    let nav = nav(active, cx);
    side_nav::sheet(id, nav, header, cx, body)
}

/// The console top bar's tabs (switched with LB / RB, see [`switch`]).
pub fn console_tabs(active: Page) -> Div {
    let pages = shown();
    let names: Vec<SharedString> = pages.iter().map(|p| SharedString::from(p.title())).collect();
    crate::console::kit::tabs(&names, pages.iter().position(|p| *p == active).unwrap_or(0))
}

/// The store LB / RB moves to from `active`, if `pad` is one of them.
pub fn switch(active: Page, pad: Pad) -> Option<Page> {
    switch_in(&shown(), active, pad)
}

fn switch_in(pages: &[Page], active: Page, pad: Pad) -> Option<Page> {
    let i = pages.iter().position(|p| *p == active)?;
    let n = pages.len();
    match pad {
        Pad::LB => Some(pages[(i + n - 1) % n]),
        Pad::RB => Some(pages[(i + 1) % n]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_store_page, Pad, Page, PAGES};

    #[test]
    fn lb_and_rb_cycle_through_the_stores() {
        let switch = |active, pad| super::switch_in(&PAGES, active, pad);
        assert_eq!(switch(Page::Steam, Pad::RB), Some(Page::Epic));
        assert_eq!(switch(Page::Epic, Pad::RB), Some(Page::Gog));
        assert_eq!(switch(Page::Amazon, Pad::RB), Some(Page::BattleNet));
        assert_eq!(switch(Page::Rockstar, Pad::RB), Some(Page::Steam));
        assert_eq!(switch(Page::Steam, Pad::LB), Some(Page::Rockstar));
        assert_eq!(switch(Page::BattleNet, Pad::LB), Some(Page::Amazon));
        assert_eq!(switch(Page::Gog, Pad::A), None);
        assert_eq!(switch(Page::Downloads, Pad::RB), None);
        assert!(is_store_page(Page::Gog) && is_store_page(Page::Ubisoft) && !is_store_page(Page::Emulators));
        // Only the stores this system has.
        assert_eq!(super::switch_in(&[Page::Steam], Page::Steam, Pad::RB), Some(Page::Steam));
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
