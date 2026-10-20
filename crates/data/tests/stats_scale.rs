//! The statistics on a library far bigger than a real one. The page runs
//! these off the UI thread when the period changes, so a period should take
//! a few tens of milliseconds at most; the per-game lookups (asked many
//! times per frame) must be next to free. Ignored by default (it times, it
//! does not check behaviour); run with
//! `cargo test --release -p heronite-data --test stats_scale -- --ignored --nocapture`.

use std::time::{Duration as StdDuration, Instant};

use chrono::{Duration, FixedOffset, NaiveDate, NaiveDateTime};
use heronite_data::models::LocalGameSession;
use heronite_data::stats::{
    capture_hours, captures_per_session, compute, game_sheet, history_in, insights, journeys, milestones, moments, GameInput, Period, Span,
};
use heronite_data::JsNum;

/// Budget (ms) of everything a period computes off the UI thread.
const PERIOD_MS: u64 = 60;
/// Budget (ms) of asking every game once (render-time lookups).
const LOOKUP_MS: u64 = 1;

/// Games, years of history and sessions per game (on average).
const GAMES: usize = 3000;
const YEARS: i64 = 12;
const SESSIONS: usize = 60;

/// Small deterministic generator (no `rand` in the data crate).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn library(today: NaiveDate) -> Vec<Vec<LocalGameSession>> {
    let mut r = Lcg(7);
    let days = YEARS * 365;
    (0..GAMES)
        .map(|_| {
            let n = (r.next() as usize) % (SESSIONS * 2);
            let from = (r.next() as i64) % days;
            (0..n)
                .map(|_| {
                    let day = today - Duration::days((from - (r.next() as i64 % 400)).clamp(0, days));
                    let at = day.and_hms_opt((r.next() % 24) as u32, (r.next() % 60) as u32, 0).unwrap();
                    LocalGameSession {
                        started_at: format!("{}Z", at.format("%Y-%m-%dT%H:%M:%S")),
                        ended_at: String::new(),
                        elapsed_seconds: JsNum((300 + r.next() % 14000) as f64),
                        source: "heroic".into(),
                        game_action_name: None,
                        extra: Default::default(),
                    }
                })
                .collect()
        })
        .collect()
}

fn time<T>(what: &str, budget_ms: u64, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    let took = t.elapsed();
    let flag = if took > StdDuration::from_millis(budget_ms) { "  <-- over budget" } else { "" };
    println!("{what:<34} {took:>10.2?} (budget {budget_ms} ms){flag}");
    out
}

#[test]
#[ignore = "timing on a synthetic library, run by hand"]
fn the_page_stays_fast_on_a_huge_library() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let games = library(today);
    let h = time("history", 2000, || {
        history_in(
            games.iter().map(|s| GameInput { sessions: s, lifetime_minutes: 0, first_played: None }),
            &FixedOffset::east_opt(0).unwrap(),
        )
    });
    let sessions: usize = games.iter().map(Vec::len).sum();
    println!("{GAMES} games, {sessions} sessions");
    for span in [Span::Week, Span::Month, Span::Year, Span::All] {
        let p = Period::of(span, today);
        let st = time(&format!("compute {span:?}"), PERIOD_MS, || compute(&h, p, today));
        time(&format!("insights {span:?}"), PERIOD_MS, || insights(&h, &st));
        if span != Span::Week {
            time(&format!("journeys {span:?}"), PERIOD_MS, || journeys(&h, p, today));
        }
        if let Some(g) = st.games.first() {
            time(&format!("game sheet {span:?}"), PERIOD_MS, || game_sheet(&h, g.game, p, today));
        }
    }
    time("milestones", PERIOD_MS, || milestones(&h, today));
    let mut r = Lcg(11);
    let shots: Vec<(usize, NaiveDateTime)> = (0..50_000)
        .map(|_| {
            let day = today - Duration::days((r.next() % (YEARS as u64 * 365)) as i64);
            ((r.next() as usize) % GAMES, day.and_hms_opt((r.next() % 24) as u32, (r.next() % 60) as u32, 0).unwrap())
        })
        .collect();
    let all = Period::of(Span::All, today);
    time("moments (50k captures)", PERIOD_MS, || moments(&shots));
    time("captures per session", PERIOD_MS, || captures_per_session(&h, &shots, all));
    time("capture hours", PERIOD_MS, || capture_hours(&h, &shots, all));
    time("total_secs of every game", LOOKUP_MS, || (0..GAMES).map(|g| h.total_secs(g)).sum::<f64>());
    time("last_day of every game", LOOKUP_MS, || (0..GAMES).filter_map(|g| h.last_day(g)).count());
}
