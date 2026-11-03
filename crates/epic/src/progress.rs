//! What the download queue shows of a running download: [`Progress`], from
//! a native download's report ([`heronite_depot::Progress`]) or from the
//! progress lines of gogdl and nile ([`ToolProgressParser`]).

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

const MIB: f64 = 1024. * 1024.;

/// `HH:MM:SS` (hours may pass 99).
fn clock(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

impl From<&heronite_depot::Progress> for Progress {
    /// A native download's report: the percent of the whole install
    /// (files an earlier run finished count as done), this run's MiB, speeds
    /// in MiB/s.
    fn from(p: &heronite_depot::Progress) -> Self {
        Self {
            percent: (p.percent() * 100.).round() / 100.,
            downloaded_mib: (p.downloaded as f64 / MIB * 100.).round() / 100.,
            eta: p.eta.map(clock).unwrap_or_default(),
            down_speed: (p.down_speed / MIB * 100.).round() / 100.,
            disk_speed: (p.disk_speed / MIB * 100.).round() / 100.,
        }
    }
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| !v.is_nan())
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

    #[test]
    fn native_reports() {
        let p = heronite_depot::Progress {
            downloaded: 512 << 20,
            download_total: 1024 << 20,
            written: 300 << 20,
            write_total: 1000 << 20,
            resumed: 1000 << 20,
            down_speed: 5.5 * MIB,
            disk_speed: 2. * MIB,
            eta: Some(std::time::Duration::from_secs(3 * 3600 + 25 * 60 + 7)),
            ..Default::default()
        };
        let r = Progress::from(&p);
        assert_eq!(r, Progress { percent: 65., downloaded_mib: 512., eta: "03:25:07".into(), down_speed: 5.5, disk_speed: 2. });
        assert_eq!(Progress::from(&heronite_depot::Progress::default()).eta, "");
    }

    #[test]
    fn verification_lines() {
        assert_eq!(verification_percent("[cli] INFO: Verification progress: 3/8 (37.5%) [12.5 MiB/s]"), Some(37.5));
        assert_eq!(verification_percent("[DLManager] INFO: = Progress: 12.34% (1/2)"), None);
    }
}
