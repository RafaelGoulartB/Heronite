//! Browse tab state and its searches of Nexus Mods and Thunderstore, which
//! run off the UI thread and are dropped when a newer one has started.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::nexus::{self, SearchMod, SearchSort};
use heronite_mods::thunderstore::{self, Listing, Ordering};

use super::{ModsDialog, View};
use crate::ui::fields;

/// Mods per request.
const PAGE: u32 = 24;
/// Typing pauses this long before a search starts.
const DEBOUNCE: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Source {
    Nexus,
    Thunderstore,
}

pub(super) struct Results<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub more: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// A search has answered (also with nothing).
    pub loaded: bool,
    /// Thunderstore: the last page fetched.
    page: u32,
    /// Counts the searches of this source; an answer for an older one is dropped.
    gen: u64,
}

impl<T> Default for Results<T> {
    fn default() -> Self {
        Self { items: Vec::new(), total: 0, more: false, loading: false, error: None, loaded: false, page: 0, gen: 0 }
    }
}

impl<T> Results<T> {
    fn clear(&mut self) {
        let gen = self.gen + 1;
        *self = Self { gen, ..Self::default() };
    }
}

pub(super) struct Browse {
    pub source: Source,
    pub input: Entity<InputState>,
    pub nexus_sort: SearchSort,
    pub ts_order: Ordering,
    pub nexus: Results<SearchMod>,
    pub ts: Results<Listing>,
    /// Installs asked for here (`n:<mod id>`, `t:<namespace>-<name>`) until the mod shows in the list.
    pub queued: HashSet<String>,
    debounce: Option<Task<()>>,
}

impl Browse {
    pub fn new(window: &mut Window, cx: &mut Context<ModsDialog>) -> Self {
        let input = fields::new_input(window, cx, "", tr!("Search mods"));
        cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.search_typed(cx);
            }
        })
        .detach();
        Self {
            source: Source::Nexus,
            input,
            nexus_sort: SearchSort::Endorsements,
            ts_order: Ordering::MostDownloaded,
            nexus: Results::default(),
            ts: Results::default(),
            queued: HashSet::new(),
            debounce: None,
        }
    }

    pub fn reset_results(&mut self) {
        self.nexus.clear();
        self.ts.clear();
    }
}

impl ModsDialog {
    /// The sites this game can be searched on.
    pub(super) fn sources(v: &View) -> Vec<Source> {
        let mut s = Vec::new();
        if v.g.info.nexus_domain.is_some() {
            s.push(Source::Nexus);
        }
        if v.g.info.thunderstore.is_some() {
            s.push(Source::Thunderstore);
        }
        // A game with a Thunderstore community is modded there first (BepInEx,
        // MelonLoader, Hades II's own loader): it comes first.
        if v.g.info.thunderstore.is_some() {
            s.reverse();
        }
        s
    }

    /// The Browse tab came up (or the game loaded under it): picks a site and first search.
    pub(super) fn browse_shown(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.snapshot(cx) else { return };
        let sources = Self::sources(&v);
        let (n, t) = (&self.browse.nexus, &self.browse.ts);
        let untouched = !n.loaded && !n.loading && n.error.is_none() && !t.loaded && !t.loading && t.error.is_none();
        if untouched || !sources.contains(&self.browse.source) {
            if let Some(first) = sources.first() {
                self.browse.source = *first;
            }
        }
        let fresh = match self.browse.source {
            Source::Nexus => !self.browse.nexus.loaded && !self.browse.nexus.loading && self.browse.nexus.error.is_none(),
            Source::Thunderstore => !self.browse.ts.loaded && !self.browse.ts.loading && self.browse.ts.error.is_none(),
        };
        if fresh && !sources.is_empty() {
            self.run_search(false, cx);
        }
    }

    pub(super) fn set_source(&mut self, source: Source, cx: &mut Context<Self>) {
        self.browse.source = source;
        self.browse_shown(cx);
        cx.notify();
    }

    /// The search box changed: search once the typing pauses.
    fn search_typed(&mut self, cx: &mut Context<Self>) {
        self.browse.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| {
                this.browse.reset_results();
                this.browse_shown(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Searches again from the first page with the sort in force.
    pub(super) fn restart_search(&mut self, cx: &mut Context<Self>) {
        match self.browse.source {
            Source::Nexus => self.browse.nexus.clear(),
            Source::Thunderstore => self.browse.ts.clear(),
        }
        self.browse_shown(cx);
        cx.notify();
    }

    /// Asks the site for a page (`more`: the one after the last).
    pub(super) fn run_search(&mut self, more: bool, cx: &mut Context<Self>) {
        let Some(v) = self.snapshot(cx) else { return };
        let query = self.browse.input.read(cx).value().trim().to_string();
        match self.browse.source {
            Source::Nexus => {
                let Some(domain) = v.g.info.nexus_domain.clone() else { return };
                let r = &mut self.browse.nexus;
                if !more {
                    r.clear();
                }
                r.loading = true;
                r.error = None;
                let (gen, offset, sort) = (r.gen, r.items.len() as u32, self.browse.nexus_sort);
                cx.spawn(async move |this, cx| {
                    let res = cx.background_spawn(async move { nexus::search(&domain, &query, sort, offset, PAGE, false) }).await;
                    this.update(cx, |this, cx| {
                        let r = &mut this.browse.nexus;
                        if r.gen != gen {
                            return;
                        }
                        r.loading = false;
                        r.loaded = true;
                        match res {
                            Ok(page) => {
                                r.total = page.total;
                                r.items.extend(page.mods);
                                r.more = (r.items.len() as u64) < page.total;
                            }
                            Err(e) => r.error = Some(e),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Source::Thunderstore => {
                let Some(community) = v.g.info.thunderstore.clone() else { return };
                let r = &mut self.browse.ts;
                if !more {
                    r.clear();
                }
                r.loading = true;
                r.error = None;
                let (gen, page, order) = (r.gen, r.page + 1, self.browse.ts_order);
                cx.spawn(async move |this, cx| {
                    let res = cx.background_spawn(async move { thunderstore::search(&community, &query, order, page) }).await;
                    this.update(cx, |this, cx| {
                        let r = &mut this.browse.ts;
                        if r.gen != gen {
                            return;
                        }
                        r.loading = false;
                        r.loaded = true;
                        match res {
                            Ok(p) => {
                                r.total = p.count;
                                r.page = page;
                                r.items.extend(p.packages);
                                r.more = p.has_next;
                            }
                            Err(e) => r.error = Some(e),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.notify();
    }
}
