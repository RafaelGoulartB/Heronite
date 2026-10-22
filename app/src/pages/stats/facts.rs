//! What the page asks about each game's status, answered once per library:
//! its status slug and the day it was beaten or abandoned. The tabs ask it
//! per game (lanes, month cards, the Backlog's panels), and asked on the fly
//! each answer was a search over the statuses and the session history, on
//! every frame. Built off the UI thread when the library, its status history
//! or the sessions change.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use heronite_data::stats::{month_ends, running_count, weekly_hours, History, Period};
use heronite_data::status_history::day_of;
use heronite_data::Library;

use super::backlog::BacklogData;

pub(super) const BEATEN: [&str; 2] = ["beaten", "completed"];
pub(super) const ABANDONED: [&str; 1] = ["abandoned"];

#[derive(Default)]
pub(super) struct GameFacts {
    /// Status slug of each game (its status id when the status has no slug).
    slug: Vec<String>,
    beaten: Vec<Option<NaiveDate>>,
    abandoned: Vec<Option<NaiveDate>>,
    /// `(day, game)`, oldest first.
    beaten_days: Vec<(NaiveDate, usize)>,
    abandoned_days: Vec<(NaiveDate, usize)>,
    /// Games per status id.
    per_status: HashMap<String, usize>,
    /// Library index of each game key (the plans keep keys).
    index: HashMap<String, usize>,
    /// Games ever opened (play time or a session), and played 5h or more.
    pub opened: usize,
    pub played_5h: usize,
    /// Hours a week played over the last six months, without endless games
    /// (how fast the queue moves).
    pub pace: f64,
    /// The pile chart: from the first session, at each month's end, how many
    /// games were added, started and beaten by then.
    pub pile: Option<Pile>,
}

pub(super) struct Pile {
    pub from: NaiveDate,
    pub added: Vec<usize>,
    pub started: Vec<usize>,
    pub beaten: Vec<usize>,
}

impl GameFacts {
    /// The day a game entered its status is the recorded change, else its
    /// last session (for changes made before they were recorded).
    pub fn build(lib: &Library, backlog: &BacklogData, h: &History, today: NaiveDate) -> Self {
        let slug_of: HashMap<&str, &str> = lib.statuses.iter().map(|s| (s.id.as_str(), s.slug.as_str())).collect();
        let ids = |slugs: &[&str]| -> Vec<&str> {
            lib.statuses.iter().filter(|s| slugs.contains(&s.slug.as_str())).map(|s| s.id.as_str()).collect()
        };
        let (beaten_ids, abandoned_ids) = (ids(&BEATEN), ids(&ABANDONED));
        let mut f = Self::default();
        for (i, g) in lib.games.iter().enumerate() {
            let slug = slug_of.get(g.status_id.as_str()).map_or_else(|| g.status_id.clone(), |s| (*s).to_string());
            let day = |slugs: &[&str], ids: &[&str]| {
                slugs
                    .contains(&slug.as_str())
                    .then(|| backlog.changes.get(&i).and_then(|c| day_of(c, &g.status_id, ids)).or_else(|| h.last_day(i)))
                    .flatten()
            };
            let (b, a) = (day(&BEATEN, &beaten_ids), day(&ABANDONED, &abandoned_ids));
            f.beaten_days.extend(b.map(|d| (d, i)));
            f.abandoned_days.extend(a.map(|d| (d, i)));
            *f.per_status.entry(g.status_id.clone()).or_default() += 1;
            f.index.insert(g.key.clone(), i);
            f.opened += usize::from(g.playtime_minutes > 0 || g.sessions.as_ref().is_some_and(|s| !s.is_empty()));
            f.played_5h += usize::from(g.playtime_minutes >= 300);
            f.slug.push(slug);
            f.beaten.push(b);
            f.abandoned.push(a);
        }
        f.beaten_days.sort_unstable();
        f.abandoned_days.sort_unstable();
        let endless: HashSet<usize> = f.with(&["endless"]).into_iter().collect();
        f.pace = weekly_hours(h, today, 26, |g| endless.contains(&g));
        f.pile = h.since.map(|from| {
            let n = lib.games.len();
            // A game was in the library by the day it was first played.
            let added: Vec<NaiveDate> = (0..n)
                .filter_map(|g| match (backlog.added.get(&g).copied(), h.first_day(g)) {
                    (Some(a), Some(first)) => Some(a.min(first)),
                    (a, first) => a.or(first),
                })
                .collect();
            let started: Vec<NaiveDate> = (0..n).filter_map(|g| h.first_day(g)).collect();
            let beaten: Vec<NaiveDate> = f.beaten_days.iter().map(|(d, _)| *d).collect();
            let points = month_ends(from, today);
            Pile {
                from,
                added: running_count(&added, &points),
                started: running_count(&started, &points),
                beaten: running_count(&beaten, &points),
            }
        });
        f
    }

    /// Built for a library (empty until then).
    pub fn ready(&self) -> bool {
        !self.slug.is_empty()
    }

    /// Library index of the game with `key`.
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.index.get(key).copied()
    }

    pub fn slug(&self, game: usize) -> &str {
        self.slug.get(game).map_or("", String::as_str)
    }

    /// The day `game` was beaten (or completed), if it is.
    pub fn beaten_on(&self, game: usize) -> Option<NaiveDate> {
        self.beaten.get(game).copied().flatten()
    }

    pub fn abandoned_on(&self, game: usize) -> Option<NaiveDate> {
        self.abandoned.get(game).copied().flatten()
    }

    /// Games beaten inside `p`, oldest first, with the day.
    pub fn beaten_in(&self, p: &Period) -> Vec<(usize, NaiveDate)> {
        in_period(&self.beaten_days, p)
    }

    pub fn abandoned_in(&self, p: &Period) -> Vec<(usize, NaiveDate)> {
        in_period(&self.abandoned_days, p)
    }

    /// Games whose status is one of `slugs`.
    pub fn with(&self, slugs: &[&str]) -> Vec<usize> {
        (0..self.slug.len()).filter(|g| slugs.contains(&self.slug(*g))).collect()
    }

    /// How many games have the status `id`.
    pub fn in_status(&self, id: &str) -> usize {
        self.per_status.get(id).copied().unwrap_or(0)
    }

    /// How many games have one of the status `slugs`.
    pub fn count(&self, slugs: &[&str]) -> usize {
        self.slug.iter().filter(|s| slugs.contains(&s.as_str())).count()
    }
}

fn in_period(days: &[(NaiveDate, usize)], p: &Period) -> Vec<(usize, NaiveDate)> {
    let from = days.partition_point(|(d, _)| *d < p.start);
    let to = days.partition_point(|(d, _)| *d < p.end);
    days[from..to].iter().map(|(d, g)| (*g, *d)).collect()
}

#[cfg(test)]
mod tests {
    use super::in_period;
    use chrono::NaiveDate;
    use heronite_data::stats::{Period, Span};

    #[test]
    fn a_period_takes_its_own_days() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let days = [(d(1, 31), 4), (d(2, 1), 2), (d(2, 28), 7), (d(3, 1), 1)];
        let feb = Period::of(Span::Month, d(2, 10));
        assert_eq!(in_period(&days, &feb), vec![(2, d(2, 1)), (7, d(2, 28))]);
    }
}
