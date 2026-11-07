//! A fresh read of a source folded into the local copy.
//!
//! The local copy is the user's record, so it only grows: an achievement
//! once unlocked stays unlocked (a store that resets its stats, an expired
//! sign-in or a source that drops an achievement never takes it back), and
//! the earliest unlock time wins. Everything else (names, icons, rarity,
//! progress) follows the source.
//!
//! Groups (Steam's Achievement Sets) follow a read that has them, renamed,
//! reordered or with achievements moved: achievements keep their ids and
//! history. A read without groups (an old cached schema, a source without
//! them) keeps the known groups and memberships, and a DLC id the read does
//! not give stays as known.

use chrono::{DateTime, Utc};

use crate::model::{Achievement, AchievementSet};

/// The merged set and the achievements unlocked since the copy was last
/// written. A first read (no copy yet) is history: nothing is new.
pub fn merge(old: Option<&AchievementSet>, mut fresh: AchievementSet, now: DateTime<Utc>) -> (AchievementSet, Vec<Achievement>) {
    let Some(old) = old else { return (fresh, Vec::new()) };
    let mut new_unlocks = Vec::new();
    for a in &mut fresh.achievements {
        let before = old.achievements.iter().find(|o| o.id == a.id);
        if let Some(o) = before {
            for (key, value) in &o.unknown {
                a.unknown.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        match before {
            Some(o) if o.unlocked => {
                a.unlocked = true;
                a.unlocked_at = earliest(o.unlocked_at, a.unlocked_at);
                a.hardcore |= o.hardcore;
                a.noticed_at = o.noticed_at;
            }
            _ if a.unlocked => {
                a.noticed_at = Some(now);
                // Sources without unlock times (RetroAchievements without
                // the Web API key): it happened in the session just read.
                a.unlocked_at = a.unlocked_at.or(Some(now));
                new_unlocks.push(a.clone());
            }
            _ => {}
        }
    }
    // Unlocked achievements the source no longer lists stay in the record.
    let gone: Vec<Achievement> =
        old.achievements.iter().filter(|o| o.unlocked && !fresh.achievements.iter().any(|a| a.id == o.id)).cloned().collect();
    fresh.achievements.extend(gone);
    merge_groups(old, &mut fresh);
    if fresh.title.is_none() {
        fresh.title.clone_from(&old.title);
    }
    for (key, value) in &old.unknown {
        fresh.unknown.entry(key.clone()).or_insert_with(|| value.clone());
    }
    (fresh, new_unlocks)
}

fn merge_groups(old: &AchievementSet, fresh: &mut AchievementSet) {
    if fresh.groups.is_empty() {
        fresh.groups.clone_from(&old.groups);
        for a in fresh.achievements.iter_mut().filter(|a| a.group.is_none()) {
            a.group = old.achievements.iter().find(|o| o.id == a.id).and_then(|o| o.group.clone());
        }
        return;
    }
    for g in &mut fresh.groups {
        let Some(before) = old.groups.iter().find(|o| o.id == g.id) else { continue };
        g.dlc_app_id = g.dlc_app_id.or(before.dlc_app_id);
        for (key, value) in &before.unknown {
            g.unknown.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn earliest(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AchievementGroup, Source};
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
    }

    fn a(id: &str, day: Option<u32>) -> Achievement {
        Achievement { id: id.into(), name: id.into(), unlocked: day.is_some(), unlocked_at: day.map(at), ..Default::default() }
    }

    fn set(items: Vec<Achievement>) -> AchievementSet {
        AchievementSet {
            source: Source::Gog,
            external_id: "1".into(),
            title: None,
            fetched_at: at(1),
            achievements: items,
            groups: Vec::new(),
            unknown: Default::default(),
        }
    }

    #[test]
    fn a_first_read_is_history() {
        let (merged, new) = merge(None, set(vec![a("x", Some(3))]), at(20));
        assert!(new.is_empty());
        assert_eq!(merged.achievements[0].noticed_at, None);
    }

    #[test]
    fn new_unlocks_are_reported_and_stamped() {
        let old = set(vec![a("x", Some(3)), a("y", None)]);
        let (merged, new) = merge(Some(&old), set(vec![a("x", Some(3)), a("y", Some(19))]), at(20));
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, "y");
        assert_eq!(merged.achievements[1].noticed_at, Some(at(20)));
        assert_eq!(merged.achievements[0].noticed_at, None);
    }

    #[test]
    fn a_new_unlock_without_a_time_takes_the_time_it_is_noticed() {
        let old = set(vec![a("x", None)]);
        let (merged, _) = merge(Some(&old), set(vec![Achievement { unlocked: true, ..a("x", None) }]), at(20));
        assert_eq!(merged.achievements[0].unlocked_at, Some(at(20)));
        let (first, _) = merge(None, set(vec![Achievement { unlocked: true, ..a("y", None) }]), at(20));
        assert_eq!(first.achievements[0].unlocked_at, None);
    }

    #[test]
    fn unlocks_are_never_taken_back() {
        let mut old_x = a("x", Some(3));
        old_x.hardcore = true;
        old_x.noticed_at = Some(at(3));
        let old = set(vec![old_x, a("gone", Some(4)), a("locked-gone", None)]);
        let (merged, new) = merge(Some(&old), set(vec![a("x", None)]), at(20));
        assert!(new.is_empty());
        let x = &merged.achievements[0];
        assert!(x.unlocked && x.hardcore);
        assert_eq!((x.unlocked_at, x.noticed_at), (Some(at(3)), Some(at(3))));
        let ids: Vec<_> = merged.achievements.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["x", "gone"]);
    }

    #[test]
    fn the_earliest_time_wins() {
        let old = set(vec![a("x", Some(9)), a("y", Some(2))]);
        let mut late_y = a("y", Some(8));
        late_y.unlocked_at = None;
        let (merged, _) = merge(Some(&old), set(vec![a("x", Some(5)), late_y]), at(20));
        assert_eq!(merged.achievements[0].unlocked_at, Some(at(5)));
        assert_eq!(merged.achievements[1].unlocked_at, Some(at(2)));
    }

    #[test]
    fn the_title_is_kept() {
        let mut old = set(vec![]);
        old.title = Some("Super Metroid".into());
        let (merged, _) = merge(Some(&old), set(vec![]), at(20));
        assert_eq!(merged.title.as_deref(), Some("Super Metroid"));
    }

    #[test]
    fn unknown_fields_survive_refresh_for_locked_and_unlocked_achievements() {
        let mut old = set(vec![a("locked", None), a("unlocked", Some(3)), a("gone", Some(4))]);
        old.unknown.insert("futureSet".into(), serde_json::json!({"version": 2}));
        for item in &mut old.achievements {
            item.unknown.insert("futureAchievement".into(), serde_json::json!([item.id]));
        }
        let mut fresh = set(vec![a("locked", None), a("unlocked", None)]);
        fresh.achievements[0].unknown.insert("futureAchievement".into(), serde_json::json!("updated"));
        let (merged, new) = merge(Some(&old), fresh, at(20));
        assert!(new.is_empty());
        assert_eq!(merged.unknown, old.unknown);
        assert_eq!(merged.achievements[0].unknown["futureAchievement"], "updated");
        assert_eq!(merged.achievements[1].unknown, old.achievements[1].unknown);
        assert_eq!(merged.achievements[2], old.achievements[2]);
    }
    fn in_group(id: &str, day: Option<u32>, group: &str) -> Achievement {
        Achievement { group: Some(group.into()), ..a(id, day) }
    }

    fn group(id: &str, name: &str, order: i64, dlc: Option<u32>) -> AchievementGroup {
        AchievementGroup { id: id.into(), name: name.into(), order: Some(order), dlc_app_id: dlc, ..Default::default() }
    }

    #[test]
    fn renamed_reordered_and_moved_groups_keep_the_history() {
        let mut old = set(vec![in_group("x", Some(2), "1"), in_group("y", None, "2"), in_group("z", Some(5), "2")]);
        old.groups = vec![group("1", "Single Player", 0, Some(0)), group("2", "Co-op", 1, Some(0))];
        // The developer renamed and swapped the sets and moved `z`; the source now reads nothing as unlocked.
        let mut fresh = set(vec![in_group("x", None, "1"), in_group("y", None, "2"), in_group("z", None, "1")]);
        fresh.groups = vec![group("2", "Cooperative", 0, Some(0)), group("1", "Story", 1, Some(0))];
        let (merged, new) = merge(Some(&old), fresh, at(20));
        assert!(new.is_empty());
        let view: Vec<_> = merged.achievements.iter().map(|a| (a.id.as_str(), a.group.as_deref(), a.unlocked_at)).collect();
        assert_eq!(view, [("x", Some("1"), Some(at(2))), ("y", Some("2"), None), ("z", Some("1"), Some(at(5)))]);
        let names: Vec<_> = merged.groups.iter().map(|g| (g.id.as_str(), g.name.as_str())).collect();
        assert_eq!(names, [("2", "Cooperative"), ("1", "Story")]);
        assert_eq!(merged.achievements.len(), 3, "no duplicates");
    }

    #[test]
    fn a_read_without_groups_keeps_the_known_ones() {
        let mut old = set(vec![in_group("x", Some(2), "0"), in_group("dlc", None, "1")]);
        old.groups = vec![group("0", "", 0, Some(0)), group("1", "The Pale Reach", 0, Some(2561440))];
        // An old cached schema: same achievements, no groups, plus one new achievement.
        let fresh = set(vec![a("x", Some(2)), a("dlc", Some(9)), a("new", None)]);
        let (merged, new) = merge(Some(&old), fresh, at(20));
        assert_eq!(new.len(), 1);
        assert_eq!(merged.groups, old.groups);
        let view: Vec<_> = merged.achievements.iter().map(|a| (a.id.as_str(), a.group.as_deref())).collect();
        assert_eq!(view, [("x", Some("0")), ("dlc", Some("1")), ("new", None)]);
    }

    #[test]
    fn a_read_that_does_not_say_which_dlc_keeps_the_known_one() {
        let mut old = set(vec![in_group("x", None, "1")]);
        old.groups = vec![group("1", "Royal Court", 0, Some(1303182))];
        old.groups[0].unknown.insert("futureGroup".into(), serde_json::json!(1));
        let mut fresh = set(vec![in_group("x", None, "1"), in_group("y", None, "2")]);
        fresh.groups = vec![group("1", "Corte Real", 0, None), group("2", "New", 1, None)];
        let (merged, _) = merge(Some(&old), fresh, at(20));
        assert_eq!(merged.groups[0].name, "Corte Real");
        assert_eq!(merged.groups[0].dlc_app_id, Some(1303182));
        assert_eq!(merged.groups[0].unknown["futureGroup"], 1);
        assert_eq!(merged.groups[1].dlc_app_id, None);
    }
}
