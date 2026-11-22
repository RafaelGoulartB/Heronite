//! What the Achievements screen lists: a game's achievements filtered (LB /
//! RB) and in an order (Y), and the counts of its header. Pure, so it is
//! tested without a window.

use heronite_achievements::{Achievement, AchievementSet};
use heronite_i18n::tr;

use crate::theme::{self, RarityTier};

/// The pills between LB and RB.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Filter {
    #[default]
    All,
    Unlocked,
    Locked,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Unlocked, Filter::Locked];

    pub fn keeps(self, a: &Achievement) -> bool {
        match self {
            Filter::All => true,
            Filter::Unlocked => a.unlocked,
            Filter::Locked => !a.unlocked,
        }
    }

    /// "All 41", "Unlocked 36", "Locked 5".
    pub fn label(self, n: usize) -> String {
        match self {
            Filter::All => tr!("All {n}", n),
            Filter::Unlocked => tr!("Unlocked {n}", n),
            Filter::Locked => tr!("Locked {n}", n),
        }
    }

    /// The filter `delta` steps away, wrapping around.
    pub fn step(self, delta: isize) -> Filter {
        let at = Self::ALL.iter().position(|f| *f == self).unwrap_or(0) as isize;
        Self::ALL[(at + delta).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// The order Y switches through.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Order {
    /// The game's own order (as its store lists them).
    #[default]
    Default,
    /// The latest unlocks first, then the locked ones closest to being
    /// unlocked, the secret ones last.
    Newest,
    /// The fewest players first; unknown rarity last.
    Rarest,
}

impl Order {
    pub fn next(self) -> Order {
        match self {
            Order::Default => Order::Newest,
            Order::Newest => Order::Rarest,
            Order::Rarest => Order::Default,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Order::Default => tr!("Default order"),
            Order::Newest => tr!("Newest first"),
            Order::Rarest => tr!("Rarest first"),
        }
    }
}

/// The positions in `set.achievements` of what `filter` keeps, in `order`.
/// Ties keep the game's order, so a refresh never shuffles them.
pub(super) fn listed(set: &AchievementSet, filter: Filter, order: Order) -> Vec<usize> {
    let all = &set.achievements;
    let mut ix: Vec<usize> = (0..all.len()).filter(|&i| filter.keeps(&all[i])).collect();
    match order {
        Order::Default => {}
        Order::Newest => {
            // Unlocked (newest first, those without a time after them), then
            // locked: counted progress first, then the most common, secrets last.
            let progress = |a: &Achievement| a.progress.filter(|p| p.current > 0).map_or(-1., |p| p.fraction());
            ix.sort_by(|&x, &y| {
                let (a, b) = (&all[x], &all[y]);
                b.unlocked
                    .cmp(&a.unlocked)
                    .then_with(|| b.unlocked_at.cmp(&a.unlocked_at))
                    .then_with(|| a.is_secret().cmp(&b.is_secret()))
                    .then_with(|| progress(b).total_cmp(&progress(a)))
                    .then_with(|| b.rarity.unwrap_or(-1.).total_cmp(&a.rarity.unwrap_or(-1.)))
            });
        }
        Order::Rarest => ix.sort_by(|&x, &y| all[x].rarity.unwrap_or(f32::MAX).total_cmp(&all[y].rarity.unwrap_or(f32::MAX))),
    }
    ix
}

/// How many are in each filter: all, unlocked, locked.
pub(super) fn counts(set: &AchievementSet) -> [usize; 3] {
    let unlocked = set.unlocked();
    [set.total(), unlocked, set.total() - unlocked]
}

/// The rarity tiers, rarest first, as the header's board shows them.
pub(super) const TIERS: [RarityTier; 4] = [RarityTier::UltraRare, RarityTier::Rare, RarityTier::Uncommon, RarityTier::Common];

pub(super) fn tier_label(tier: RarityTier) -> &'static str {
    match tier {
        RarityTier::UltraRare => tr!("Ultra rare"),
        RarityTier::Rare => tr!("Rare"),
        RarityTier::Uncommon => tr!("Uncommon"),
        RarityTier::Common => tr!("Common"),
    }
}

