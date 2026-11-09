//! Times and sizes of the Downloads page: what is left, when it ends, when
//! each waiting element starts and how long ago one finished.

use chrono::{Duration, Local, TimeZone};
use heronite_epic::progress::Progress;
use heronite_epic::queue::DmElement;
use heronite_epic::util::{file_size_local, parse_size};
use heronite_i18n::tr;

use crate::services::widgets::eta_secs;

/// Wall clock `secs` from now: `14:28`.
pub(super) fn clock_in(secs: u64) -> String {
    (Local::now() + Duration::seconds(secs as i64)).format("%H:%M").to_string()
}

/// Download size of an element in bytes (the queue keeps it as text).
pub(super) fn size_bytes(el: &DmElement) -> Option<f64> {
    el.params.size.as_deref().and_then(parse_size)
}

/// What the running element still has to download, in bytes.
pub(super) fn left_bytes(el: &DmElement, progress: Option<&Progress>) -> Option<f64> {
    let size = size_bytes(el)?;
    let done = progress.map_or(0., |p| p.percent.clamp(0., 100.) / 100.);
    Some(size * (1. - done))
}

/// Seconds until the running element ends: its tool's ETA, else what is
/// left at `speed` MiB/s.
pub(super) fn current_eta(el: &DmElement, progress: Option<&Progress>, speed: f64) -> Option<u64> {
    progress.and_then(|p| eta_secs(&p.eta)).filter(|s| *s > 0).or_else(|| {
        let left = left_bytes(el, progress)?;
        (speed > 0.).then(|| (left / (speed * 1024. * 1024.)) as u64)
    })
}

/// Seconds from now until each waiting element starts (`waiting` in queue
/// order), when the current one ends in `current_eta` and the rest download
/// at `speed` MiB/s. `None` from the first element with no known size.
pub(super) fn starts_in(current_eta: u64, waiting: &[DmElement], speed: f64) -> Vec<Option<u64>> {
    let mut at = Some(current_eta);
    waiting
        .iter()
        .map(|el| {
            let starts = at;
            at = at.zip(size_bytes(el)).filter(|_| speed > 0.).map(|(t, b)| t + (b / (speed * 1024. * 1024.)) as u64);
            starts
        })
        .collect()
}

/// `1.21 GiB` in the interface's decimal separator.
pub(super) fn size_text(bytes: f64) -> String {
    file_size_local(bytes)
}

/// When a finished element ended: "today, 01:58", "yesterday, 22:14", or
/// the full date.
pub(super) fn relative_date(ms: u64) -> String {
    let Some(at) = Local.timestamp_millis_opt(ms as i64).single() else { return String::new() };
    let days = (Local::now().date_naive() - at.date_naive()).num_days();
    let time = at.format("%H:%M").to_string();
    match days {
        0 => tr!("today, {time}", time),
        1 => tr!("yesterday, {time}", time),
        _ => super::date_text(ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_epic::queue::{ElementKind, InstallParams};

    fn el(size: &str) -> DmElement {
        DmElement::new(ElementKind::Install, InstallParams { size: Some(size.into()), ..Default::default() })
    }

    #[test]
    fn the_current_eta_falls_back_to_the_speed() {
        let e = el("1 GiB");
        let half = Progress { percent: 50., eta: String::new(), ..Default::default() };
        assert_eq!(current_eta(&e, Some(&half), 512.), Some(1));
        let told = Progress { eta: "00:01:40".into(), ..half.clone() };
        assert_eq!(current_eta(&e, Some(&told), 512.), Some(100));
        assert_eq!(current_eta(&e, None, 0.), None);
    }

    #[test]
    fn waiting_elements_start_one_after_the_other() {
        let waiting = [el("1 GiB"), el("2 GiB"), el("?"), el("1 GiB")];
        assert_eq!(starts_in(10, &waiting, 1024.), [Some(10), Some(11), Some(13), None]);
        assert_eq!(starts_in(10, &waiting[..1], 0.), [Some(10)]);
    }
}
