//! The achievements of one game as every source is turned into: what the
//! focus panel, the Timeline and the "All achievements" dialog draw, and
//! what the local copy (`store`) keeps.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a set of achievements comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Steam,
    Gog,
    Epic,
    #[serde(rename = "retroachievements")]
    RetroAchievements,
}

impl Source {
    pub const ALL: [Source; 4] = [Source::Steam, Source::Gog, Source::Epic, Source::RetroAchievements];

    /// Name shown in the UI (a brand: never translated).
    pub fn label(self) -> &'static str {
        match self {
            Source::Steam => "Steam",
            Source::Gog => "GOG",
            Source::Epic => "Epic Games",
            Source::RetroAchievements => "RetroAchievements",
        }
    }
}

/// RetroAchievements' achievement types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Progression,
    WinCondition,
    Missable,
}

/// Progress towards a counted achievement ("64 of 100 headshots").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub current: u32,
    pub max: u32,
}

impl Progress {
    /// `0.0..=1.0`.
    pub fn fraction(&self) -> f32 {
        if self.max == 0 {
            0.
        } else {
            (self.current.min(self.max) as f32) / self.max as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Achievement {
    /// The source's own id (Steam's API name, GOG's key, RetroAchievements'
    /// number): what a refresh matches on.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A secret achievement: its name and description are spoilers until it
    /// is unlocked.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Icon URL (or file path) of the unlocked achievement.
    #[serde(default)]
    pub icon: String,
    /// Icon of the locked achievement; empty when the source has none (the
    /// unlocked one is drawn greyed out).
    #[serde(default)]
    pub icon_locked: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub unlocked: bool,
    /// When it was unlocked, when the source says (`None` while locked, and
    /// for the few unlocks a source gives no time for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlocked_at: Option<DateTime<Utc>>,
    /// RetroAchievements: unlocked in hardcore mode.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hardcore: bool,
    /// Share of the players that have it, `0.0..=100.0`; `None` = unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity: Option<f32>,
    /// RetroAchievements points, Epic XP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    /// When this app first saw it unlocked (`None` for what came with the
    /// first read of the game: history, not news).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noticed_at: Option<DateTime<Utc>>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Achievement {
    /// The icon to draw: the locked one while locked, when there is one.
    pub fn icon_now(&self) -> &str {
        if !self.unlocked && !self.icon_locked.is_empty() {
            &self.icon_locked
        } else {
            &self.icon
        }
    }

    /// Whether the name and description must stay hidden.
    pub fn is_secret(&self) -> bool {
        self.hidden && !self.unlocked
    }
}

/// The achievements of one game from one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementSet {
    pub source: Source,
    /// The game at the source: Steam app id, GOG product id, Epic sandbox
    /// (namespace), RetroAchievements game id.
    pub external_id: String,
    /// The game's name at the source, when it differs from ours (a ROM
    /// matched on RetroAchievements).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub fetched_at: DateTime<Utc>,
    /// In the source's order.
    pub achievements: Vec<Achievement>,
}

impl AchievementSet {
    pub fn total(&self) -> usize {
        self.achievements.len()
    }

    pub fn unlocked(&self) -> usize {
        self.achievements.iter().filter(|a| a.unlocked).count()
    }

    pub fn is_complete(&self) -> bool {
        !self.achievements.is_empty() && self.unlocked() == self.total()
    }

    /// Points of the unlocked achievements and of all of them; `None` when
    /// the source has no points.
    pub fn points(&self) -> Option<(u32, u32)> {
        if self.achievements.iter().all(|a| a.points.is_none()) {
            return None;
        }
        let total = self.achievements.iter().filter_map(|a| a.points).sum();
        let got = self.achievements.iter().filter(|a| a.unlocked).filter_map(|a| a.points).sum();
        Some((got, total))
    }

    pub fn last_unlocked_at(&self) -> Option<DateTime<Utc>> {
        self.achievements.iter().filter_map(|a| a.unlocked_at).max()
    }

    /// Unlocked achievements, the latest first (those without a time last).
    pub fn recent(&self) -> Vec<&Achievement> {
        let mut v: Vec<&Achievement> = self.achievements.iter().filter(|a| a.unlocked).collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.unlocked_at));
        v
    }

    /// Locked achievements closest to being unlocked: the most advanced
    /// counted ones, then the most common. Secret ones are left out.
    pub fn up_next(&self) -> Vec<&Achievement> {
        let mut v: Vec<&Achievement> = self.achievements.iter().filter(|a| !a.unlocked && !a.hidden).collect();
        let progress = |a: &Achievement| a.progress.filter(|p| p.current > 0).map_or(-1., |p| p.fraction());
        v.sort_by(|a, b| progress(b).total_cmp(&progress(a)).then_with(|| b.rarity.unwrap_or(0.).total_cmp(&a.rarity.unwrap_or(0.))));
        v
    }

