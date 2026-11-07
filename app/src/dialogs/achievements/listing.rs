use std::collections::HashSet;

use heronite_achievements::{Achievement, AchievementSet};

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

pub(super) fn listed<'a>(
    set: &'a AchievementSet,
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
            let all = listed(&set, Filter::All, sort, &revealed, true, "");
            assert_eq!(all.len(), 3);
            assert_eq!(listed(&set, Filter::Unlocked, sort, &revealed, true, "").len(), 1);
            assert_eq!(listed(&set, Filter::Locked, sort, &revealed, true, "").len(), 2);
            let counted = all.iter().find(|a| a.id == "counted").unwrap();
            assert_eq!(counted.progress.unwrap().current, 3);
            assert_eq!(counted.rarity, Some(4.2));
        }
    }

    #[test]
    fn search_never_matches_masked_names_or_descriptions() {
        let set = set();
        let revealed = HashSet::new();
        for query in ["spoiler", "ending"] {
            for sort in [Sort::Recent, Sort::Rarest] {
                assert!(listed(&set, Filter::All, sort, &revealed, true, query).is_empty());
                assert_eq!(listed(&set, Filter::All, sort, &revealed, false, query).len(), 1);
                let revealed = HashSet::from(["secret".into()]);
                assert_eq!(listed(&set, Filter::All, sort, &revealed, true, query).len(), 1);
            }
        }
        assert_eq!(listed(&set, Filter::All, Sort::Recent, &revealed, true, "earned").len(), 1);
    }
}
