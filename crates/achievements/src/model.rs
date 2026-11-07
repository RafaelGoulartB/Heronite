//! The achievements of one game as every source is turned into: what the
//! focus panel, the Timeline and the "All achievements" dialog draw, and
//! what the local copy (`store`) keeps.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

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
    /// The [`AchievementGroup::id`] it belongs to; `None` when the source
    /// gives no groups (or none for it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Fields written by a newer Heronite, kept through refreshes.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
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

/// A collection the game's developer put some of its achievements in:
/// Steam's Achievement Sets (base game or DLC). Not to be confused with
/// [`AchievementSet`], which is the whole game at one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AchievementGroup {
    /// The source's id of the group (Steam's group id): stable across
    /// renames and languages, what membership and refreshes match on.
    pub id: String,
    /// In the interface language; empty when the source names it not
    /// (Steam's default group).
    #[serde(default)]
    pub name: String,
    /// The source's order key; ties keep the source's order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    /// Steam: the DLC the group's achievements need, `Some(0)` for the base
    /// game, `None` when the source does not say. Whether the player owns
    /// the DLC is never known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dlc_app_id: Option<u32>,
    /// Retired by the developer (seasonal achievements).
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
    /// Fields written by a newer Heronite, kept through refreshes.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl AchievementGroup {
    /// A DLC group, as far as the source says.
    pub fn is_dlc(&self) -> bool {
        self.dlc_app_id.is_some_and(|id| id != 0)
    }
}

/// The achievements of a group (or of none, `group: None`) and its counts:
/// unlocked out of listed, not Steam's own "set complete" status.
#[derive(Debug, Clone, PartialEq)]
pub struct Section<'a> {
    pub group: Option<&'a AchievementGroup>,
    pub achievements: Vec<&'a Achievement>,
}

impl Section<'_> {
    pub fn total(&self) -> usize {
        self.achievements.len()
    }

    pub fn unlocked(&self) -> usize {
        self.achievements.iter().filter(|a| a.unlocked).count()
    }

    pub fn group_id(&self) -> Option<&str> {
        self.group.map(|g| g.id.as_str())
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
    /// The developer's groups, in display order; empty when the source has
    /// none or gives none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<AchievementGroup>,
    /// Fields written by a newer Heronite, kept through refreshes.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl AchievementSet {
    /// The known group the achievement is in.
    pub fn group_of(&self, a: &Achievement) -> Option<&AchievementGroup> {
        let id = a.group.as_deref()?;
        self.groups.iter().find(|g| g.id == id)
    }

    /// The achievements by group, in the groups' order, the ones in no
    /// known group last; empty (a flat list) unless at least two known
    /// groups have achievements. The ones in no group never make a split
    /// on their own: a game without sets keeps Steam's single unnamed group
    /// from the Community page, and a later read without groups (the
    /// client's schema) leaves its new achievements in none.
    pub fn sections(&self) -> Vec<Section<'_>> {
        let mut sections: Vec<Section> = self.groups.iter().map(|g| Section { group: Some(g), achievements: Vec::new() }).collect();
        let mut other = Section { group: None, achievements: Vec::new() };
        for a in &self.achievements {
            match self.group_of(a).and_then(|g| sections.iter_mut().find(|s| s.group_id() == Some(g.id.as_str()))) {
                Some(section) => section.achievements.push(a),
                None => other.achievements.push(a),
            }
        }
        sections.retain(|s| !s.achievements.is_empty());
        if sections.len() < 2 {
            return Vec::new();
        }
        if !other.achievements.is_empty() {
            sections.push(other);
        }
        sections
    }

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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameAchievements {
    pub sets: Vec<AchievementSet>,
    /// The RetroAchievements game picked by hand for a ROM whose hash
    /// matches no set.
    pub retro_game_id: Option<u32>,
    /// When a source last said the game has no achievements there (so it is
    /// not asked again at every start).
    pub empty_at: BTreeMap<Source, DateTime<Utc>>,
    /// What a newer Heronite wrote that this one does not know.
    pub unknown: Unknown,
}

/// Parts of a game's copy written by a newer Heronite (a set from a source
/// this version does not know, a new field). They are kept as they are and
/// written back, so going back to an older version loses nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Unknown {
    sets: Vec<Value>,
    empty_at: Map<String, Value>,
    fields: Map<String, Value>,
}

impl Unknown {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty() && self.empty_at.is_empty() && self.fields.is_empty()
    }
}

/// A source this version knows, from its stored name.
fn known_source(name: &Value) -> Option<Source> {
    Source::deserialize(name).ok()
}

/// The file as stored, before the sources are told apart.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredGame {
    #[serde(default)]
    sets: Vec<Value>,
    #[serde(default)]
    retro_game_id: Option<u32>,
    #[serde(default)]
    empty_at: Map<String, Value>,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

impl<'de> Deserialize<'de> for GameAchievements {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredGame::deserialize(deserializer)?;
        let mut game = GameAchievements { retro_game_id: stored.retro_game_id, ..Default::default() };
        game.unknown.fields = stored.fields;
        for set in stored.sets {
            if set.get("source").and_then(known_source).is_some() {
                game.sets.push(AchievementSet::deserialize(set).map_err(de::Error::custom)?);
            } else {
                game.unknown.sets.push(set);
            }
        }
        for (name, at) in stored.empty_at {
            match known_source(&Value::String(name.clone())) {
                Some(source) => {
                    game.empty_at.insert(source, DateTime::deserialize(at).map_err(de::Error::custom)?);
                }
                None => {
                    game.unknown.empty_at.insert(name, at);
                }
            }
        }
        Ok(game)
    }
}

