//! What a console tab shows: its games (`TabSpec::source`) and, for the
//! layouts that have them, the theme's shelves (`ConsoleTheme::shelves`).
//! Both are kept in a small cache ([`SourceCache`]) shared by the frame, the
//! controller presses and the hints, and rebuilt when anything they read
//! changes (the library, the theme, the categories, the day...), so a theme
//! edit shows at once without a filter and a sort over the library per call.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::*;
use heronite_data::categories::Categories;
use heronite_data::console_theme::ShelfSpec;
use heronite_data::hltb::WikiCache;
use heronite_data::organize::{GameExtras, Organize};
use heronite_data::{CollectionGame, Library};

use super::super::CollectionView;
use super::layouts::Filters;
use crate::console::theme_rows;

/// One row of covers.
#[derive(Clone)]
pub(crate) struct Shelf {
    pub title: SharedString,
    pub keys: Vec<SharedString>,
    /// Its games before the theme's `shelfLimit` cut it (`keys` may hold fewer).
    pub total: usize,
    /// What it shows (`ShelfSpec::kind`, `statuses` already split into `status:<id>`).
    pub kind: SharedString,
}

impl Shelf {
    /// The theme's limit left games out of it.
    pub fn cut(&self) -> bool {
        self.total > self.keys.len()
    }
}

pub(super) fn by_last_played(mut v: Vec<&CollectionGame>) -> Vec<&CollectionGame> {
    v.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower)));
    v
}

pub(super) fn by_title(mut v: Vec<&CollectionGame>) -> Vec<&CollectionGame> {
    v.sort_by(|a, b| a.title_lower.cmp(&b.title_lower));
    v
}

pub(super) fn keys(v: Vec<&CollectionGame>) -> Vec<SharedString> {
    v.into_iter().map(|g| SharedString::from(g.key.clone())).collect()
}

/// The status of `id` by its slug or id (`plan-to-play`, `beaten`...).
fn status_is(lib: &Library, g: &CollectionGame, slug: &str) -> bool {
    g.status_id == slug || lib.statuses.iter().any(|s| s.id == g.status_id && s.slug == slug)
}

/// The name of the status `value` (its slug or id) in the library.
fn status_name(lib: &Library, value: &str) -> String {
    lib.statuses
        .iter()
        .find(|s| s.id == value || s.slug == value)
        .map_or_else(|| value.to_string(), |s| heronite_data::status::status_label(&s.name).to_string())
}

/// A day number for "YYYY-MM-DD..." (enough to compare ages).
fn day_number(date: &str) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(date.get(..10)?, "%Y-%m-%d").ok()?;
    Some(i64::from(d.num_days_from_ce()))
}

use chrono::Datelike as _;

/// Today, as a [`day_number`].
fn today() -> i64 {
    i64::from(chrono::Local::now().date_naive().num_days_from_ce())
}

/// A number per game that changes every day, for the daily pick: the lowest
/// is the pick.
pub(super) fn daily_rank(key: &str, day: u64) -> u64 {
    fxhash(key) ^ day.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// Everything the games and shelves of a source read; they are rebuilt when
/// any of it differs from what the cached ones were built from.
struct Inputs {
    lib: Arc<Library>,
    extras: Arc<GameExtras>,
    wiki: Arc<WikiCache>,
    categories: Categories,
    epoch: u64,
    day: i64,
}

/// A list of game keys a layout derives from a source's games (filtered and
/// sorted), with what it depends on besides the games and [`Inputs`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum ListKey {
    Grid {
        source: String,
        filters: Filters,
        /// The search text, trimmed.
        search: String,
    },
    /// The Carousel's pill `filter` (its A\u{2013}Z order).
    Carousel { source: String, filter: usize },
}

impl ListKey {
    fn source(&self) -> &str {
        match self {
            Self::Grid { source, .. } | Self::Carousel { source, .. } => source,
        }
    }
}

/// The lists of the combinations seen lately. A few at most: a search typed
/// letter by letter makes one each.
#[derive(Default)]
struct ListMemo(HashMap<ListKey, Arc<[SharedString]>>);

impl ListMemo {
    const MAX: usize = 8;

    fn get(&self, key: &ListKey) -> Option<Arc<[SharedString]>> {
        self.0.get(key).cloned()
    }

    fn insert(&mut self, key: ListKey, keys: Arc<[SharedString]>) {
        if self.0.len() >= Self::MAX {
            self.0.clear();
        }
        self.0.insert(key, keys);
    }
}

/// The games, shelves and key lists of each source seen lately, for [`Inputs`].
#[derive(Default)]
pub(super) struct SourceCache {
    built_from: Option<Inputs>,
    games: HashMap<String, Arc<Vec<Arc<CollectionGame>>>>,
    shelves: HashMap<String, Arc<Vec<Shelf>>>,
    lists: ListMemo,
}

