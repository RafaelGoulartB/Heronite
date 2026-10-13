//! When the release list of a source is fetched. GitHub allows 60 anonymous
//! API calls an hour per IP address, shared with the rest of the app (update
//! check, DXVK) and every other program on the network, so a check spends one
//! only when something changed:
//!
//! * a source is checked when it is shown, at most every [`AUTO_EVERY`]; the
//!   refresh button checks it again after [`MANUAL_COOLDOWN`];
//! * a check reads the source's release feed first (a page of the website,
//!   not the API). When it is the same as at the last check, nothing was
//!   published and no API call is made;
//! * every API reply says how many calls are left. When none are left nothing
//!   is asked until GitHub's reset time, and automatic checks stop while
//!   fewer than [`AUTO_RESERVE`] are left.
//!
//! The state lives in `store/wine-release-checks.json`, so it survives
//! restarts.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use heronite_data::{json_store, DataPaths};
use serde::{Deserialize, Serialize};

use crate::github::{self, ApiError, ApiReply, Budget};
use crate::repos::{self, Repository};
use crate::store;
use crate::types::WineVersionInfo;

/// A source shown again within this time is not checked again by itself.
pub const AUTO_EVERY: Duration = Duration::from_secs(6 * 3600);
/// The refresh button does nothing within this time of the last check.
pub const MANUAL_COOLDOWN: Duration = Duration::from_secs(60);
/// API calls automatic checks leave for the rest of the app.
pub const AUTO_RESERVE: u32 = 10;
/// Releases fetched per source.
const RELEASES_PER_FETCH: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckMode {
    /// The source was shown.
    Auto,
    /// The user asked (refresh button).
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The releases were fetched and merged into the cache.
    Updated,
    /// Nothing new: no API call was needed.
    UpToDate,
    /// No API call may be made before `until` (Unix seconds); the cache stays.
    RateLimited {
        until: u64,
    },
    Failed(String),
}

/// What a check reaches over the network; tests use a fake.
pub trait Remote: Send + Sync {
    fn feed(&self, repo: Repository) -> Result<String>;
    fn releases(&self, repo: Repository, count: usize, now: u64) -> ApiReply;
}

/// The real GitHub.
pub struct GitHub;