impl Serialize for GameAchievements {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(untagged)]
        enum Either<'a, K, U> {
            Known(&'a K),
            Unknown(&'a U),
        }

        struct Sets<'a>(&'a GameAchievements);
        impl Serialize for Sets<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let g = self.0;
                serializer.collect_seq(g.sets.iter().map(Either::<_, Value>::Known).chain(g.unknown.sets.iter().map(Either::Unknown)))
            }
        }

        struct EmptyAt<'a>(&'a GameAchievements);
        impl Serialize for EmptyAt<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let g = self.0;
                let known = g.empty_at.iter().map(|(k, v)| (Either::<_, String>::Known(k), Either::<_, Value>::Known(v)));
                let unknown = g.unknown.empty_at.iter().map(|(k, v)| (Either::Unknown(k), Either::Unknown(v)));
                serializer.collect_map(known.chain(unknown))
            }
        }

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Out<'a> {
            sets: Sets<'a>,
            #[serde(skip_serializing_if = "Option::is_none")]
            retro_game_id: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            empty_at: Option<EmptyAt<'a>>,
            #[serde(flatten)]
            fields: &'a Map<String, Value>,
        }

        let empty = self.empty_at.is_empty() && self.unknown.empty_at.is_empty();
        Out {
            sets: Sets(self),
            retro_game_id: self.retro_game_id,
            empty_at: (!empty).then_some(EmptyAt(self)),
            fields: &self.unknown.fields,
        }
        .serialize(serializer)
    }
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
        let none = self.empty_at.get(&source).copied();
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
        AchievementSet {
            source: Source::Steam,
            external_id: "1".into(),
            title: None,
            fetched_at: Utc::now(),
            achievements: items,
            groups: Vec::new(),
            unknown: Map::new(),
        }
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
        g.empty_at.insert(Source::Epic, Utc::now());
        let back: GameAchievements = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
        assert_eq!(serde_json::to_value(Source::RetroAchievements).unwrap(), "retroachievements");
    }
    fn grouped(id: &str, unlocked_day: Option<u32>, group: Option<&str>) -> Achievement {
        Achievement { group: group.map(str::to_string), ..a(id, unlocked_day, 10.) }
    }

    fn g(id: &str, name: &str, dlc: Option<u32>) -> AchievementGroup {
        AchievementGroup { id: id.into(), name: name.into(), dlc_app_id: dlc, ..Default::default() }
    }

    fn view<'a>(sections: &'a [Section<'a>]) -> Vec<(Option<&'a str>, Vec<&'a str>, usize, usize)> {
        sections.iter().map(|s| (s.group_id(), s.achievements.iter().map(|a| a.id.as_str()).collect(), s.unlocked(), s.total())).collect()
    }

    #[test]
    fn sections_follow_the_groups_with_the_ungrouped_last() {
        let mut s = set(vec![
            grouped("dlc1", Some(2), Some("1")),
            grouped("base1", Some(1), Some("0")),
            grouped("base2", None, Some("0")),
            grouped("dlc2", None, Some("1")),
            // In a group the source no longer lists, or in none.
            grouped("old", Some(3), Some("9")),
            grouped("loose", None, None),
        ]);
        s.groups = vec![g("0", "", Some(0)), g("1", "The Pale Reach", Some(2561440)), g("5", "Empty", None)];
        let sections = s.sections();
        assert_eq!(
            view(&sections),
            [(Some("0"), vec!["base1", "base2"], 1, 2), (Some("1"), vec!["dlc1", "dlc2"], 1, 2), (None, vec!["old", "loose"], 1, 2),]
        );
        // Per-group counts add up to the game's.
        assert_eq!(sections.iter().map(Section::total).sum::<usize>(), s.total());
        assert_eq!(sections.iter().map(Section::unlocked).sum::<usize>(), s.unlocked());
        assert!(sections[1].group.unwrap().is_dlc() && !sections[0].group.unwrap().is_dlc());
        assert_eq!(s.group_of(&s.achievements[4]), None);
    }

    #[test]
    fn one_group_or_none_is_a_flat_list() {
        let mut s = set(vec![grouped("a", None, Some("0")), grouped("b", Some(1), Some("0"))]);
        assert!(s.sections().is_empty());
        s.groups = vec![g("0", "", Some(0)), g("1", "Unused", None)];
        assert!(s.sections().is_empty());
    }

    #[test]
    fn one_group_plus_ungrouped_ones_is_still_a_flat_list() {
        // Portal (no sets) read from the Community page, then from a client
        // schema without groups that lists one more achievement.
        let mut s = set(vec![grouped("x", Some(1), Some("0")), grouped("y", None, Some("0")), grouped("new", None, None)]);
        s.groups = vec![g("0", "", Some(0))];
        assert!(s.sections().is_empty());
        // Ungrouped ones alone are a flat list too.
        s.achievements.iter_mut().for_each(|a| a.group = None);
        assert!(s.sections().is_empty());
    }

    #[test]
    fn unknown_dlc_is_not_dlc() {
        assert!(!g("1", "Set", None).is_dlc());
        assert!(!g("0", "", Some(0)).is_dlc());
        assert!(g("1", "Set", Some(7)).is_dlc());
    }
}
