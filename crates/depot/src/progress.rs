//! What a running install reports, and the speeds in it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How far an [`crate::install`] is. Sizes in bytes, speeds in bytes a second.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    /// Fetched in this run (a chunk fetched again after it was dropped
    /// counts twice).
    pub downloaded: u64,
    /// What this run has to fetch: every chunk the unfinished files use, once.
    pub download_total: u64,
    /// File bytes written in this run.
    pub written: u64,
    /// File bytes this run has to write.
    pub write_total: u64,
    /// File bytes an earlier run already finished (resumed, not written again).
    pub resumed: u64,
    pub files_done: usize,
    pub files_total: usize,
    /// Over the last few seconds.
    pub down_speed: f64,
    pub disk_speed: f64,
    /// `None` until something has been written.
    pub eta: Option<Duration>,
}

impl Progress {
    /// `0..=100`, over the whole plan (resumed files count as done).
    pub fn percent(&self) -> f64 {
        let total = self.resumed + self.write_total;
        if total == 0 {
            return if self.files_done >= self.files_total { 100. } else { 0. };
        }
        ((self.resumed + self.written) as f64 / total as f64 * 100.).clamp(0., 100.)
    }
}

/// The rate of a growing counter over a sliding window.
#[derive(Debug)]
pub(crate) struct Meter {
    window: Duration,
    samples: VecDeque<(Instant, u64)>,
}

impl Meter {
    pub fn new(window: Duration) -> Self {
        Self { window, samples: VecDeque::new() }
    }

    /// Records the counter's value now; gives the rate in units a second.
    pub fn sample(&mut self, now: Instant, value: u64) -> f64 {
        self.samples.push_back((now, value));
        while self.samples.len() > 2 && self.samples.front().is_some_and(|(t, _)| now.duration_since(*t) > self.window) {
            self.samples.pop_front();
        }
        let (t0, v0) = self.samples[0];
        let secs = now.duration_since(t0).as_secs_f64();
        if secs < 0.05 {
            return 0.;
        }
        value.saturating_sub(v0) as f64 / secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_follow_the_window() {
        let start = Instant::now();
        let mut m = Meter::new(Duration::from_secs(2));
        assert_eq!(m.sample(start, 0), 0.);
        assert_eq!(m.sample(start + Duration::from_secs(1), 100), 100.);
        assert_eq!(m.sample(start + Duration::from_secs(2), 300), 150.);
        // The first sample has left the window.
        assert_eq!(m.sample(start + Duration::from_secs(4), 500), 100.);
    }

    #[test]
    fn percent_counts_resumed_files() {
        let p = Progress { written: 25, write_total: 50, resumed: 50, ..Default::default() };
        assert_eq!(p.percent(), 75.);
        assert_eq!(Progress::default().percent(), 100.);
    }
}