impl SourceCache {
    /// Keeps the entries only while the inputs are what they were built from
    /// (the libraries and caches are compared by identity, held so that an
    /// address cannot be reused while it is remembered).
    fn check(&mut self, view: &CollectionView, lib: &Arc<Library>) {
        let (epoch, day) = (crate::console::look::epoch(), today());
        let same = self.built_from.as_ref().is_some_and(|b| {
            Arc::ptr_eq(&b.lib, lib)
                && Arc::ptr_eq(&b.extras, &view.game_extras)
                && Arc::ptr_eq(&b.wiki, &view.wiki)
                && b.epoch == epoch
                && b.day == day
                && b.categories == view.categories
        });
        if !same {
            self.games.clear();
            self.shelves.clear();
            self.lists.0.clear();
            self.built_from = Some(Inputs {
                lib: lib.clone(),
                extras: view.game_extras.clone(),
                wiki: view.wiki.clone(),
                categories: view.categories.clone(),
                epoch,
                day,
            });
        }
    }
}

impl CollectionView {
    pub(super) fn organize(&self) -> Organize<'_> {
        Organize { extras: Some(&self.game_extras), categories: Some(&self.categories), ..Default::default() }
    }

    /// Whether `g` belongs to a tab's `source` (see `TabSpec::source`).
    pub(super) fn console_in_source(&self, lib: &Library, g: &CollectionGame, source: &str) -> bool {
        let (kind, value) = source.split_once(':').unwrap_or((source, ""));
        match kind {
            "installed" => g.installed,
            "not-installed" => !g.installed,
            "favourites" => g.favourite,
            "pc" => !self.game_extras.systems.contains_key(&g.key),
            "emulated" => self.game_extras.systems.contains_key(&g.key),
            "status" => status_is(lib, g, value),
            "category" => self.categories.contains(value, &g.runner, &g.app_name),
            "store" => g.source_label == value,
            "platform" => self.organize().platform(g) == value,
            _ => true,
        }
    }

    /// The list `key` stands for: `order` runs once per change of the
    /// source's games or of what `key` holds, then the result is shared.
    pub(super) fn console_list(
        &self,
        key: ListKey,
        order: impl FnOnce(&[Arc<CollectionGame>], &Library) -> Vec<SharedString>,
    ) -> Arc<[SharedString]> {
        let Some(lib) = self.lib.as_ref() else { return Arc::default() };
        // Also checks the cache against its inputs.
        let games = self.console_games(key.source());
        if let Some(keys) = self.console.sources.borrow().lists.get(&key) {
            return keys;
        }
        let keys: Arc<[SharedString]> = order(&games, lib).into();
        self.console.sources.borrow_mut().lists.insert(key, keys.clone());
        keys
    }

    /// The games of `source`, in library order.
    pub(super) fn console_games(&self, source: &str) -> Arc<Vec<Arc<CollectionGame>>> {
        let Some(lib) = self.lib.as_ref() else { return Arc::default() };
        let mut cache = self.console.sources.borrow_mut();
        cache.check(self, lib);
        if let Some(games) = cache.games.get(source) {
            return games.clone();
        }
        let games: Arc<Vec<Arc<CollectionGame>>> =
            Arc::new(lib.games.iter().filter(|g| self.console_in_source(lib, g, source)).cloned().collect());
        cache.games.insert(source.to_string(), games.clone());
        games
    }

    /// The theme's shelves over `source`, empty ones left out.
    pub(super) fn console_shelves(&self, source: &str) -> Vec<Shelf> {
        self.cached_shelves(source).to_vec()
    }

    fn cached_shelves(&self, source: &str) -> Arc<Vec<Shelf>> {
        let Some(lib) = self.lib.as_ref() else { return Arc::default() };
        let cached = {
            let mut cache = self.console.sources.borrow_mut();
            cache.check(self, lib);
            cache.shelves.get(source).cloned()
        };
        if let Some(shelves) = cached {
            return shelves;
        }
        let shelves = Arc::new(self.build_shelves(lib, source));
        self.console.sources.borrow_mut().shelves.insert(source.to_string(), shelves.clone());
        shelves
    }

    fn build_shelves(&self, lib: &Library, source: &str) -> Vec<Shelf> {
        let theme = crate::console::look::theme();
        let games = self.console_games(source);
        let games: Vec<&CollectionGame> = games.iter().map(|g| g.as_ref()).collect();
        let limit = if theme.shelf_limit == 0 { usize::MAX } else { theme.shelf_limit as usize };
        // `statuses` stands for one shelf per status of the library, in its order.
        let specs: Vec<ShelfSpec> = theme
            .shelves
            .iter()
            .filter(|s| s.on)
            .flat_map(|s| {
                if s.kind == "statuses" {
                    lib.statuses.iter().map(|st| ShelfSpec::new(&format!("status:{}", st.id), true)).collect()
                } else {
                    vec![s.clone()]
                }
            })
            .collect();
        specs
            .iter()
            .filter_map(|spec| {
                let mut keys = keys(self.shelf_games(lib, &games, &spec.kind));
                let total = keys.len();
                keys.truncate(limit);
                (!keys.is_empty()).then(|| Shelf {
                    title: theme_rows::shelf_title(spec, |v| status_name(lib, v)).into(),
                    keys,
                    total,
                    kind: spec.kind.clone().into(),
                })
            })
            .collect()
    }

    fn shelf_games<'a>(&self, lib: &Library, games: &[&'a CollectionGame], kind: &str) -> Vec<&'a CollectionGame> {
        let (k, value) = kind.split_once(':').unwrap_or((kind, ""));
        let pick = |f: &dyn Fn(&CollectionGame) -> bool| games.iter().copied().filter(|g| f(g)).collect::<Vec<_>>();
        match k {
            "recent" => {
                let mut v = by_last_played(pick(&|g| g.last_played.is_some()));
                v.truncate(20);
                v
            }
            "favourites" => by_last_played(pick(&|g| g.favourite)),
            "installed" => by_last_played(pick(&|g| g.installed)),
            "added" => {
                let org = self.organize();
                let mut v: Vec<(&CollectionGame, &str)> = games.iter().filter_map(|g| Some((*g, org.added(g)?))).collect();
                v.sort_by(|a, b| b.1.cmp(a.1));
                v.into_iter().take(20).map(|(g, _)| g).collect()
            }
            "backlog-short" => {
                let mut v: Vec<(&CollectionGame, f64)> = games
                    .iter()
                    .filter(|g| status_is(lib, g, "plan-to-play") || (g.playtime_minutes == 0 && g.last_played.is_none()))
                    .filter_map(|g| Some((*g, self.wiki.hltb(&g.title, &g.game.title)?.main_story)))
                    .filter(|(_, h)| *h > 0. && *h <= 15.)
                    .collect();
                v.sort_by(|a, b| a.1.total_cmp(&b.1));
                v.into_iter().map(|(g, _)| g).collect()
            }
            "random" => {
                // A different pick each day, the same all day long.
                let day = today() as u64;
                let mut v: Vec<(u64, &CollectionGame)> = games
                    .iter()
                    .filter(|g| status_is(lib, g, "plan-to-play") || g.playtime_minutes == 0)
                    .map(|g| (daily_rank(&g.key, day), *g))
                    .collect();
                v.sort_by_key(|(h, _)| *h);
                v.into_iter().take(6).map(|(_, g)| g).collect()
            }
            "forgotten" => {
                let today = today();
                by_last_played(pick(&|g| g.installed && g.last_played.as_deref().and_then(day_number).is_none_or(|d| today - d > 90)))
            }
            "status" => by_last_played(pick(&|g| status_is(lib, g, value))),
            "store" => by_last_played(pick(&|g| g.source_label == value)),
            "platform" => {
                let org = self.organize();
                by_last_played(pick(&|g| org.platform(g) == value))
            }
            "genre" => by_last_played(pick(&|g| g.metadata.as_ref().is_some_and(|m| m.genres.iter().any(|x| x == value)))),
            "category" => by_last_played(pick(&|g| self.categories.contains(value, &g.runner, &g.app_name))),
            _ => Vec::new(),
        }
    }
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    fn key(source: &str, search: &str) -> ListKey {
        ListKey::Grid { source: source.into(), filters: Filters::default(), search: search.into() }
    }

    fn keys_of(names: &[&str]) -> Arc<[SharedString]> {
        names.iter().map(|n| SharedString::from(n.to_string())).collect()
    }

    #[test]
    fn a_grid_is_found_only_by_the_same_source_filters_and_search() {
        let mut memo = ListMemo::default();
        let all = keys_of(&["a", "b"]);
        memo.insert(key("all", ""), all.clone());
        assert!(Arc::ptr_eq(&memo.get(&key("all", "")).unwrap(), &all));
        assert!(memo.get(&key("installed", "")).is_none());
        assert!(memo.get(&key("all", "zel")).is_none());
        let filters = Filters { sort: 1, ..Filters::default() };
        assert!(memo.get(&ListKey::Grid { source: "all".into(), filters, search: String::new() }).is_none());
        // Another layout's list of the same source is another entry.
        assert!(memo.get(&ListKey::Carousel { source: "all".into(), filter: 0 }).is_none());
    }

    #[test]
    fn the_memo_stays_small() {
        let mut memo = ListMemo::default();
        for i in 0..ListMemo::MAX + 3 {
            memo.insert(key("all", &i.to_string()), keys_of(&["a"]));
        }
        assert!(memo.0.len() <= ListMemo::MAX);
        // The last one asked is still there.
        assert!(memo.get(&key("all", &(ListMemo::MAX + 2).to_string())).is_some());
    }
}
