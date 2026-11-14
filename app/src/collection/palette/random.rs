//! "Pick a random game" in the palette: from the games on screen (the
//! search, filters and categories in use), the whole Collection, the
//! installed games, the ones never played, the favourites, or one category,
//! manual or smart.
//!
//! The pick is focused in the Collection at once, and the palette stays open
//! on it: "Pick another" first (Enter rolls again from the same games), then
//! the game's own actions (Play, Install...). The last picks are not offered
//! again while there are other games to pick.

use std::collections::{HashSet, VecDeque};
use std::hash::BuildHasher;

use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::sources::{Act, Entry, Kind};
use super::view::PaletteView;
use crate::collection::view_filters::category_label;
use crate::collection::CollectionView;
use crate::icons::Icon;

/// Picks remembered so they do not come back at once.
const REMEMBERED: usize = 8;

/// Which games a random pick is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::collection) enum RandomScope {
    /// The cards on screen.
    OnScreen,
    All,
    Installed,
    NotPlayed,
    Favourites,
    /// A manual category, by name.
    Category(String),
    /// A smart category, by id.
    Smart(String),
}

impl RandomScope {
    fn id(&self) -> String {
        match self {
            Self::OnScreen => "random:screen".into(),
            Self::All => "random:all".into(),
            Self::Installed => "random:installed".into(),
            Self::NotPlayed => "random:not-played".into(),
            Self::Favourites => "random:favourites".into(),
            Self::Category(name) => format!("random:category:{name}"),
            Self::Smart(id) => format!("random:smart:{id}"),
        }
    }
}

/// One of `pool`, at random, leaving out the `recent` picks while others
/// remain. `roll` is any random number.
pub(super) fn pick<'a>(pool: &'a [String], recent: &VecDeque<String>, roll: u64) -> Option<&'a String> {
    let fresh: Vec<&String> = pool.iter().filter(|k| !recent.contains(k)).collect();
    let from: Vec<&String> = if fresh.is_empty() { pool.iter().collect() } else { fresh };
    (!from.is_empty()).then(|| from[(roll % from.len() as u64) as usize])
}

fn roll() -> u64 {
    // Each `RandomState` is seeded from the system's random source.
    std::collections::hash_map::RandomState::new().hash_one(std::time::Instant::now())
}

/// What the games of `scope` are, in a sentence with their count.
fn scope_line(scope: &RandomScope, n: usize, name: &str) -> String {
    match scope {
        RandomScope::OnScreen => trn!(n, "From the {n} game on screen", "From the {n} games on screen"),
        RandomScope::All => trn!(n, "From the whole Collection: {n} game", "From the whole Collection: {n} games"),
        RandomScope::Installed => trn!(n, "From your {n} installed game", "From your {n} installed games"),
        RandomScope::NotPlayed => trn!(n, "From the {n} game you have not played", "From the {n} games you have not played"),
        RandomScope::Favourites => trn!(n, "From your {n} favourite", "From your {n} favourites"),
        RandomScope::Category(_) | RandomScope::Smart(_) => trn!(n, "From {name}: {n} game", "From {name}: {n} games", name),
    }
}

fn keywords() -> &'static str {
    "random pick surprise roll dice shuffle roulette lucky aleatorio sortear surpresa dado sorteio azar zufall würfel hasard dé"
}

impl CollectionView {
    /// The keys `scope` picks from: one per card, hidden games only while
    /// they are shown.
    fn random_pool(&self, scope: &RandomScope) -> Vec<String> {
        if *scope == RandomScope::OnScreen {
            return self.shown_keys().into_iter().map(|k| k.to_string()).collect();
        }
        let Some(lib) = self.lib.as_ref() else { return Vec::new() };
        let smart = match scope {
            RandomScope::Smart(id) => match self.smart_groups.get(id) {
                Some(g) => Some(self.group_members(g)),
                None => return Vec::new(),
            },
            _ => None,
        };
        let keeps = |g: &CollectionGame| match scope {
            RandomScope::OnScreen | RandomScope::All => true,
            RandomScope::Installed => g.installed,
            RandomScope::NotPlayed => g.playtime_minutes == 0 && g.last_played.is_none(),
            RandomScope::Favourites => g.favourite,
            RandomScope::Category(name) => self.categories.matches(&[name.clone()].into(), &g.runner, &g.app_name),
            RandomScope::Smart(_) => smart.as_ref().is_some_and(|m| m.contains(&g.key)),
        };
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for g in lib.games.iter().filter(|g| self.ui.show_hidden || !g.hidden) {
            // A card stands for all its copies: it counts once, by its first.
            let card = if self.ui.group_copies { lib.copies_of(&g.key).first().map_or(&g.key, |c| &c.key) } else { &g.key };
            if !seen.contains(card) && self.with_copies(card).iter().any(|c| keeps(c)) {
                seen.insert(card.clone());
                out.push(card.clone());
            }
        }
        out
    }

