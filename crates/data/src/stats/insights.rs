//! Discoveries: facts about a period worth a sentence ("back to X after 387
//! days", "Saturday is a third of your time"), each with a score of how far
//! it is from the usual so the page can lead with the most surprising; and
//! milestones of the whole history (100 h played, a 30-day streak...).

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

use super::{History, Period, Span, Stats};

#[derive(Clone, Debug, PartialEq)]
pub enum Fact {
    /// Played `game` again on `back` after `days` without it (`last` = the day before the gap).
    Return { game: usize, days: i64, last: NaiveDate, back: NaiveDate },
    /// The period's longest session, `ratio` times the average one.
    Longest { game: usize, secs: f64, start: NaiveDateTime, ratio: f64 },
    /// The game whose time grew most against the previous period.
    Rising { game: usize, secs: f64, prev_secs: f64 },
    /// Games played for the first time (`prev` in the previous period).
    NewGames { n: usize, prev: Option<usize>, secs: f64 },
    /// The longest stretch without play.
    Pause { days: usize, last: NaiveDate, back: NaiveDate },
    /// The weekday with the biggest share of the time (Monday = 0).
    Weekday { day: usize, share: f64 },
    /// Longest run of days with play, and the game most played in it.
    Streak { days: usize, from: NaiveDate, to: NaiveDate, game: Option<usize> },
    /// The day with the most play.
    Marathon { day: NaiveDate, secs: f64, games: usize },
    /// Sessions that started between midnight and 5 h.
    Owl { sessions: usize, share: f64 },
    /// How few games hold 80% of the time.
    Focus { games: usize, of: usize },
    /// The period against the previous one.
    Compare { secs: f64, prev_secs: f64 },
}