impl Remote for GitHub {
    fn feed(&self, repo: Repository) -> Result<String> {
        github::feed(repo)
    }
    fn releases(&self, repo: Repository, count: usize, now: u64) -> ApiReply {
        github::api_releases(repo, count, now)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceCheck {
    /// Last check that reached GitHub (Unix seconds).
    #[serde(default)]
    checked_at: u64,
    /// Fingerprint of the feed when the cache was last brought up to date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    feed: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiState {
    /// Calls left at the last reply, until `reset_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remaining: Option<u32>,
    #[serde(default)]
    reset_at: u64,
    /// GitHub refused a call: nothing is asked before this (Unix seconds).
    #[serde(default)]
    blocked_until: u64,
}

impl ApiState {
    /// Until when a call of `mode` must wait, `None` if it may go now.
    fn wait_until(&self, mode: CheckMode, now: u64) -> Option<u64> {
        if self.blocked_until > now {
            return Some(self.blocked_until);
        }
        let floor = if mode == CheckMode::Auto { AUTO_RESERVE } else { 0 };
        match self.remaining {
            Some(left) if left <= floor && self.reset_at > now => Some(self.reset_at),
            _ => None,
        }
    }

    fn record(&mut self, budget: Option<Budget>) {
        if let Some(b) = budget {
            self.remaining = Some(b.remaining);
            self.reset_at = b.reset_at;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckState {
    /// By source `type`.
    #[serde(default)]
    sources: BTreeMap<String, SourceCheck>,
    #[serde(default)]
    api: ApiState,
}

impl CheckState {
    fn load(path: &Path) -> Self {
        json_store::read_value(path).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        let value = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(e) => return log::warn!("wine manager: {e}"),
        };
        if let Err(e) = json_store::update_value(path, |doc| {
            *doc = value;
            Ok(())
        }) {
            log::warn!("wine manager: saving {}: {e:#}", path.display());
        }
    }
}

/// The newest release has no build yet: its files are often uploaded a while
/// after it is published, so the feed must not be trusted until they are.
fn waiting_for_files(fetched: &[WineVersionInfo]) -> bool {
    fetched.iter().find(|r| !r.version.ends_with("-latest")).is_some_and(|r| !r.downloadable())
}

/// A failed fetch still counts as a check, so a broken source is not asked
/// again right away; the fingerprint stays, the cache too.
fn failed(state: &mut CheckState, kind: &str, now: u64, message: String) -> CheckOutcome {
    state.sources.entry(kind.to_owned()).or_default().checked_at = now;
    CheckOutcome::Failed(message)
}

/// Brings the cached releases of `repo` up to date if needed. Blocking
/// (network). `now`: Unix seconds.
pub fn check_source(data_paths: &DataPaths, remote: &dyn Remote, repo: Repository, mode: CheckMode, now: u64) -> CheckOutcome {
    let path = data_paths.wine_release_checks();
    let mut state = CheckState::load(&path);
    let kind = repo.kind();
    let source = state.sources.get(kind).cloned().unwrap_or_default();
    let cached = store::read_releases(data_paths).iter().any(|r| r.kind == kind && r.downloadable());

    let since = now.saturating_sub(source.checked_at);
    let recent = since < MANUAL_COOLDOWN.as_secs() || (mode == CheckMode::Auto && cached && since < AUTO_EVERY.as_secs());
    if recent {
        return CheckOutcome::UpToDate;
    }

    // A feed that cannot be read only costs the API call it would have saved.
    let fingerprint = match remote.feed(repo) {
        Ok(xml) => github::feed_fingerprint(&xml),
        Err(e) => {
            log::info!("wine manager: {kind} feed: {e:#}");
            None
        }
    };
    if cached && fingerprint.is_some() && fingerprint == source.feed {
        state.sources.insert(kind.to_owned(), SourceCheck { checked_at: now, feed: fingerprint });
        state.save(&path);
        return CheckOutcome::UpToDate;
    }

    if let Some(until) = state.api.wait_until(mode, now) {
        return CheckOutcome::RateLimited { until };
    }
    let reply = remote.releases(repo, RELEASES_PER_FETCH, now);
    state.api.record(reply.budget);
    let outcome = match reply.result {
        Ok(body) => match repos::parse_releases(repo, &body).and_then(|fetched| {
            let waiting = waiting_for_files(&fetched);
            store::apply_fetched(data_paths, fetched).map(|_| waiting)
        }) {
            Ok(waiting) => {
                let feed = if waiting { None } else { fingerprint };
                state.sources.insert(kind.to_owned(), SourceCheck { checked_at: now, feed });
                CheckOutcome::Updated
            }
            Err(e) => failed(&mut state, kind, now, format!("{e:#}")),
        },
        Err(ApiError::RateLimited { until }) => {
            state.api.blocked_until = until;
            CheckOutcome::RateLimited { until }
        }
        Err(ApiError::Other(e)) => failed(&mut state, kind, now, e),
    };
    state.save(&path);
    outcome
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const DW: &str = include_str!("../tests/fixtures/dw-releases.json");

    fn feed(entry: &str) -> String {
        format!("<feed><entry><id>{entry}</id><updated>2026-09-30T13:14:54Z</updated></entry></feed>")
    }

    /// Answers from a script; counts what was asked.
    pub(crate) struct Fake {
        pub(crate) feed: Mutex<Result<String, String>>,
        reply: Mutex<Vec<(Result<String, ApiError>, Option<Budget>)>>,
        feed_calls: AtomicUsize,
        api_calls: AtomicUsize,
        /// Sources whose releases were asked, in order.
        pub(crate) asked: Mutex<Vec<Repository>>,
    }

    impl Fake {
        pub(crate) fn new(feed: &str) -> Self {
            Self {
                feed: Mutex::new(Ok(feed.into())),
                reply: Mutex::new(Vec::new()),
                feed_calls: AtomicUsize::new(0),
                api_calls: AtomicUsize::new(0),
                asked: Mutex::new(Vec::new()),
            }
        }
        pub(crate) fn answer(&self, result: Result<String, ApiError>, remaining: u32) {
            self.reply.lock().unwrap().push((result, Some(Budget { remaining, reset_at: 5000 })));
        }
        fn feed_calls(&self) -> usize {
            self.feed_calls.load(Ordering::SeqCst)
        }
        fn api_calls(&self) -> usize {
            self.api_calls.load(Ordering::SeqCst)
        }
    }

    impl Remote for Fake {
        fn feed(&self, _repo: Repository) -> Result<String> {
            self.feed_calls.fetch_add(1, Ordering::SeqCst);
            self.feed.lock().unwrap().clone().map_err(|e| anyhow::anyhow!(e))
        }
        fn releases(&self, repo: Repository, _count: usize, _now: u64) -> ApiReply {
            self.api_calls.fetch_add(1, Ordering::SeqCst);
            self.asked.lock().unwrap().push(repo);
            let mut replies = self.reply.lock().unwrap();
            let (result, budget) = if replies.is_empty() { (Ok("[]".into()), None) } else { replies.remove(0) };
            ApiReply { result, budget }
        }
    }

    fn setup() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths)
    }

    const DW_SOURCE: Repository = Repository::DwProton;

    #[test]
    fn an_unchanged_feed_costs_no_api_call() {
        let (_d, paths) = setup();
        let gh = Fake::new(&feed("dwproton-11.0-14"));
        gh.answer(Ok(DW.into()), 50);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Auto, 1000), CheckOutcome::Updated);
        assert!(store::read_releases(&paths).iter().any(|r| r.version == "dwproton-11.0-14" && r.downloadable()));
        assert_eq!(gh.api_calls(), 1);

        // Shown again soon: not even the feed is read.
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Auto, 2000), CheckOutcome::UpToDate);
        assert_eq!(gh.feed_calls(), 1);
        // Later, or asked for: the feed is read and is the same.
        let later = 1000 + AUTO_EVERY.as_secs() + 1;
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Auto, later), CheckOutcome::UpToDate);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, later + 120), CheckOutcome::UpToDate);
        assert_eq!((gh.feed_calls(), gh.api_calls()), (3, 1));
        // The refresh button right after a check does nothing.
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, later + 130), CheckOutcome::UpToDate);
        assert_eq!(gh.feed_calls(), 3);

        // Something was published: the API is asked.
        *gh.feed.lock().unwrap() = Ok(feed("dwproton-11.0-15"));
        gh.answer(Ok(DW.into()), 49);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, later + 400), CheckOutcome::Updated);
        assert_eq!(gh.api_calls(), 2);
    }

    #[test]
    fn a_feed_that_cannot_be_read_falls_back_to_the_api() {
        let (_d, paths) = setup();
        let gh = Fake::new("");
        *gh.feed.lock().unwrap() = Err("HTTP 429".into());
        gh.answer(Ok(DW.into()), 50);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Auto, 1000), CheckOutcome::Updated);
        // Without a fingerprint the next check asks the API again.
        gh.answer(Ok(DW.into()), 49);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1100), CheckOutcome::Updated);
        assert_eq!(gh.api_calls(), 2);
    }

    #[test]
    fn a_spent_budget_blocks_every_call_until_the_reset() {
        let (_d, paths) = setup();
        let gh = Fake::new(&feed("a"));
        gh.answer(Err(ApiError::RateLimited { until: 5000 }), 0);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1000), CheckOutcome::RateLimited { until: 5000 });
        // Persisted: a restart does not ask again.
        let state = CheckState::load(&paths.wine_release_checks());
        assert_eq!(state.api.blocked_until, 5000);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1100), CheckOutcome::RateLimited { until: 5000 });
        assert_eq!(gh.api_calls(), 1);
        // After the reset it goes again.
        gh.answer(Ok(DW.into()), 59);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 5001), CheckOutcome::Updated);
    }

    #[test]
    fn automatic_checks_leave_a_reserve() {
        let (_d, paths) = setup();
        let gh = Fake::new(&feed("a"));
        gh.answer(Ok(DW.into()), AUTO_RESERVE);
        assert_eq!(check_source(&paths, &gh, Repository::ProtonGe, CheckMode::Auto, 1000), CheckOutcome::Updated);
        // Another source, nothing cached: an automatic check waits for the reset...
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Auto, 1001), CheckOutcome::RateLimited { until: 5000 });
        assert_eq!(gh.api_calls(), 1);
        // ...the user may still spend what is left.
        gh.answer(Ok(DW.into()), AUTO_RESERVE - 1);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1002), CheckOutcome::Updated);
    }

    #[test]
    fn a_release_without_files_is_checked_again() {
        let (_d, paths) = setup();
        let gh = Fake::new(&feed("new"));
        let no_files = r#"[{"tag_name":"dwproton-11.0-15","published_at":"2026-10-12T10:00:00Z","assets":[]}]"#;
        gh.answer(Ok(no_files.into()), 50);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1000), CheckOutcome::Updated);
        gh.answer(Ok(DW.into()), 49);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1100), CheckOutcome::Updated, "same feed, files still missing");
        assert_eq!(gh.api_calls(), 2);
    }

    #[test]
    fn failures_keep_the_cache_and_are_reported() {
        let (_d, paths) = setup();
        let gh = Fake::new(&feed("a"));
        gh.answer(Err(ApiError::Other("HTTP 500".into())), 40);
        assert_eq!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1000), CheckOutcome::Failed("HTTP 500".into()));
        gh.answer(Ok("{\"message\":\"x\"}".into()), 39);
        assert!(matches!(check_source(&paths, &gh, DW_SOURCE, CheckMode::Manual, 1100), CheckOutcome::Failed(_)));
        assert!(store::read_releases(&paths).is_empty());
    }
}