    /// The palette's "Pick a random game" entries: the games on screen and
    /// the usual sets first, each category when something is typed.
    pub(super) fn random_entries(&self) -> Vec<Entry> {
        let mut v = Vec::new();
        let mut add = |scope: RandomScope, title: String, name: &str, secondary: bool| {
            let n = self.random_pool(&scope).len();
            let mut e = Entry::new(
                Kind::Action,
                Act::Random(scope.clone()),
                scope.id(),
                &title,
                &scope_line(&scope, n, name),
                Icon::Casino,
                keywords(),
            );
            e.secondary = secondary;
            if n == 0 {
                e.disable(tr!("No game to pick from"));
            }
            v.push(e);
        };
        add(RandomScope::OnScreen, tr!("Pick a random game").into(), "", false);
        add(RandomScope::Installed, tr!("Pick a random installed game").into(), "", false);
        add(RandomScope::NotPlayed, tr!("Pick a random game you have not played").into(), "", false);
        add(RandomScope::Favourites, tr!("Pick a random favourite").into(), "", true);
        add(RandomScope::All, tr!("Pick a random game from the whole Collection").into(), "", true);
        for name in self.categories.names().map(str::to_owned).collect::<Vec<_>>() {
            let label = category_label(&name);
            add(RandomScope::Category(name), tr!("Pick a random game: {category}", category = label.as_str()), &label, true);
        }
        let smart: Vec<(String, String)> = self.smart_groups.groups.iter().map(|g| (g.id.clone(), g.name.clone())).collect();
        for (id, name) in smart {
            add(RandomScope::Smart(id), tr!("Pick a random game: {category}", category = name.as_str()), &name, true);
        }
        v
    }

    /// Picks a game of `scope`, focuses it and lists it in the palette with
    /// "Pick another" and its actions.
    pub(super) fn palette_random(&mut self, scope: RandomScope, view: &Entity<PaletteView>, window: &mut Window, cx: &mut Context<Self>) {
        let pool = self.random_pool(&scope);
        let Some(key) = pick(&pool, &self.palette.random_recent, roll()).cloned() else {
            self.close_palette(window, cx);
            self.show_toast(tr!("No game to pick from"), true, cx);
            return;
        };
        let recent = &mut self.palette.random_recent;
        recent.retain(|k| *k != key);
        recent.push_front(key.clone());
        recent.truncate(REMEMBERED.min(pool.len().saturating_sub(1)));
        // Picked from the cards on screen, the copy shown stays the card's.
        let game = self.acting_copy(&key).or_else(|| self.lib.as_ref().and_then(|l| l.get(&key)).cloned());
        let Some(game) = game else { return };
        self.palette_select(&key, window, cx);
        let name = match &scope {
            RandomScope::Category(n) => category_label(n),
            RandomScope::Smart(id) => self.smart_groups.get(id).map(|g| g.name.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        let mut again = Entry::new(
            Kind::Action,
            Act::Random(scope.clone()),
            scope.id(),
            tr!("Pick another"),
            &scope_line(&scope, pool.len(), &name),
            Icon::Casino,
            keywords(),
        );
        if pool.len() < 2 {
            again.disable(tr!("It is the only game to pick from"));
        }
        let items = self.game_actions(&game, cx);
        let mut entries = vec![again];
        entries.extend(super::sources::action_entries(&game.key, &game.title, &items, false));
        let title: SharedString = game.title.clone().into();
        view.update(cx, |p, cx| p.show_game(title, entries, window, cx));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::pick;

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("g{i}")).collect()
    }

    #[test]
    fn a_pick_comes_from_the_pool() {
        let pool = keys(5);
        for roll in 0..20 {
            assert!(pool.contains(pick(&pool, &VecDeque::new(), roll).unwrap()));
        }
        assert_eq!(pick(&[], &VecDeque::new(), 3), None);
    }

    #[test]
    fn recent_picks_wait_while_others_remain() {
        let pool = keys(3);
        let recent: VecDeque<String> = ["g0".to_string(), "g1".to_string()].into();
        for roll in 0..10 {
            assert_eq!(pick(&pool, &recent, roll).map(String::as_str), Some("g2"));
        }
        let all: VecDeque<String> = pool.iter().cloned().collect();
        assert!(pick(&pool, &all, 7).is_some(), "every game was picked lately: any of them");
    }
}