    /// Unlocked achievements rarer than this share of players count as rare.
    pub const RARE_BELOW: f32 = 15.;

    pub fn rare_unlocked(&self) -> usize {
        self.achievements.iter().filter(|a| a.unlocked && a.rarity.is_some_and(|r| r < Self::RARE_BELOW)).count()
    }
}

/// Everything kept about one game: one set per source it has achievements
/// on (a game owned on GOG and on Steam has two).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAchievements {
    #[serde(default)]
    pub sets: Vec<AchievementSet>,
    /// The RetroAchievements game picked by hand for a ROM whose hash
    /// matches no set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retro_game_id: Option<u32>,
    /// When a source last said the game has no achievements there (so it is
    /// not asked again at every start).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub none_at: std::collections::BTreeMap<Source, DateTime<Utc>>,
}

impl GameAchievements {
    pub fn set(&self, source: Source) -> Option<&AchievementSet> {
        self.sets.iter().find(|s| s.source == source)
    }

    /// The set to show: the first, in `order`, that has achievements.
    pub fn primary(&self, order: &[Source]) -> Option<&AchievementSet> {
        order.iter().filter_map(|s| self.set(*s)).find(|s| !s.achievements.is_empty())
    }

    /// When `source` was last read with an answer (a set, or "none").
    pub fn checked_at(&self, source: Source) -> Option<DateTime<Utc>> {
        let set = self.set(source).map(|s| s.fetched_at);
        let none = self.none_at.get(&source).copied();
        set.max(none)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn a(id: &str, unlocked_day: Option<u32>, rarity: f32) -> Achievement {
        Achievement {
            id: id.into(),
            name: id.into(),
            unlocked: unlocked_day.is_some(),
            unlocked_at: unlocked_day.map(|d| Utc.with_ymd_and_hms(2026, 9, d, 20, 0, 0).unwrap()),
            rarity: Some(rarity),
            ..Default::default()
        }
    }

    fn set(items: Vec<Achievement>) -> AchievementSet {
        AchievementSet { source: Source::Steam, external_id: "1".into(), title: None, fetched_at: Utc::now(), achievements: items }
    }

    #[test]
    fn recent_puts_the_latest_first() {
        let s = set(vec![a("x", Some(3), 50.), a("y", Some(9), 2.), a("z", None, 70.)]);
        let ids: Vec<_> = s.recent().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["y", "x"]);
        assert_eq!(s.unlocked(), 2);
        assert_eq!(s.rare_unlocked(), 1);
        assert!(!s.is_complete());
    }

    #[test]
    fn up_next_prefers_progress_then_the_most_common() {
        let mut counted = a("counted", None, 4.);
        counted.progress = Some(Progress { current: 18, max: 27 });
        let mut secret = a("secret", None, 90.);
        secret.hidden = true;
        let s = set(vec![a("rare", None, 3.), a("common", None, 60.), counted, secret]);
        let ids: Vec<_> = s.up_next().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["counted", "common", "rare"]);
    }

    #[test]
    fn points_only_when_the_source_has_them() {
        let mut x = a("x", Some(1), 1.);
        x.points = Some(10);
        let mut y = a("y", None, 1.);
        y.points = Some(25);
        assert_eq!(set(vec![x, y]).points(), Some((10, 35)));
        assert_eq!(set(vec![a("z", None, 1.)]).points(), None);
    }

    #[test]
    fn secrets_and_icons() {
        let mut x = a("x", None, 1.);
        x.hidden = true;
        x.icon = "on.png".into();
        x.icon_locked = "off.png".into();
        assert!(x.is_secret());
        assert_eq!(x.icon_now(), "off.png");
        x.unlocked = true;
        assert!(!x.is_secret());
        assert_eq!(x.icon_now(), "on.png");
    }

    #[test]
    fn json_keys_are_camel_case_and_lean() {
        let v = serde_json::to_value(a("x", None, 5.)).unwrap();
        assert_eq!(v, serde_json::json!({"id":"x","name":"x","description":"","icon":"","iconLocked":"","rarity":5.0}));
        let mut g = GameAchievements { sets: vec![set(vec![])], ..Default::default() };
        g.none_at.insert(Source::Epic, Utc::now());
        let back: GameAchievements = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
        assert_eq!(serde_json::to_value(Source::RetroAchievements).unwrap(), "retroachievements");
    }
}