/// Unlocked and total of each tier (of the achievements whose rarity is
/// known), in [`TIERS`]' order; `None` when no rarity is known.
pub(super) fn tier_counts(set: &AchievementSet) -> Option<[(usize, usize); 4]> {
    let mut out = [(0, 0); 4];
    let mut any = false;
    for a in &set.achievements {
        let Some(r) = a.rarity else { continue };
        any = true;
        let t = TIERS.iter().position(|t| *t == theme::rarity_tier(r)).unwrap_or(3);
        out[t].1 += 1;
        if a.unlocked {
            out[t].0 += 1;
        }
    }
    any.then_some(out)
}

/// The latest unlocked achievements with an icon, newest first.
pub(super) fn recent_icons(set: &AchievementSet, max: usize) -> Vec<&Achievement> {
    set.recent().into_iter().filter(|a| !a.icon.is_empty()).take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> AchievementSet {
        serde_json::from_value(serde_json::json!({
            "source": "steam", "externalId": "1", "fetchedAt": "2026-10-01T10:00:00Z",
            "achievements": [
                {"id": "a", "name": "A", "rarity": 50.0},
                {"id": "b", "name": "B", "unlocked": true, "unlockedAt": "2026-09-01T10:00:00Z", "rarity": 2.0, "icon": "b.png"},
                {"id": "c", "name": "C", "hidden": true, "rarity": 60.0},
                {"id": "d", "name": "D", "unlocked": true, "unlockedAt": "2026-09-05T10:00:00Z", "rarity": 20.0, "icon": "d.png"},
                {"id": "e", "name": "E", "progress": {"current": 3, "max": 10}, "rarity": 10.0},
                {"id": "f", "name": "F"}
            ]
        }))
        .unwrap()
    }

    fn ids(set: &AchievementSet, ix: &[usize]) -> Vec<String> {
        ix.iter().map(|&i| set.achievements[i].id.clone()).collect()
    }

    #[test]
    fn the_default_order_is_the_games_and_filters_keep_it() {
        let s = set();
        assert_eq!(ids(&s, &listed(&s, Filter::All, Order::Default)), ["a", "b", "c", "d", "e", "f"]);
        assert_eq!(ids(&s, &listed(&s, Filter::Unlocked, Order::Default)), ["b", "d"]);
        assert_eq!(ids(&s, &listed(&s, Filter::Locked, Order::Default)), ["a", "c", "e", "f"]);
        assert_eq!(counts(&s), [6, 2, 4]);
    }

    #[test]
    fn newest_first_puts_the_latest_unlocks_then_the_closest_and_secrets_last() {
        let s = set();
        assert_eq!(ids(&s, &listed(&s, Filter::All, Order::Newest)), ["d", "b", "e", "a", "f", "c"]);
    }

    #[test]
    fn rarest_first_leaves_unknown_rarity_last() {
        let s = set();
        assert_eq!(ids(&s, &listed(&s, Filter::All, Order::Rarest)), ["b", "e", "d", "a", "c", "f"]);
        assert_eq!(ids(&s, &listed(&s, Filter::Locked, Order::Rarest)), ["e", "a", "c", "f"]);
    }

    #[test]
    fn tiers_count_only_known_rarity() {
        let s = set();
        // b 2% ultra rare (unlocked), e 10% rare, d 20% uncommon (unlocked), a 50% and c 60% common.
        assert_eq!(tier_counts(&s), Some([(1, 1), (0, 1), (1, 1), (0, 2)]));
        let mut none = s.clone();
        none.achievements.iter_mut().for_each(|a| a.rarity = None);
        assert_eq!(tier_counts(&none), None);
    }

    #[test]
    fn filters_and_orders_cycle() {
        assert_eq!(Filter::All.step(-1), Filter::Locked);
        assert_eq!(Filter::Locked.step(1), Filter::All);
        assert_eq!(Order::Default.next().next().next(), Order::Default);
        assert_eq!(
            ids(
                &set(),
                &recent_icons(&set(), 12).iter().map(|a| set().achievements.iter().position(|x| x.id == a.id).unwrap()).collect::<Vec<_>>()
            ),
            ["d", "b"]
        );
    }
}
