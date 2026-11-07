//! What the achievements dialog lists: one of the game's sections (Steam's
//! Achievement Sets) or all of them, filtered, sorted and searched without
//! matching the text of a masked secret.

use std::collections::HashSet;

use heronite_achievements::{Achievement, AchievementSet, Section};

/// The section shown: all achievements, one group, or the ones in no known
/// group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Pick {
    All,
    Group(String),
    Ungrouped,
}

impl Pick {
    pub(super) fn of(section: &Section) -> Self {
        section.group_id().map_or(Pick::Ungrouped, |id| Pick::Group(id.to_string()))
    }

    /// The pick if the game still has that section, else all (a refresh
    /// dropped the group or the game has a flat list).
    pub(super) fn valid_in(self, sections: &[Section]) -> Self {
        if self == Pick::All || sections.iter().any(|s| Pick::of(s) == self) {
            self
        } else {
            Pick::All
        }
    }

    fn contains(&self, set: &AchievementSet, a: &Achievement) -> bool {
        match self {
            Pick::All => true,
            Pick::Group(id) => set.group_of(a).is_some_and(|g| g.id == *id),
            Pick::Ungrouped => set.group_of(a).is_none(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Filter {
    All,
    Unlocked,
    Locked,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Sort {
    Recent,
    Rarest,
}

/// The section's achievements in the sort's order, filtered and searched.
pub(super) fn listed<'a>(
    set: &'a AchievementSet,
    pick: &Pick,
    filter: Filter,
    sort: Sort,
    revealed: &HashSet<String>,
    hide_secrets: bool,
    query: &str,
) -> Vec<&'a Achievement> {
    let query = query.trim().to_lowercase();
    let mut list: Vec<&Achievement> = match sort {
        Sort::Recent => {
            set.recent().into_iter().chain(set.up_next()).chain(set.achievements.iter().filter(|a| !a.unlocked && a.hidden)).collect()
        }
        Sort::Rarest => {
            let mut list: Vec<&Achievement> = set.achievements.iter().collect();
            list.sort_by(|a, b| a.rarity.unwrap_or(101.).total_cmp(&b.rarity.unwrap_or(101.)));
            list
        }
    };
    list.retain(|a| pick.contains(set, a));
    list.retain(|a| match filter {
        Filter::All => true,
        Filter::Unlocked => a.unlocked,
        Filter::Locked => !a.unlocked,
    });
    list.retain(|a| {
        query.is_empty()
            || (!(hide_secrets && a.is_secret() && !revealed.contains(&a.id))
                && (a.name.to_lowercase().contains(&query) || a.description.to_lowercase().contains(&query)))
    });
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> AchievementSet {
        serde_json::from_value(serde_json::json!({
            "source": "steam", "externalId": "1", "fetchedAt": "2026-10-01T10:00:00Z",
            "achievements": [
                {"id": "secret", "name": "Spoiler", "description": "Hidden ending", "hidden": true},
                {"id": "earned", "name": "Earned", "unlocked": true, "hidden": true},
                {"id": "counted", "name": "Counted", "progress": {"current": 3, "max": 10}, "rarity": 4.2}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn filters_and_sorts_keep_counts_and_counted_progress() {
        let set = set();
        let revealed = HashSet::new();
        for sort in [Sort::Recent, Sort::Rarest] {
            let all = listed(&set, &Pick::All, Filter::All, sort, &revealed, true, "");
            assert_eq!(all.len(), 3);
            assert_eq!(listed(&set, &Pick::All, Filter::Unlocked, sort, &revealed, true, "").len(), 1);
            assert_eq!(listed(&set, &Pick::All, Filter::Locked, sort, &revealed, true, "").len(), 2);
            let counted = all.iter().find(|a| a.id == "counted").unwrap();
            assert_eq!(counted.progress.unwrap().current, 3);
            assert_eq!(counted.rarity, Some(4.2));
        }
    }

    fn grouped() -> AchievementSet {
        // The shape of DREDGE's sets (https://steamcommunity.com/stats/1562430/achievements/).
        serde_json::from_value(serde_json::json!({
            "source": "steam", "externalId": "1562430", "fetchedAt": "2026-10-01T10:00:00Z",
            "groups": [{"id": "0", "name": "", "dlcAppId": 0}, {"id": "1", "name": "The Pale Reach", "dlcAppId": 2561440}],
            "achievements": [
                {"id": "CATCH_FISH_ROD_1", "name": "Lifted From the Deep", "unlocked": true, "unlockedAt": "2026-09-01T10:00:00Z", "group": "0"},
                {"id": "COMPLETE_CHAPTER_2", "name": "The Secret", "description": "Surrender the Music Box.", "hidden": true, "group": "0"},
                {"id": "DLC_3_1", "name": "Polar Angler", "rarity": 3.5, "progress": {"current": 2, "max": 9}, "group": "1"},
                {"id": "DLC_3_3", "name": "Feeding Time", "description": "Feed the Narwhal.", "hidden": true, "group": "1"},
                {"id": "LOOSE", "name": "Loose", "group": "7"}
            ]
        }))
        .unwrap()
    }

    fn ids(list: &[&Achievement]) -> Vec<String> {
        list.iter().map(|a| a.id.clone()).collect()
    }

    #[test]
    fn a_section_lists_its_own_achievements_with_their_data() {
        let set = grouped();
        let revealed = HashSet::new();
        let dlc = Pick::Group("1".into());
        let list = listed(&set, &dlc, Filter::All, Sort::Rarest, &revealed, true, "");
        assert_eq!(ids(&list), ["DLC_3_1", "DLC_3_3"]);
        assert_eq!((list[0].rarity, list[0].progress.unwrap().current), (Some(3.5), 2));
        assert_eq!(ids(&listed(&set, &dlc, Filter::Locked, Sort::Recent, &revealed, true, "")), ["DLC_3_1", "DLC_3_3"]);
        assert!(listed(&set, &dlc, Filter::Unlocked, Sort::Recent, &revealed, true, "").is_empty());
        let base = Pick::Group("0".into());
        assert_eq!(ids(&listed(&set, &base, Filter::Unlocked, Sort::Recent, &revealed, true, "")), ["CATCH_FISH_ROD_1"]);
        assert_eq!(ids(&listed(&set, &Pick::Ungrouped, Filter::All, Sort::Recent, &revealed, true, "")), ["LOOSE"]);
        assert_eq!(listed(&set, &Pick::All, Filter::All, Sort::Recent, &revealed, true, "").len(), 5);
        // Secret text stays masked inside a section too.
        assert!(listed(&set, &dlc, Filter::All, Sort::Recent, &revealed, true, "narwhal").is_empty());
        let revealed = HashSet::from(["DLC_3_3".to_string()]);
        assert_eq!(ids(&listed(&set, &dlc, Filter::All, Sort::Recent, &revealed, true, "narwhal")), ["DLC_3_3"]);
    }

    #[test]
    fn a_pick_the_game_no_longer_has_falls_back_to_all() {
        let set = grouped();
        let sections = set.sections();
        assert_eq!(sections.iter().map(Pick::of).collect::<Vec<_>>(), [Pick::Group("0".into()), Pick::Group("1".into()), Pick::Ungrouped]);
        assert_eq!(Pick::Group("1".into()).valid_in(&sections), Pick::Group("1".into()));
        assert_eq!(Pick::Group("9".into()).valid_in(&sections), Pick::All);
        assert_eq!(Pick::Group("1".into()).valid_in(&[]), Pick::All);
    }

    #[test]
    fn search_never_matches_masked_names_or_descriptions() {
        let set = set();
        let revealed = HashSet::new();
        for query in ["spoiler", "ending"] {
            for sort in [Sort::Recent, Sort::Rarest] {
                assert!(listed(&set, &Pick::All, Filter::All, sort, &revealed, true, query).is_empty());
                assert_eq!(listed(&set, &Pick::All, Filter::All, sort, &revealed, false, query).len(), 1);
                let revealed = HashSet::from(["secret".into()]);
                assert_eq!(listed(&set, &Pick::All, Filter::All, sort, &revealed, true, query).len(), 1);
            }
        }
        assert_eq!(listed(&set, &Pick::All, Filter::All, Sort::Recent, &revealed, true, "earned").len(), 1);
    }
}
