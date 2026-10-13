//! Parsing legendary's download progress (port of `onInstallOrUpdateOutput`).
//!
//! legendary logs, every second or so:
//! ```text
//! [DLManager] INFO: = Progress: 12.34% (123/1000), Running for 00:01:23, ETA: 00:08:20
//! [DLManager] INFO:  - Downloaded: 123.45 MiB, Written: 120.00 MiB
//! [DLManager] INFO:  - Cache usage: 2.00 MiB, active tasks: 16
//! [DLManager] INFO:  + Download\t- 5.43 MiB/s (raw) / 4.20 MiB/s (decompressed)
//! [DLManager] INFO:  + Disk\t- 5.12 MiB/s (write) / 0.00 MiB/s (read)
//! ```
//! preceded once by `Download size: 12345.67 MiB (Compression savings: ..)`.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// What the UI shows for the running operation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// 0..=100.
    pub percent: f64,
    /// MiB downloaded in this run.
    pub downloaded_mib: f64,
    /// `HH:MM:SS`.
    pub eta: String,
    /// MiB/s.
    pub down_speed: f64,
    /// MiB/s.
    pub disk_speed: f64,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| !v.is_nan())
}

/// Accumulates the lines of one report and emits a [`Progress`] when it is
/// complete (the `Disk` line closes a report).
#[derive(Debug, Default)]
pub struct ProgressParser {
    /// Full download size of the game (`manifest.download_size`), bytes.
    total_bytes: Option<f64>,
    /// `Download size:` of this run (smaller when resuming), MiB.
    run_download_mib: Option<f64>,
    eta: Option<String>,
    legendary_percent: Option<f64>,
    downloaded: Option<f64>,
    down_speed: Option<f64>,
    disk_speed: Option<f64>,
}

impl ProgressParser {
    pub fn new(total_download_bytes: Option<f64>) -> Self {
        Self { total_bytes: total_download_bytes.filter(|b| *b > 0.), ..Default::default() }
    }

    /// Feeds one output line.
    pub fn feed(&mut self, line: &str) -> Option<Progress> {
        static SIZE: OnceLock<Regex> = OnceLock::new();
        static ETA: OnceLock<Regex> = OnceLock::new();
        static PERC: OnceLock<Regex> = OnceLock::new();
        static DL: OnceLock<Regex> = OnceLock::new();
        static DOWN: OnceLock<Regex> = OnceLock::new();
        static DISK: OnceLock<Regex> = OnceLock::new();

        if let Some(c) = re(&SIZE, r"Download size: ([\d.]+) MiB").captures(line) {
            self.run_download_mib = num(&c[1]);
        }
        if let Some(c) = re(&ETA, r"ETA: (\d\d:\d\d:\d\d)").captures(line) {
            self.eta = Some(c[1].to_string());
            if let Some(p) = re(&PERC, r"Progress: ([\d.]+)%").captures(line) {
                self.legendary_percent = num(&p[1]);
            }
        }
        if let Some(c) = re(&DL, r"Downloaded: ([\d.]+) MiB").captures(line) {
            self.downloaded = num(&c[1]);
        }
        if let Some(c) = re(&DOWN, r"Download\t- ([\d.]+) MiB").captures(line) {
            self.down_speed = num(&c[1]);
        }
        if let Some(c) = re(&DISK, r"Disk\t- ([\d.]+) MiB").captures(line) {
            self.disk_speed = num(&c[1]);
            return self.take_report();
        }
        None
    }

    fn take_report(&mut self) -> Option<Progress> {
        let report = (|| {
            let downloaded = self.downloaded?;
            let percent = match self.total_bytes {
                // Heroic: (downloaded + what an earlier run already fetched) / total.
                Some(total) => {
                    let total_mib = (total / 1024. / 1024. * 100.).round() / 100.;
                    // Without the run's own size nothing is known to be cached.
                    let cached = self.run_download_mib.map_or(0., |run| (total_mib - run).max(0.));
                    (((downloaded + cached) / total_mib) * 10000.).round() / 100.
                }
                None => self.legendary_percent?,
            };
            Some(Progress {
                percent: percent.clamp(0., 100.),
                downloaded_mib: downloaded,
                eta: self.eta.clone().unwrap_or_default(),
                down_speed: self.down_speed.unwrap_or(0.),
                disk_speed: self.disk_speed.unwrap_or(0.),
            })
        })();
        self.eta = None;
        self.legendary_percent = None;
        self.downloaded = None;
        self.down_speed = None;
        self.disk_speed = None;
        report
    }
}

