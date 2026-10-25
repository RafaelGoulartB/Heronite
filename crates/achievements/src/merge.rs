//! A fresh read of a source folded into the local copy.
//!
//! The local copy is the user's record, so it only grows: an achievement
//! once unlocked stays unlocked (a store that resets its stats, an expired
//! sign-in or a source that drops an achievement never takes it back), and
//! the earliest unlock time wins. Everything else (names, icons, rarity,
//! progress) follows the source.

use chrono::{DateTime, Utc};

use crate::model::{Achievement, AchievementSet};

/// The merged set and the achievements unlocked since the copy was last
/// written. A first read (no copy yet) is history: nothing is new.
pub fn merge(old: Option<&AchievementSet>, mut fresh: AchievementSet, now: DateTime<Utc>) -> (AchievementSet, Vec<Achievement>) {
    let Some(old) = old else { return (fresh, Vec::new()) };
    let mut new_unlocks = Vec::new();
    for a in &mut fresh.achievements {
        let before = old.achievements.iter().find(|o| o.id == a.id);
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
    if fresh.title.is_none() {
        fresh.title.clone_from(&old.title);
    }
    (fresh, new_unlocks)
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
    use crate::model::Source;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
    }

    fn a(id: &str, day: Option<u32>) -> Achievement {
        Achievement { id: id.into(), name: id.into(), unlocked: day.is_some(), unlocked_at: day.map(at), ..Default::default() }
    }

    fn set(items: Vec<Achievement>) -> AchievementSet {
        AchievementSet { source: Source::Gog, external_id: "1".into(), title: None, fetched_at: at(1), achievements: items }
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
}
