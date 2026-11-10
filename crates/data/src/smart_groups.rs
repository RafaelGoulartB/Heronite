//! Smart groups: groups of games defined by rules, which stay current by
//! themselves (a game that starts to match joins, one that stops leaves). This
//! app's own file, `local_library/groups.json`:
//!
//! ```json
//! { "groups": [ { "id": "g19a2c4e0b1", "name": "Weekend", "colour": "#f5cd5b", "pinned": true,
//!     "rule": { "all": [ { "field": "installed", "op": "is", "value": true },
//!                        { "field": "hltb", "op": "lt", "value": 8 },
//!                        { "not": { "field": "tag", "op": "in", "value": ["Early Access"] } } ] } } ] }
//! ```
//!
//! A rule is `all` or `any` (a list of rules), `not` (one rule) or a
//! condition on a [`Field`]. Rules, fields and keys this version does not
//! know are kept as read and written back, so an older build never damages a
//! newer file; an unknown rule matches no game. A condition on a value the
//! game lacks (no rating, no size, no release year...) never matches: "not
//! rated" is asked for with `not`.

use std::collections::{BTreeSet, HashSet};

use anyhow::{bail, Result};
use chrono::NaiveDate;
use serde_json::{json, Map, Value};

use crate::categories::Categories;
use crate::collection::{runs_offline, FacetKind, InstallFilter, FACET_KINDS};
use crate::hltb::WikiCache;
use crate::json_store;
use crate::library::CollectionGame;
use crate::organize::{release_year, Organize};
use crate::paths::DataPaths;
use crate::status::resolve_status_id;
use crate::text::facet_key;
use crate::ui_state::ViewFilters;

/// What a condition looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Installed,
    Favourite,
    /// [`runs_offline`].
    RunsOffline,
    /// Played at least once.
    Played,
    /// An installed game with an update waiting ([`Facts::updates`]).
    HasUpdate,
    /// In the hidden list (seen only while hidden games are shown).
    Hidden,
    /// Notes written in its details.
    HasNotes,
    /// A game of another store also owned on Steam.
    AlsoOnSteam,
    /// Shared by another member of the Steam Family.
    FamilyShared,
    /// Completion status ids.
    Status,
    /// Custom category names ([`crate::categories`]).
    Category,
    /// The system an emulated game runs on ([`crate::organize::GameExtras::systems`]).
    System,
    Facet(FacetKind),
    /// Hours played.
    Playtime,
    /// Days since the game was last played.
    LastPlayed,
    /// Days since the game was first played.
    FirstPlayed,
    /// Sessions recorded.
    TimesPlayed,
    /// Average session, in minutes.
    SessionLength,
    /// Hours played in the last 30 days.
    RecentPlaytime,
    /// Time played against HowLongToBeat's main story, in percent.
    Progress,
    /// Days since the game entered the library ([`crate::added`]).
    Added,
    ReleaseYear,
    /// The user's stars, `1..=5`.
    Rating,
    /// As the metadata gives it (IGDB's 0 to 100).
    CriticScore,
    /// GiB on disk, installed games only.
    InstallSize,
    /// The youngest age its rating is for ([`CollectionGame::age_rating`]).
    AgeRating,
    /// HowLongToBeat's main story, in hours.
    Hltb,
}

/// The kind of value a field compares with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// `is` true or false.
    Flag,
    /// `in` a list of names (any of them).
    Names,
    /// `lt`, `gt` or `eq` a number.
    Number,
}

impl Field {
    /// Every field, in the order the editor lists them.
    pub fn all() -> Vec<Field> {
        let mut out = vec![
            Field::Installed,
            Field::Favourite,
            Field::Hidden,
            Field::Status,
            Field::Category,
            Field::HasNotes,
            Field::HasUpdate,
            Field::RunsOffline,
            Field::AlsoOnSteam,
            Field::FamilyShared,
            Field::Played,
            Field::Playtime,
            Field::RecentPlaytime,
            Field::TimesPlayed,
            Field::SessionLength,
            Field::LastPlayed,
            Field::FirstPlayed,
            Field::Progress,
            Field::Hltb,
            Field::Added,
            Field::ReleaseYear,
            Field::Rating,
            Field::CriticScore,
            Field::AgeRating,
            Field::InstallSize,
            Field::System,
        ];
        out.extend(FACET_KINDS.map(Field::Facet));
        out
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Field::Installed => "installed",
            Field::Favourite => "favourite",
            Field::RunsOffline => "runsOffline",
            Field::Played => "played",
            Field::HasUpdate => "hasUpdate",
            Field::Hidden => "hidden",
            Field::HasNotes => "hasNotes",
            Field::AlsoOnSteam => "alsoOnSteam",
            Field::FamilyShared => "familyShared",
            Field::System => "system",
            Field::FirstPlayed => "firstPlayed",
            Field::TimesPlayed => "timesPlayed",
            Field::SessionLength => "sessionLength",
            Field::RecentPlaytime => "recentPlaytime",
            Field::Progress => "progress",
            Field::AgeRating => "ageRating",
            Field::Status => "status",
            Field::Category => "category",
            Field::Facet(kind) => kind.as_str(),
            Field::Playtime => "playtime",
            Field::LastPlayed => "lastPlayed",
            Field::Added => "dateAdded",
            Field::ReleaseYear => "releaseYear",
            Field::Rating => "rating",
            Field::CriticScore => "criticScore",
            Field::InstallSize => "installSize",
            Field::Hltb => "hltb",
        }
    }

    pub fn parse(value: &str) -> Option<Field> {
        Field::all().into_iter().find(|f| f.as_str() == value)
    }

    pub fn kind(self) -> ValueKind {
        match self {
            Field::Installed
            | Field::Favourite
            | Field::RunsOffline
            | Field::Played
            | Field::HasUpdate
            | Field::Hidden
            | Field::HasNotes
            | Field::AlsoOnSteam
            | Field::FamilyShared => ValueKind::Flag,
            Field::Status | Field::Category | Field::System | Field::Facet(_) => ValueKind::Names,
            _ => ValueKind::Number,
        }
    }
}