/// The report of gogdl (GOG) and nile (Amazon Games), printed every second:
/// ```text
/// [PROGRESS] INFO: = Progress: 12.34 1234/10000, Running for: 00:00:05, ETA: 00:01:23
/// [PROGRESS] INFO: = Downloaded: 123.45 MiB, Written: 150.00 MiB
/// [PROGRESS] INFO:  + Download\t- 5.12 MiB/s (raw) / 8.00 MiB/s (decompressed)
/// [PROGRESS] INFO:  + Disk\t- 9.00 MiB/s (write) / 0.00 MiB/s (read)
/// ```
/// nile leaves out the parts in parentheses. The percent is the tool's own
/// (it already counts what an earlier run fetched); the `Disk` line closes a
/// report.
#[derive(Debug, Default)]
pub struct ToolProgressParser {
    percent: Option<f64>,
    eta: Option<String>,
    downloaded: Option<f64>,
    down_speed: Option<f64>,
}

impl ToolProgressParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one output line.
    pub fn feed(&mut self, line: &str) -> Option<Progress> {
        static PERC: OnceLock<Regex> = OnceLock::new();
        static ETA: OnceLock<Regex> = OnceLock::new();
        static DL: OnceLock<Regex> = OnceLock::new();
        static DOWN: OnceLock<Regex> = OnceLock::new();
        static DISK: OnceLock<Regex> = OnceLock::new();

        if let Some(c) = re(&PERC, r"= Progress: ([\d.]+) ").captures(line) {
            self.percent = num(&c[1]);
            self.eta = re(&ETA, r"ETA: (\d\d:\d\d:\d\d)").captures(line).map(|c| c[1].to_string());
        }
        if let Some(c) = re(&DL, r"Downloaded: ([\d.]+) MiB").captures(line) {
            self.downloaded = num(&c[1]);
        }
        if let Some(c) = re(&DOWN, r"Download\t- ([\d.]+) MiB").captures(line) {
            self.down_speed = num(&c[1]);
        }
        if let Some(c) = re(&DISK, r"Disk\t- ([\d.]+) MiB").captures(line) {
            let report = self.percent.map(|percent| Progress {
                percent: percent.clamp(0., 100.),
                downloaded_mib: self.downloaded.unwrap_or(0.),
                eta: self.eta.clone().unwrap_or_default(),
                down_speed: self.down_speed.unwrap_or(0.),
                disk_speed: num(&c[1]).unwrap_or(0.),
            });
            *self = Self::default();
            return report;
        }
        None
    }
}