impl Fact {
    /// Stable id of the kind (pinned and hidden kinds are kept by it).
    pub fn kind(&self) -> &'static str {
        match self {
            Fact::Return { .. } => "return",
            Fact::Longest { .. } => "longest",
            Fact::Rising { .. } => "rising",
            Fact::NewGames { .. } => "new",
            Fact::Pause { .. } => "pause",
            Fact::Weekday { .. } => "weekday",
            Fact::Streak { .. } => "streak",
            Fact::Marathon { .. } => "marathon",
            Fact::Owl { .. } => "owl",
            Fact::Focus { .. } => "focus",
            Fact::Compare { .. } => "compare",
        }
    }

    /// The game the fact is about, if one.
    pub fn game(&self) -> Option<usize> {
        match self {
            Fact::Return { game, .. } | Fact::Longest { game, .. } | Fact::Rising { game, .. } => Some(*game),
            Fact::Streak { game, .. } => *game,
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Insight {
    pub fact: Fact,
    /// How unusual it is; higher leads.
    pub score: f64,
}

/// Days with play of each game, from the whole history.
fn game_days(h: &History) -> HashMap<usize, Vec<NaiveDate>> {
    let mut out: HashMap<usize, Vec<NaiveDate>> = HashMap::new();
    for s in &h.sessions {
        out.entry(s.game).or_default().push(s.start.date());
    }
    for days in out.values_mut() {
        days.sort_unstable();
        days.dedup();
    }
    out
}

/// The discoveries of `st` (computed for `st.period`), most surprising first.
pub fn insights(h: &History, st: &Stats) -> Vec<Insight> {
    let p = st.period;
    let mut out = Vec::new();
    let mut push = |fact: Fact, score: f64| {
        if score.is_finite() && score > 0. {
            out.push(Insight { fact, score });
        }
    };
    // The biggest return to a game inside the period.
    let days = game_days(h);
    let mut best: Option<Fact> = None;
    for (game, d) in &days {
        for w in d.windows(2) {
            let gap = (w[1] - w[0]).num_days();
            if p.contains(w[1]) && gap >= 30 && best.as_ref().is_none_or(|b| matches!(b, Fact::Return { days, .. } if gap > *days)) {
                best = Some(Fact::Return { game: *game, days: gap, last: w[0], back: w[1] });
            }
        }
    }
    // A return that ends the period's longest pause tells the same story once.
    let return_back = match &best {
        Some(Fact::Return { back, .. }) => Some(*back),
        _ => None,
    };
    if let Some(f @ Fact::Return { days, .. }) = best {
        push(f, days as f64 / 60.);
    }
    if let Some(l) = st.longest {
        let avg = st.avg_session_secs();
        let ratio = if avg > 0. { l.secs / avg } else { 1. };
        push(Fact::Longest { game: l.game, secs: l.secs, start: l.start, ratio }, ratio / 3. + l.secs / 3600. / 8.);
    }
    if let Some(prev) = p.prev() {
        let mut before: HashMap<usize, f64> = HashMap::new();
        for x in h.pieces_in(prev) {
            *before.entry(x.game as usize).or_default() += x.secs as f64;
        }
        if let Some(g) = st.games.iter().max_by(|a, b| {
            let da = a.secs - before.get(&a.game).copied().unwrap_or(0.);
            let db = b.secs - before.get(&b.game).copied().unwrap_or(0.);
            da.total_cmp(&db)
        }) {
            let prev_secs = before.get(&g.game).copied().unwrap_or(0.);
            push(Fact::Rising { game: g.game, secs: g.secs, prev_secs }, (g.secs - prev_secs) / 3600. / 25.);
        }
        let prev_new = {
            let firsts: Vec<NaiveDate> = h.first_day.iter().flatten().copied().collect();
            Some(firsts.iter().filter(|d| prev.contains(**d)).count())
        };
        let new: Vec<&super::GameStat> = st.games.iter().filter(|g| g.new).collect();
        if !new.is_empty() {
            let secs = new.iter().map(|g| g.secs).sum();
            push(Fact::NewGames { n: new.len(), prev: prev_new, secs }, new.len() as f64 / 12. + 0.2);
        }
        if let Some(prev_secs) = st.prev_secs.filter(|s| *s > 0.) {
            let change = (st.total_secs / prev_secs - 1.).abs();
            push(Fact::Compare { secs: st.total_secs, prev_secs }, change * 1.5);
        }
    }
    if let Some(b) = st.longest_break.filter(|b| b.days >= 4 && Some(b.back) != return_back) {
        push(Fact::Pause { days: b.days, last: b.last, back: b.back }, b.days as f64 / 25.);
    }
    let per_day: Vec<f64> = st.heat.iter().map(|r| r.iter().sum()).collect();
    let total: f64 = per_day.iter().sum();
    if total > 0. {
        let (day, secs) = per_day.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(d, s)| (d, *s)).unwrap_or((0, 0.));
        let share = secs / total;
        push(Fact::Weekday { day, share }, (share - 1. / 7.) * 5.);
    }
    if let Some((from, to)) = st.streak_range.filter(|_| st.streak >= 5) {
        push(Fact::Streak { days: st.streak, from, to, game: st.streak_game }, st.streak as f64 / 15.);
    }
    if let Some((day, secs)) = st.busiest.filter(|b| b.1 >= 4. * 3600.) {
        let games = st.days.iter().position(|(d, _)| *d == day).map_or(1, |_| {
            h.sessions_between(day, day + Duration::days(1)).iter().map(|s| s.game).collect::<std::collections::BTreeSet<_>>().len()
        });
        push(Fact::Marathon { day, secs, games }, secs / 3600. / 10.);
    }
    let owl = st.session_list_starts(h).filter(|t| t.hour() < 5).count();
    if owl >= 3 && st.sessions > 0 {
        let share = owl as f64 / st.sessions as f64;
        push(Fact::Owl { sessions: owl, share }, share * 3.);
    }
    if st.games.len() >= 5 {
        let mut acc = 0.;
        let n = st.games.iter().take_while(|g| {
            let before = acc;
            acc += g.secs;
            before < st.total_secs * 0.8
        });
        let n = n.count();
        push(Fact::Focus { games: n, of: st.games.len() }, (1. - n as f64 / st.games.len() as f64) * 0.8);
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

impl Stats {
    /// Start times of the sessions of the period.
    fn session_list_starts<'a>(&'a self, h: &'a History) -> impl Iterator<Item = NaiveDateTime> + 'a {
        h.sessions_in(self.period).iter().map(|s| s.start)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MilestoneKind {
    /// Hours played in all.
    Hours,
    Sessions,
    /// Different games played.
    Games,
    /// Hours in one game.
    GameHours,
    /// Days in a row with play.
    Streak,
}

impl MilestoneKind {
    pub const ALL: [MilestoneKind; 5] = [Self::Hours, Self::Sessions, Self::Games, Self::GameHours, Self::Streak];

    pub fn steps(self) -> &'static [u32] {
        match self {
            Self::Hours => &[100, 250, 500, 1000, 1500, 2000, 3000, 5000],
            Self::Sessions => &[100, 250, 500, 1000, 1500, 2000, 3000],
            Self::Games => &[10, 25, 50, 100, 150, 200],
            Self::GameHours => &[25, 50, 100, 200, 500],
            Self::Streak => &[7, 14, 30, 60, 100],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Milestone {
    pub kind: MilestoneKind,
    /// The highest step reached and when, if any.
    pub reached: Option<(u32, NaiveDate)>,
    /// The game of [`MilestoneKind::GameHours`].
    pub game: Option<usize>,
    /// Where the history stands now.
    pub value: f64,
    /// The next step, if any.
    pub next: Option<u32>,
    /// Days of history it took per unit lately (to tell when the next comes):
    /// units gained over the last 90 days.
    pub recent: f64,
}

/// Milestones of the whole recorded history (sessions only), as of `today`.
pub fn milestones(h: &History, today: NaiveDate) -> Vec<Milestone> {
    // The history keeps its sessions oldest first.
    let sessions: Vec<&super::Played> = h.sessions.iter().collect();
    let since = today - Duration::days(90);
    let mut out = Vec::new();
    for kind in MilestoneKind::ALL {
        let steps = kind.steps();
        let mut reached = None;
        let mut value = 0.;
        let mut recent = 0.;
        let mut game = None;
        match kind {
            MilestoneKind::Hours | MilestoneKind::Sessions | MilestoneKind::Games => {
                let mut seen = std::collections::HashSet::new();
                for s in &sessions {
                    let add = match kind {
                        MilestoneKind::Hours => s.secs / 3600.,
                        MilestoneKind::Sessions => 1.,
                        _ => {
                            if seen.insert(s.game) {
                                1.
                            } else {
                                0.
                            }
                        }
                    };
                    value += add;
                    if s.start.date() > since {
                        recent += add;
                    }
                    if let Some(step) = steps.iter().rev().find(|st| value >= **st as f64) {
                        if reached.is_none_or(|(r, _)| *step > r) {
                            reached = Some((*step, s.start.date()));
                        }
                    }
                }
            }
            MilestoneKind::GameHours => {
                let mut per: HashMap<usize, f64> = HashMap::new();
                for s in &sessions {
                    let v = per.entry(s.game).or_default();
                    *v += s.secs / 3600.;
                    if let Some(step) = steps.iter().rev().find(|st| *v >= **st as f64) {
                        if reached.is_none_or(|(r, _)| *step > r) {
                            reached = Some((*step, s.start.date()));
                            game = Some(s.game);
                        }
                    }
                }
                if let Some((g, v)) = per.iter().max_by(|a, b| a.1.total_cmp(b.1)) {
                    value = *v;
                    game = game.or(Some(*g));
                    recent = h.sessions_of(*g).filter(|s| s.start.date() > since).map(|s| s.secs / 3600.).sum();
                }
            }
            MilestoneKind::Streak => {
                let mut days: Vec<NaiveDate> = sessions.iter().map(|s| s.start.date()).collect();
                days.dedup();
                let mut run = 0;
                for (i, d) in days.iter().enumerate() {
                    run = if i > 0 && days[i - 1] + Duration::days(1) == *d { run + 1 } else { 1 };
                    value = value.max(run as f64);
                    if let Some(step) = steps.iter().rev().find(|st| run >= **st) {
                        if reached.is_none_or(|(r, _)| *step > r) {
                            reached = Some((*step, *d));
                        }
                    }
                }
            }
        }
        let next = steps.iter().copied().find(|s| *s as f64 > value);
        out.push(Milestone { kind, reached, game, value, next, recent });
    }
    out
}

/// The same span a year before `p` (memories).
pub fn year_before(p: Period) -> Period {
    let d = p.start;
    let back = NaiveDate::from_ymd_opt(d.year() - 1, d.month(), d.day().min(28)).unwrap_or(d);
    match p.span {
        Span::All => p,
        span => Period::of(span, back),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LocalGameSession;
    use crate::stats::{compute, history_in, GameInput};
    use crate::JsNum;
    use chrono::FixedOffset;

    fn s(started: &str, secs: f64) -> LocalGameSession {
        LocalGameSession {
            started_at: started.into(),
            ended_at: started.into(),
            elapsed_seconds: JsNum(secs),
            source: "heroic".into(),
            game_action_name: None,
            extra: Default::default(),
        }
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_return_leads_and_milestones_count() {
        let a: Vec<LocalGameSession> =
            (1..=8).map(|d| s(&format!("2026-03-{d:02}T20:00:00Z"), 3600. * 2.)).chain([s("2026-09-20T02:00:00Z", 5. * 3600.)]).collect();
        let b = [s("2026-09-21T02:30:00Z", 3600.), s("2026-09-22T03:00:00Z", 1800.)];
        let h = history_in(
            [
                GameInput { sessions: &a, lifetime_minutes: 0, first_played: None },
                GameInput { sessions: &b, lifetime_minutes: 0, first_played: None },
            ],
            &FixedOffset::east_opt(0).unwrap(),
        );
        let today = day(2026, 10, 9);
        let st = compute(&h, Period::of(Span::Year, today), today);
        let all = insights(&h, &st);
        assert_eq!(all[0].fact, Fact::Return { game: 0, days: 196, last: day(2026, 3, 8), back: day(2026, 9, 20) });
        assert!(all.iter().any(|i| matches!(i.fact, Fact::Streak { days: 8, .. })));
        assert!(all.iter().any(|i| matches!(i.fact, Fact::Owl { sessions: 3, .. })));
        assert!(all.windows(2).all(|w| w[0].score >= w[1].score));
        assert_eq!(all[0].fact.kind(), "return");

        let m = milestones(&h, today);
        let streak = m.iter().find(|x| x.kind == MilestoneKind::Streak).unwrap();
        assert_eq!((streak.reached, streak.value, streak.next), (Some((7, day(2026, 3, 7))), 8., Some(14)));
        let games = m.iter().find(|x| x.kind == MilestoneKind::Games).unwrap();
        assert_eq!((games.reached, games.value, games.next), (None, 2., Some(10)));
        assert_eq!(year_before(Period::of(Span::Month, day(2026, 10, 9))), Period::of(Span::Month, day(2025, 10, 1)));
    }
}