/// How a condition compares.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Is(bool),
    /// Any of the names.
    In(Vec<String>),
    Lt(f64),
    Gt(f64),
    Eq(f64),
}

impl Op {
    fn kind(&self) -> ValueKind {
        match self {
            Op::Is(_) => ValueKind::Flag,
            Op::In(_) => ValueKind::Names,
            _ => ValueKind::Number,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cond {
    pub field: Field,
    pub op: Op,
    /// Keys this version does not read, written back unchanged.
    pub extra: Map<String, Value>,
}

impl Cond {
    pub fn new(field: Field, op: Op) -> Self {
        Self { field, op, extra: Map::new() }
    }

    /// A condition with the field's first value: true, no names, or 0.
    pub fn empty(field: Field) -> Self {
        let op = match field.kind() {
            ValueKind::Flag => Op::Is(true),
            ValueKind::Names => Op::In(Vec::new()),
            ValueKind::Number => Op::Lt(0.),
        };
        Self::new(field, op)
    }

    fn from_value(value: &Value) -> Option<Self> {
        let field = Field::parse(value.get("field")?.as_str()?)?;
        let v = value.get("value");
        let op = match value.get("op")?.as_str()? {
            "is" => Op::Is(v?.as_bool()?),
            "in" => Op::In(v?.as_array()?.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()),
            "lt" => Op::Lt(v?.as_f64()?),
            "gt" => Op::Gt(v?.as_f64()?),
            "eq" => Op::Eq(v?.as_f64()?),
            _ => return None,
        };
        let extra = value.as_object()?.iter().filter(|(k, _)| !matches!(k.as_str(), "field" | "op" | "value"));
        let extra = extra.map(|(k, v)| (k.clone(), v.clone())).collect();
        (op.kind() == field.kind()).then_some(Self { field, op, extra })
    }

    fn to_value(&self) -> Value {
        let (op, value) = match &self.op {
            Op::Is(b) => ("is", json!(b)),
            Op::In(names) => ("in", json!(names)),
            Op::Lt(n) => ("lt", json!(n)),
            Op::Gt(n) => ("gt", json!(n)),
            Op::Eq(n) => ("eq", json!(n)),
        };
        let mut obj = self.extra.clone();
        obj.insert("field".into(), json!(self.field.as_str()));
        obj.insert("op".into(), json!(op));
        obj.insert("value".into(), value);
        Value::Object(obj)
    }

    fn matches(&self, g: &CollectionGame, facts: &Facts) -> bool {
        match &self.op {
            Op::Is(want) => flag(self.field, g, facts) == *want,
            Op::In(names) => names_match(self.field, names, g, facts),
            Op::Lt(n) => number(self.field, g, facts).is_some_and(|v| v < *n),
            Op::Gt(n) => number(self.field, g, facts).is_some_and(|v| v > *n),
            Op::Eq(n) => number(self.field, g, facts).is_some_and(|v| (v - n).abs() < 1e-9),
        }
    }
}

/// What the rules read besides the games themselves.
#[derive(Clone, Copy)]
pub struct Facts<'a> {
    /// Ratings, sizes, dates added, categories and statuses.
    pub organize: Organize<'a>,
    /// HowLongToBeat times.
    pub wiki: Option<&'a WikiCache>,
    /// `app_name` of the installed games with an update waiting.
    pub updates: Option<&'a HashSet<String>>,
    /// The day "days ago" counts from.
    pub today: NaiveDate,
}

fn flag(field: Field, g: &CollectionGame, facts: &Facts) -> bool {
    match field {
        Field::Installed => g.installed,
        Field::Favourite => g.favourite,
        Field::RunsOffline => runs_offline(g),
        Field::Played => g.playtime_minutes > 0 || g.last_played.as_deref().is_some_and(|s| !s.is_empty()),
        Field::HasUpdate => facts.updates.is_some_and(|u| u.contains(&g.app_name)),
        Field::Hidden => g.hidden,
        Field::HasNotes => g.details.notes.as_deref().is_some_and(|n| !n.trim().is_empty()),
        Field::AlsoOnSteam => g.steam_link.is_some(),
        Field::FamilyShared => g.family.is_some(),
        _ => false,
    }
}

fn names_match(field: Field, names: &[String], g: &CollectionGame, facts: &Facts) -> bool {
    match field {
        Field::Status => {
            let id = resolve_status_id(g.status_raw.as_deref(), facts.organize.statuses);
            names.contains(&id)
        }
        Field::Category => {
            let Some(cats) = facts.organize.categories else { return false };
            let selected: BTreeSet<String> = names.iter().cloned().collect();
            !selected.is_empty() && cats.matches(&selected, &g.runner, &g.app_name)
        }
        Field::Facet(kind) => names.iter().any(|n| {
            let wanted = facet_key(n);
            g.facets[kind.index()].iter().any(|item| item.key == wanted)
        }),
        Field::System => {
            let Some(system) = facts.organize.extras.and_then(|e| e.systems.get(&g.key)).map(|s| facet_key(s)) else { return false };
            names.iter().any(|n| facet_key(n) == system)
        }
        _ => false,
    }
}

/// Days from `text` (an RFC 3339 instant, or a `YYYY-MM-DD` date) to the
/// local `today`.
fn days_ago(text: &str, today: NaiveDate) -> Option<f64> {
    Some((today - local_day(text, &chrono::Local)?).num_days() as f64)
}

/// The day of `text` in `tz`: an instant's own day there, or a plain date.
fn local_day<Tz: chrono::TimeZone>(text: &str, tz: &Tz) -> Option<NaiveDate> {
    let text = text.trim();
    match chrono::DateTime::parse_from_rfc3339(text) {
        Ok(at) => Some(at.with_timezone(tz).date_naive()),
        Err(_) => NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok(),
    }
}

fn number(field: Field, g: &CollectionGame, facts: &Facts) -> Option<f64> {
    let o = &facts.organize;
    match field {
        Field::Playtime => Some(g.playtime_minutes as f64 / 60.),
        Field::LastPlayed => g.last_played.as_deref().and_then(|s| days_ago(s, facts.today)),
        Field::Added => o.added(g).and_then(|s| days_ago(s, facts.today)),
        Field::ReleaseYear => release_year(g).map(f64::from),
        Field::Rating => o.rating(g).map(f64::from),
        Field::CriticScore => g.details.critic_score.filter(|s| *s > 0.),
        Field::InstallSize => o.size(g).filter(|_| g.installed).map(|b| b as f64 / (1024. * 1024. * 1024.)),
        Field::Hltb => facts.wiki?.main_story(&g.title, &g.game.title),
        Field::FirstPlayed => g.first_played.as_deref().and_then(|s| days_ago(s, facts.today)),
        Field::TimesPlayed => Some(crate::organize::times_played(g) as f64),
        Field::SessionLength => {
            let sessions = g.sessions.as_deref().filter(|s| !s.is_empty())?;
            Some(sessions.iter().map(|s| s.elapsed_seconds.get()).sum::<f64>() / sessions.len() as f64 / 60.)
        }
        Field::RecentPlaytime => {
            let seconds = g.sessions.as_deref().map_or(0., |list| {
                list.iter().filter(|s| days_ago(&s.started_at, facts.today).is_some_and(|d| d < 30.)).map(|s| s.elapsed_seconds.get()).sum()
            });
            Some(seconds / 3600.)
        }
        Field::Progress => {
            let main = facts.wiki?.main_story(&g.title, &g.game.title)?;
            Some(g.playtime_minutes as f64 / 60. / main * 100.)
        }
        Field::AgeRating => g.age_rating.as_ref().and_then(|r| r.min_age()).map(f64::from),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// Every rule matches (an empty list matches every game).
    All(Vec<Rule>),
    /// One of the rules matches (an empty list, no condition at all,
    /// matches every game, as an empty `All`).
    Any(Vec<Rule>),
    Not(Box<Rule>),
    Cond(Cond),
    /// A rule of a newer version, kept as read. It never decides a match:
    /// a group needing its answer (`not` around it, or in an `all`) shows no
    /// game rather than a guess.
    Unknown(Value),
}

impl Default for Rule {
    fn default() -> Self {
        Rule::All(Vec::new())
    }
}

impl Rule {
    pub fn from_value(value: &Value) -> Rule {
        let list = |v: &Value| v.as_array().map(|a| a.iter().map(Rule::from_value).collect::<Vec<_>>());
        let Some(obj) = value.as_object() else { return Rule::Unknown(value.clone()) };
        let parsed = if obj.contains_key("field") {
            Cond::from_value(value).map(Rule::Cond)
        } else if let (1, Some(v)) = (obj.len(), obj.get("all")) {
            list(v).map(Rule::All)
        } else if let (1, Some(v)) = (obj.len(), obj.get("any")) {
            list(v).map(Rule::Any)
        } else if let (1, Some(v)) = (obj.len(), obj.get("not")) {
            Some(Rule::Not(Box::new(Rule::from_value(v))))
        } else {
            None
        };
        parsed.unwrap_or_else(|| Rule::Unknown(value.clone()))
    }

    pub fn to_value(&self) -> Value {
        match self {
            Rule::All(list) => json!({ "all": list.iter().map(Rule::to_value).collect::<Vec<_>>() }),
            Rule::Any(list) => json!({ "any": list.iter().map(Rule::to_value).collect::<Vec<_>>() }),
            Rule::Not(rule) => json!({ "not": rule.to_value() }),
            Rule::Cond(c) => c.to_value(),
            Rule::Unknown(v) => v.clone(),
        }
    }

    pub fn matches(&self, g: &CollectionGame, facts: &Facts) -> bool {
        self.eval(g, facts) == Some(true)
    }

    /// Three-valued: `None` where a rule this version can't read decides.
    fn eval(&self, g: &CollectionGame, facts: &Facts) -> Option<bool> {
        match self {
            Rule::All(list) => {
                let mut known = true;
                for r in list {
                    match r.eval(g, facts) {
                        Some(false) => return Some(false),
                        None => known = false,
                        Some(true) => {}
                    }
                }
                known.then_some(true)
            }
            // No conditions limit nothing: every game, as with an empty `all`.
            Rule::Any(list) if list.is_empty() => Some(true),
            Rule::Any(list) => {
                let mut known = true;
                for r in list {
                    match r.eval(g, facts) {
                        Some(true) => return Some(true),
                        None => known = false,
                        Some(false) => {}
                    }
                }
                known.then_some(false)
            }
            Rule::Not(rule) => rule.eval(g, facts).map(|b| !b),
            Rule::Cond(c) => Some(c.matches(g, facts)),
            Rule::Unknown(_) => None,
        }
    }

    /// The games of the View menu's filters `v` (install, "Only" switches,
    /// categories, statuses and values of each kind; the ages are left out).
    pub fn from_view(v: &ViewFilters) -> Rule {
        let mut all = Vec::new();
        match v.install_filter {
            InstallFilter::All => {}
            f => all.push(Cond::new(Field::Installed, Op::Is(f == InstallFilter::Installed))),
        }
        for (on, field) in [(v.only.favourites, Field::Favourite), (v.only.updates, Field::HasUpdate), (v.only.offline, Field::RunsOffline)]
        {
            if on {
                all.push(Cond::new(field, Op::Is(true)));
            }
        }
        if !v.statuses.is_empty() {
            all.push(Cond::new(Field::Status, Op::In(v.statuses.iter().cloned().collect())));
        }
        if !v.categories.is_empty() {
            all.push(Cond::new(Field::Category, Op::In(v.categories.iter().cloned().collect())));
        }
        for kind in FACET_KINDS {
            let values = v.facet_filters.get(kind);
            if !values.is_empty() {
                all.push(Cond::new(Field::Facet(kind), Op::In(values.to_vec())));
            }
        }
        Rule::All(all.into_iter().map(Rule::Cond).collect())
    }

    /// Whether the rule uses `field` anywhere.
    pub fn uses(&self, field: Field) -> bool {
        match self {
            Rule::All(list) | Rule::Any(list) => list.iter().any(|r| r.uses(field)),
            Rule::Not(rule) => rule.uses(field),
            Rule::Cond(c) => c.field == field,
            Rule::Unknown(_) => false,
        }
    }
}

/// The shape the editor shows: conditions that must all (or any) match,
/// minus the games that match any of `except`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Simple {
    pub any: bool,
    pub conds: Vec<Cond>,
    pub except: Vec<Cond>,
}

impl Simple {
    /// `rule` in that shape; `None` for one nested deeper (or holding rules
    /// this version does not know), which the editor keeps as it is.
    pub fn of(rule: &Rule) -> Option<Simple> {
        let conds =
            |list: &[Rule]| list.iter().map(|r| if let Rule::Cond(c) = r { Some(c.clone()) } else { None }).collect::<Option<Vec<_>>>();
        let except = |rule: &Rule| match rule {
            Rule::Cond(c) => Some(vec![c.clone()]),
            Rule::Any(list) => conds(list),
            _ => None,
        };
        match rule {
            Rule::Cond(c) => Some(Simple { any: false, conds: vec![c.clone()], except: Vec::new() }),
            Rule::Any(list) => Some(Simple { any: true, conds: conds(list)?, except: Vec::new() }),
            Rule::All(list) => {
                let (nots, rest): (Vec<&Rule>, Vec<&Rule>) = list.iter().partition(|r| matches!(r, Rule::Not(_)));
                let except = match nots.as_slice() {
                    [] => Vec::new(),
                    [Rule::Not(inner)] => except(inner)?,
                    _ => return None,
                };
                match rest.as_slice() {
                    [Rule::Any(list)] if !except.is_empty() => Some(Simple { any: true, conds: conds(list)?, except }),
                    _ => {
                        let rest: Vec<Rule> = rest.into_iter().cloned().collect();
                        Some(Simple { any: false, conds: conds(&rest)?, except })
                    }
                }
            }
            Rule::Not(_) | Rule::Unknown(_) => None,
        }
    }

    pub fn to_rule(&self) -> Rule {
        let conds: Vec<Rule> = self.conds.iter().cloned().map(Rule::Cond).collect();
        let except = match self.except.as_slice() {
            [] => None,
            [one] => Some(Rule::Not(Box::new(Rule::Cond(one.clone())))),
            many => Some(Rule::Not(Box::new(Rule::Any(many.iter().cloned().map(Rule::Cond).collect())))),
        };
        match (self.any, except) {
            (false, None) => Rule::All(conds),
            (true, None) => Rule::Any(conds),
            (false, Some(not)) => Rule::All(conds.into_iter().chain([not]).collect()),
            (true, Some(not)) => Rule::All(vec![Rule::Any(conds), not]),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SmartGroup {
    pub id: String,
    pub name: String,
    /// `#rrggbb`; `None` for the accent colour.
    pub colour: Option<String>,
    /// Shown in the Collection's header (none is, until the user pins one).
    pub pinned: bool,
    pub rule: Rule,
    /// Keys this version does not read, written back unchanged.
    pub extra: Map<String, Value>,
}

const GROUP_KEYS: [&str; 5] = ["id", "name", "colour", "pinned", "rule"];

impl SmartGroup {
    pub fn new(id: impl Into<String>, name: impl Into<String>, rule: Rule) -> Self {
        Self { id: id.into(), name: name.into(), colour: None, pinned: false, rule, extra: Map::new() }
    }

    pub fn matches(&self, g: &CollectionGame, facts: &Facts) -> bool {
        self.rule.matches(g, facts)
    }

    /// A group as stored; `None` without an id or a name.
    pub fn from_value(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let id = obj.get("id")?.as_str().filter(|s| !s.is_empty())?.to_owned();
        let name = obj.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?.to_owned();
        Some(Self {
            id,
            name,
            colour: obj.get("colour").and_then(Value::as_str).filter(|s| is_hex_colour(s)).map(str::to_owned),
            pinned: obj.get("pinned").and_then(Value::as_bool).unwrap_or(false),
            rule: obj.get("rule").map(Rule::from_value).unwrap_or_default(),
            extra: obj.iter().filter(|(k, _)| !GROUP_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect(),
        })
    }

    pub fn to_value(&self) -> Value {
        let mut obj = self.extra.clone();
        obj.insert("id".into(), json!(self.id));
        obj.insert("name".into(), json!(self.name));
        obj.insert("colour".into(), json!(self.colour));
        obj.insert("pinned".into(), json!(self.pinned));
        obj.insert("rule".into(), self.rule.to_value());
        Value::Object(obj)
    }
}

/// `#rrggbb`.
pub fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// The groups as loaded, in the user's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmartGroups {
    pub groups: Vec<SmartGroup>,
}

impl SmartGroups {
    /// A missing or unreadable file gives no groups.
    pub fn load(paths: &DataPaths) -> Self {
        let doc = json_store::read_value(&paths.smart_groups()).unwrap_or(Value::Null);
        let groups = doc.get("groups").and_then(Value::as_array).map(|list| list.iter().filter_map(SmartGroup::from_value).collect());
        Self { groups: groups.unwrap_or_default() }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&SmartGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn pinned(&self) -> impl Iterator<Item = &SmartGroup> {
        self.groups.iter().filter(|g| g.pinned)
    }

    /// An id no group has: the clock, then a part of its nanoseconds, so
    /// two PCs that sync never make the same one in the same millisecond.
    pub fn new_id(&self) -> String {
        let now = chrono::Utc::now();
        let mut n = now.timestamp_millis().max(0) as u64;
        let spread = now.timestamp_subsec_nanos() % 0x1000;
        loop {
            let id = format!("g{n:x}{spread:03x}");
            if self.get(&id).is_none() {
                return id;
            }
            n += 1;
        }
    }
}

/// Writes `groups` in that order. Keys of the file this version does not
/// model are kept, and so are the stored groups this version could not read
/// (a newer one's), after the others.
pub fn save(paths: &DataPaths, groups: &[SmartGroup]) -> Result<()> {
    for g in groups {
        if g.name.trim().is_empty() {
            bail!("{}", heronite_i18n::tr!("The category needs a name"));
        }
    }
    json_store::update_value(&paths.smart_groups(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        let obj = doc.as_object_mut().expect("object");
        let unread: Vec<Value> = obj
            .get("groups")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter(|v| SmartGroup::from_value(v).is_none()).cloned().collect())
            .unwrap_or_default();
        let list: Vec<Value> = groups.iter().map(SmartGroup::to_value).chain(unread).collect();
        obj.insert("groups".into(), Value::Array(list));
        Ok(())
    })
}

/// Stores `group` in place of the one with its id, or after the others.
pub fn upsert(paths: &DataPaths, group: SmartGroup) -> Result<()> {
    let mut all = SmartGroups::load(paths).groups;
    match all.iter_mut().find(|g| g.id == group.id) {
        Some(slot) => *slot = group,
        None => all.push(group),
    }
    save(paths, &all)
}

pub fn remove(paths: &DataPaths, id: &str) -> Result<()> {
    let mut all = SmartGroups::load(paths).groups;
    all.retain(|g| g.id != id);
    save(paths, &all)
}

/// Pins or unpins the group `id`.
pub fn set_pinned(paths: &DataPaths, id: &str, pinned: bool) -> Result<()> {
    let mut all = SmartGroups::load(paths).groups;
    if let Some(g) = all.iter_mut().find(|g| g.id == id) {
        g.pinned = pinned;
    }
    save(paths, &all)
}

/// The groups whose rules name category `old`, with `new` (`None`: the
/// category was removed and leaves the rules). True when one changed.
pub fn rename_category(groups: &mut [SmartGroup], old: &str, new: Option<&str>) -> bool {
    fn walk(rule: &mut Rule, old: &str, new: Option<&str>) -> bool {
        match rule {
            Rule::All(list) | Rule::Any(list) => list.iter_mut().fold(false, |acc, r| walk(r, old, new) | acc),
            Rule::Not(inner) => walk(inner, old, new),
            Rule::Cond(Cond { field: Field::Category, op: Op::In(names), .. }) if names.iter().any(|n| n == old) => {
                names.retain(|n| n != old);
                if let Some(new) = new.filter(|n| !names.iter().any(|x| x == n)) {
                    names.push(new.to_owned());
                }
                true
            }
            _ => false,
        }
    }
    groups.iter_mut().fold(false, |acc, g| walk(&mut g.rule, old, new) | acc)
}

/// The names `rule`'s list conditions on `field` hold (statuses, categories,
/// values of a library field), each once.
pub fn names_used(rule: &Rule, field: Field) -> Vec<&str> {
    fn walk<'r>(rule: &'r Rule, field: Field, out: &mut Vec<&'r str>) {
        match rule {
            Rule::All(list) | Rule::Any(list) => list.iter().for_each(|r| walk(r, field, out)),
            Rule::Not(inner) => walk(inner, field, out),
            Rule::Cond(Cond { field: f, op: Op::In(names), .. }) if *f == field => {
                for n in names {
                    if !out.contains(&n.as_str()) {
                        out.push(n);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(rule, field, &mut out);
    out
}

/// Categories referenced by `rule` that `cats` no longer has.
pub fn missing_categories<'r>(rule: &'r Rule, cats: &Categories) -> Vec<&'r str> {
    let mut out = Vec::new();
    fn walk<'r>(rule: &'r Rule, cats: &Categories, out: &mut Vec<&'r str>) {
        match rule {
            Rule::All(list) | Rule::Any(list) => list.iter().for_each(|r| walk(r, cats, out)),
            Rule::Not(inner) => walk(inner, cats, out),
            Rule::Cond(Cond { field: Field::Category, op: Op::In(names), .. }) => {
                out.extend(names.iter().map(String::as_str).filter(|n| *n != crate::categories::UNCATEGORIZED && !cats.exists(n)));
            }
            _ => {}
        }
    }
    walk(rule, cats, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game;
    use crate::organize::GameExtras;
    use std::collections::HashMap;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()
    }

    fn cond(field: Field, op: Op) -> Rule {
        Rule::Cond(Cond::new(field, op))
    }

    fn with<'a>(extras: &'a GameExtras, ratings: &'a HashMap<String, u8>, statuses: &'a [crate::models::CompletionStatus]) -> Facts<'a> {
        Facts {
            organize: Organize { extras: Some(extras), ratings: Some(ratings), categories: None, statuses },
            wiki: None,
            updates: None,
            today: today(),
        }
    }

    #[test]
    fn flags_names_and_numbers_match() {
        let statuses = crate::status::default_statuses();
        let (extras, ratings) = (GameExtras::default(), HashMap::new());
        let facts = with(&extras, &ratings, &statuses);
        let coop = game("a", "Co-op game", |s| {
            s.installed = true;
            s.minutes = 90;
            s.game_modes = vec!["Co-operative".into()];
            s.status = Some("playing".into());
            s.last_played = Some("2026-10-29T20:00:00.000Z".into());
        });
        let solo = game("b", "Solo game", |s| s.genres = vec!["RPG".into()]);
        let rule = Rule::All(vec![
            cond(Field::Installed, Op::Is(true)),
            cond(Field::Facet(FacetKind::GameMode), Op::In(vec!["co-operative".into(), "Split screen".into()])),
            cond(Field::Playtime, Op::Lt(2.)),
            cond(Field::LastPlayed, Op::Lt(14.)),
            cond(Field::Status, Op::In(vec!["playing".into()])),
        ]);
        assert!(rule.matches(&coop, &facts));
        assert!(!rule.matches(&solo, &facts));
        assert!(cond(Field::Played, Op::Is(false)).matches(&solo, &facts));
        assert!(cond(Field::Facet(FacetKind::Genre), Op::In(vec!["rpg".into()])).matches(&solo, &facts));
        // Nine days ago: not "more than 10 days ago".
        assert!(!cond(Field::LastPlayed, Op::Gt(10.)).matches(&coop, &facts));
    }

    #[test]
    fn a_missing_value_never_matches_and_not_finds_it() {
        let statuses = crate::status::default_statuses();
        let extras = GameExtras::default();
        let rated = game("rated", "Rated", |_| {});
        let ratings = HashMap::from([(rated.key.clone(), 4u8)]);
        let facts = with(&extras, &ratings, &statuses);
        let unrated = game("unrated", "Unrated", |_| {});
        let low = cond(Field::Rating, Op::Lt(3.));
        assert!(!low.matches(&unrated, &facts));
        assert!(!low.matches(&rated, &facts));
        let not_rated = Rule::Not(Box::new(cond(Field::Rating, Op::Gt(0.))));
        assert!(not_rated.matches(&unrated, &facts));
        assert!(!not_rated.matches(&rated, &facts));
        assert!(!cond(Field::InstallSize, Op::Gt(0.)).matches(&rated, &facts));
        assert!(!cond(Field::ReleaseYear, Op::Lt(3000.)).matches(&rated, &facts));
    }

    #[test]
    fn sizes_and_dates_come_from_the_extras() {
        let statuses = crate::status::default_statuses();
        let big = game("big", "Big", |s| s.installed = true);
        let mut extras = GameExtras::default();
        extras.sizes.insert(big.key.clone(), 60 * 1024 * 1024 * 1024);
        extras.added.insert(big.key.clone(), "2026-01-01".into());
        let ratings = HashMap::new();
        let facts = with(&extras, &ratings, &statuses);
        assert!(cond(Field::InstallSize, Op::Gt(50.)).matches(&big, &facts));
        assert!(cond(Field::Added, Op::Gt(300.)).matches(&big, &facts));
    }

    #[test]
    fn how_a_game_is_played_comes_from_its_sessions() {
        let statuses = crate::status::default_statuses();
        let mut g = std::sync::Arc::try_unwrap(game("p", "Played", |s| {
            s.minutes = 300;
            s.first_played = Some("2025-11-08T10:00:00.000Z".into());
        }))
        .unwrap();
        let session = |at: &str, minutes: u64| -> crate::models::LocalGameSession {
            serde_json::from_value(json!({ "startedAt": at, "endedAt": at, "elapsedSeconds": minutes * 60, "source": "heroic" })).unwrap()
        };
        g.sessions = Some(std::sync::Arc::new(vec![session("2026-11-01T20:00:00.000Z", 90), session("2026-06-01T20:00:00.000Z", 30)]));
        g.details.notes = Some("Boss rush next".into());
        let mut extras = GameExtras::default();
        extras.systems.insert(g.key.clone(), "PlayStation 2".into());
        let ratings = HashMap::new();
        let facts = with(&extras, &ratings, &statuses);
        assert!(cond(Field::TimesPlayed, Op::Eq(2.)).matches(&g, &facts));
        assert!(cond(Field::SessionLength, Op::Eq(60.)).matches(&g, &facts));
        assert!(cond(Field::RecentPlaytime, Op::Eq(1.5)).matches(&g, &facts), "only the session of the last 30 days");
        assert!(cond(Field::FirstPlayed, Op::Gt(360.)).matches(&g, &facts));
        assert!(cond(Field::HasNotes, Op::Is(true)).matches(&g, &facts));
        assert!(cond(Field::System, Op::In(vec!["playstation 2".into()])).matches(&g, &facts));
        // Never played: no average session, no time in the last 30 days.
        let new = game("n", "New", |_| {});
        assert!(!cond(Field::SessionLength, Op::Gt(0.)).matches(&new, &facts));
        assert!(cond(Field::RecentPlaytime, Op::Eq(0.)).matches(&new, &facts));
        assert!(!cond(Field::System, Op::In(vec!["PlayStation 2".into()])).matches(&new, &facts));
    }

    #[test]
    fn empty_lists_and_unknown_rules() {
        let statuses = crate::status::default_statuses();
        let (extras, ratings) = (GameExtras::default(), HashMap::new());
        let facts = with(&extras, &ratings, &statuses);
        let g = game("g", "G", |_| {});
        assert!(Rule::All(vec![]).matches(&g, &facts));
        // No condition limits nothing, with "any" as with "all".
        assert!(Rule::Any(vec![]).matches(&g, &facts));
        let future = json!({ "field": "deckVerified", "op": "is", "value": true });
        let rule = Rule::from_value(&json!({ "all": [future.clone()] }));
        assert_eq!(rule, Rule::All(vec![Rule::Unknown(future.clone())]));
        assert!(!rule.matches(&g, &facts));
        assert_eq!(rule.to_value(), json!({ "all": [future] }));
        // A known field with an op of another kind is not ours either.
        assert!(matches!(Rule::from_value(&json!({ "field": "installed", "op": "lt", "value": 3 })), Rule::Unknown(_)));
    }

    #[test]
    fn rules_round_trip_through_json() {
        let rule = Rule::All(vec![
            cond(Field::Installed, Op::Is(true)),
            Rule::Any(vec![cond(Field::Hltb, Op::Lt(8.)), cond(Field::Facet(FacetKind::Genre), Op::In(vec!["Party".into()]))]),
            Rule::Not(Box::new(cond(Field::Facet(FacetKind::Tag), Op::In(vec!["Early Access".into()])))),
            cond(Field::ReleaseYear, Op::Eq(2020.)),
        ]);
        assert_eq!(Rule::from_value(&rule.to_value()), rule);
    }

    #[test]
    fn the_simple_shape_round_trips() {
        let a = Cond::new(Field::Installed, Op::Is(true));
        let b = Cond::new(Field::Hltb, Op::Lt(8.));
        let x = Cond::new(Field::Facet(FacetKind::Tag), Op::In(vec!["Early Access".into()]));
        let y = Cond::new(Field::Favourite, Op::Is(true));
        for any in [false, true] {
            for except in [vec![], vec![x.clone()], vec![x.clone(), y.clone()]] {
                let simple = Simple { any, conds: vec![a.clone(), b.clone()], except };
                assert_eq!(Simple::of(&simple.to_rule()), Some(simple));
            }
        }
        assert_eq!(Simple::of(&Rule::Cond(a.clone())), Some(Simple { any: false, conds: vec![a.clone()], except: vec![] }));
        // Deeper nesting is left to the file.
        let deep = Rule::Any(vec![Rule::All(vec![Rule::Cond(a), Rule::Cond(b)]), Rule::Cond(y)]);
        assert_eq!(Simple::of(&deep), None);
        assert_eq!(Simple::of(&Rule::Unknown(json!({ "xor": [] }))), None);
    }

    #[test]
    fn the_view_menu_filters_become_a_rule() {
        let mut v = ViewFilters { install_filter: InstallFilter::Installed, ..Default::default() };
        v.only.favourites = true;
        v.statuses.insert("playing".into());
        v.facet_filters = v.facet_filters.toggled(FacetKind::GameMode, "Co-operative").toggled(FacetKind::GameMode, "Split screen");
        let Rule::All(list) = Rule::from_view(&v) else { panic!("all") };
        assert_eq!(
            list,
            vec![
                cond(Field::Installed, Op::Is(true)),
                cond(Field::Favourite, Op::Is(true)),
                cond(Field::Status, Op::In(vec!["playing".into()])),
                cond(Field::Facet(FacetKind::GameMode), Op::In(vec!["Co-operative".into(), "Split screen".into()])),
            ]
        );
        assert_eq!(Rule::from_view(&ViewFilters::default()), Rule::All(vec![]));
    }

    #[test]
    fn save_keeps_what_it_does_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        let doc = json!({
            "version": 3,
            "groups": [
                { "id": "g1", "name": "Weekend", "pinned": true, "colour": "#f5cd5b", "icon": "bolt",
                  "rule": { "all": [ { "field": "installed", "op": "is", "value": true } ] } },
                { "id": "", "name": "From a newer version" }
            ]
        });
        std::fs::write(paths.smart_groups(), doc.to_string()).unwrap();
        let mut groups = SmartGroups::load(&paths);
        assert_eq!(groups.groups.len(), 1);
        let g = &mut groups.groups[0];
        assert!(g.pinned);
        assert_eq!(g.colour.as_deref(), Some("#f5cd5b"));
        g.name = "Weekend games".into();
        save(&paths, &groups.groups).unwrap();
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(paths.smart_groups()).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 3);
        assert_eq!(on_disk["groups"][0]["name"], "Weekend games");
        assert_eq!(on_disk["groups"][0]["icon"], "bolt");
        assert_eq!(on_disk["groups"][1]["name"], "From a newer version");
        remove(&paths, "g1").unwrap();
        assert!(SmartGroups::load(&paths).is_empty());
        assert!(save(&paths, &[SmartGroup::new("g2", " ", Rule::default())]).is_err());
    }

    #[test]
    fn a_category_rename_follows_into_the_rules() {
        let mut groups = vec![SmartGroup::new(
            "g",
            "RPG minus done",
            Rule::All(vec![
                cond(Field::Category, Op::In(vec!["RPG".into()])),
                Rule::Not(Box::new(cond(Field::Category, Op::In(vec!["Done".into()])))),
            ]),
        )];
        assert!(rename_category(&mut groups, "RPG", Some("Role-playing")));
        assert!(rename_category(&mut groups, "Done", None));
        assert!(!rename_category(&mut groups, "Nope", None));
        assert_eq!(
            groups[0].rule,
            Rule::All(vec![
                cond(Field::Category, Op::In(vec!["Role-playing".into()])),
                Rule::Not(Box::new(cond(Field::Category, Op::In(vec![]))))
            ])
        );
    }

    #[test]
    fn names_used_by_a_field_are_listed_once() {
        let rule = Rule::All(vec![
            cond(Field::Status, Op::In(vec!["playing".into(), "custom-1".into()])),
            Rule::Not(Box::new(cond(Field::Status, Op::In(vec!["custom-1".into()])))),
            cond(Field::Category, Op::In(vec!["RPG".into()])),
        ]);
        assert_eq!(names_used(&rule, Field::Status), ["playing", "custom-1"]);
        assert_eq!(names_used(&rule, Field::Category), ["RPG"]);
    }

    #[test]
    fn new_ids_never_repeat() {
        let mut groups = SmartGroups::default();
        let a = groups.new_id();
        groups.groups.push(SmartGroup::new(a.clone(), "A", Rule::default()));
        assert_ne!(groups.new_id(), a);
    }

    #[test]
    fn except_on_a_newer_rule_shows_no_game_instead_of_every_game() {
        let statuses = crate::status::default_statuses();
        let (extras, ratings) = (GameExtras::default(), HashMap::new());
        let facts = with(&extras, &ratings, &statuses);
        let installed = game("a", "Installed", |s| s.installed = true);
        let newer = Rule::from_value(&json!({ "field": "achievementsPercent", "op": "gt", "value": 80 }));
        assert!(matches!(newer, Rule::Unknown(_)));
        let except = Rule::All(vec![cond(Field::Installed, Op::Is(true)), Rule::Not(Box::new(newer.clone()))]);
        assert!(!except.matches(&installed, &facts));
        assert!(!Rule::Not(Box::new(newer.clone())).matches(&installed, &facts));
        // An `any` with a known match still matches.
        assert!(Rule::Any(vec![newer, cond(Field::Installed, Op::Is(true))]).matches(&installed, &facts));
    }

    #[test]
    fn a_conditions_unknown_keys_are_kept() {
        let leaf = json!({ "field": "playtime", "op": "gt", "value": 5.0, "unit": "hours" });
        let rule = Rule::from_value(&json!({ "all": [leaf.clone()] }));
        assert!(matches!(&rule, Rule::All(list) if matches!(list[0], Rule::Cond(_))));
        assert_eq!(rule.to_value(), json!({ "all": [leaf] }));
    }

    #[test]
    fn any_with_no_conditions_is_every_game_like_all() {
        let statuses = crate::status::default_statuses();
        let (extras, ratings) = (GameExtras::default(), HashMap::new());
        let facts = with(&extras, &ratings, &statuses);
        let installed = game("a", "Installed", |s| s.installed = true);
        let other = game("b", "Other", |_| {});
        for any in [false, true] {
            let all = Simple { any, conds: Vec::new(), except: Vec::new() };
            assert!(all.to_rule().matches(&other, &facts), "any = {any}");
            assert!(all.to_rule().matches(&installed, &facts), "any = {any}");
            assert_eq!(Simple::of(&all.to_rule()), Some(all), "the editor shows it as saved");
            let but = Simple { any, conds: Vec::new(), except: vec![Cond::new(Field::Installed, Op::Is(true))] };
            assert!(but.to_rule().matches(&other, &facts), "any = {any}");
            assert!(!but.to_rule().matches(&installed, &facts), "any = {any}");
            assert_eq!(Simple::of(&but.to_rule()), Some(but), "the editor shows it as saved");
        }
    }

    #[test]
    fn day_counts_use_the_local_day_of_an_instant() {
        let utc_plus_2 = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        let utc_minus_5 = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        // 01:30 on the 9th in UTC+2.
        assert_eq!(local_day("2026-11-08T23:30:00.000Z", &utc_plus_2), NaiveDate::from_ymd_opt(2026, 11, 9));
        assert_eq!(local_day("2026-11-09T02:00:00Z", &utc_minus_5), NaiveDate::from_ymd_opt(2026, 11, 8));
        assert_eq!(local_day("2026-11-09", &utc_plus_2), NaiveDate::from_ymd_opt(2026, 11, 9));
        assert_eq!(local_day("bad", &utc_plus_2), None);
    }
}