/// `Verification progress: 3/8 (37.5%)` style percentages of `repair`.
pub fn verification_percent(line: &str) -> Option<f64> {
    static VERIFY: OnceLock<Regex> = OnceLock::new();
    if !line.contains("erif") {
        return None;
    }
    re(&VERIFY, r"\(([\d.]+)%\)").captures(line).and_then(|c| num(&c[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: [&str; 6] = [
        "[DLManager] INFO: = Progress: 12.34% (123/1000), Running for 00:01:23, ETA: 00:08:20",
        "[DLManager] INFO:  - Downloaded: 512.00 MiB, Written: 500.00 MiB",
        "[DLManager] INFO:  - Cache usage: 2.00 MiB, active tasks: 16",
        "[DLManager] INFO:  + Download\t- 5.43 MiB/s (raw) / 4.20 MiB/s (decompressed)",
        "[DLManager] INFO:  + Disk\t- 5.12 MiB/s (write) / 0.00 MiB/s (read)",
        "[DLManager] INFO: ...",
    ];

    fn run(parser: &mut ProgressParser, lines: &[&str]) -> Vec<Progress> {
        lines.iter().filter_map(|l| parser.feed(l)).collect()
    }

    #[test]
    fn full_report_uses_manifest_total_like_heroic() {
        // 2048 MiB total, a fresh download of 2048 MiB: 512 MiB done = 25%.
        let mut p = ProgressParser::new(Some(2048. * 1024. * 1024.));
        assert!(p.feed("[cli] INFO: Download size: 2048.00 MiB (Compression savings: 3.1%)").is_none());
        let out = run(&mut p, &REPORT);
        assert_eq!(out.len(), 1);
        let r = &out[0];
        assert_eq!(r.percent, 25.);
        assert_eq!(r.downloaded_mib, 512.);
        assert_eq!(r.eta, "00:08:20");
        assert_eq!(r.down_speed, 5.43);
        assert_eq!(r.disk_speed, 5.12);
    }

    #[test]
    fn resumed_downloads_count_what_was_already_fetched() {
        // Total 2048 MiB, this run only needs 1024 MiB: 512 downloaded now plus
        // the 1024 MiB from before = 75%.
        let mut p = ProgressParser::new(Some(2048. * 1024. * 1024.));
        p.feed("Download size: 1024.00 MiB (Compression savings: 0.0%)");
        let out = run(&mut p, &REPORT);
        assert_eq!(out[0].percent, 75.);
    }

    #[test]
    fn without_a_total_the_percent_of_legendary_is_used() {
        let mut p = ProgressParser::new(None);
        let out = run(&mut p, &REPORT);
        assert_eq!(out[0].percent, 12.34);
        // The next report starts clean: a partial one yields nothing.
        assert!(p.feed("[DLManager] INFO:  + Disk\t- 1.00 MiB/s (write) / 0.00 MiB/s (read)").is_none());
    }

    #[test]
    fn partial_reports_and_noise_give_nothing() {
        let mut p = ProgressParser::new(Some(1024. * 1024.));
        assert!(run(&mut p, &["hello", "[cli] INFO: Installing game", REPORT[3], REPORT[4]]).is_empty());
    }

    #[test]
    fn percent_is_clamped() {
        let mut p = ProgressParser::new(Some(100. * 1024. * 1024.));
        let out = run(&mut p, &REPORT);
        assert_eq!(out[0].percent, 100.);
    }

    #[test]
    fn gogdl_and_nile_reports() {
        let gogdl = [
            "[PROGRESS] INFO: = Progress: 42.50 4250/10000, Running for: 00:00:05, ETA: 00:01:23",
            "[PROGRESS] INFO: = Downloaded: 123.45 MiB, Written: 150.00 MiB",
            "[PROGRESS] INFO:  + Download\t- 5.12 MiB/s (raw) / 8.00 MiB/s (decompressed)",
            "[PROGRESS] INFO:  + Disk\t- 9.00 MiB/s (write) / 0.00 MiB/s (read)",
        ];
        let mut p = ToolProgressParser::new();
        let out: Vec<Progress> = gogdl.iter().filter_map(|l| p.feed(l)).collect();
        assert_eq!(out, [Progress { percent: 42.5, downloaded_mib: 123.45, eta: "00:01:23".into(), down_speed: 5.12, disk_speed: 9. }]);
        let nile = [
            "[PROGRESS] INFO: = Progress: 7.00 70/1000, Running for: 00:00:01, ETA: 00:00:30",
            "[PROGRESS] INFO: = Downloaded: 1.00 MiB, Written: 1.00 MiB",
            "[PROGRESS] INFO:  + Download\t- 2.00 MiB/s",
            "[PROGRESS] INFO:  + Disk\t- 3.00 MiB/s",
        ];
        let out: Vec<Progress> = nile.iter().filter_map(|l| p.feed(l)).collect();
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].percent, out[0].down_speed, out[0].disk_speed), (7., 2., 3.));
        // A report without its percent line gives nothing.
        assert!(p.feed(nile[3]).is_none());
    }

    #[test]
    fn verification_lines() {
        assert_eq!(verification_percent("[cli] INFO: Verification progress: 3/8 (37.5%) [12.5 MiB/s]"), Some(37.5));
        assert_eq!(verification_percent("[DLManager] INFO: = Progress: 12.34% (1/2)"), None);
    }
}
