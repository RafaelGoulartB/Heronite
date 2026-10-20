//! How the page gets its data. The Collection hands over its loaded library
//! ([`StatsPage::set_library`]); the session history, the status facts and
//! the screenshot listings are built from it off the UI thread, and each
//! period is computed off it too ([`super::compute`]). Screenshot previews
//! are made in the background, one at a time.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Local};
use gpui_kit::*;
use heronite_data::models::LocalGameSession;
use heronite_data::stats::{self, GameInput, History};
use heronite_data::Library;
use heronite_images::ImageRef;
use heronite_media::{list_screenshots, GameQuery, ThumbnailCache};

use super::{backlog, compute, facts, today, Shot, StatsPage};

fn build_history(lib: &Library) -> History {
    // `HERONITE_DEBUG_STATS_SCALE=N`: every session N times, each copy two days
    // earlier than the last, to try the page on a history N times bigger.
    let scale = crate::env::parse::<i64>("HERONITE_DEBUG_STATS_SCALE").unwrap_or(1).max(1);
    let copies: Vec<Vec<LocalGameSession>> = if scale > 1 {
        lib.games
            .iter()
            .map(|g| {
                let own = g.sessions.as_deref().map_or(&[][..], Vec::as_slice);
                (0..scale)
                    .flat_map(|k| {
                        own.iter().filter_map(move |s| {
                            let at = DateTime::parse_from_rfc3339(&s.started_at).ok()? - chrono::Duration::days(2 * k);
                            Some(LocalGameSession { started_at: at.to_rfc3339(), ..s.clone() })
                        })
                    })
                    .collect()
            })
            .collect()
    } else {
        Vec::new()
    };
    stats::history(lib.games.iter().enumerate().map(|(i, g)| GameInput {
        sessions: copies.get(i).map_or_else(|| g.sessions.as_deref().map_or(&[][..], Vec::as_slice), Vec::as_slice),
        lifetime_minutes: g.playtime_minutes,
        first_played: g.first_played.as_deref(),
    }))
}

/// Screenshots of each game that has sessions. `games` comes most played
/// first: when two games match the same capture folder (a demo and the full
/// game), the folder goes to the first.
fn list_shots(folder: &Path, games: Vec<(usize, GameQuery)>) -> HashMap<usize, Vec<Shot>> {
    let mut out = HashMap::new();
    let mut taken = std::collections::HashSet::new();
    for (game, query) in games {
        let found = list_screenshots(folder, &query);
        if found.matched_folder.as_ref().is_some_and(|f| !taken.insert(f.clone())) {
            continue;
        }
        let items: Vec<Shot> = found
            .items
            .into_iter()
            .filter_map(|s| {
                let at = DateTime::from_timestamp_millis(s.taken_at? as i64)?.with_timezone(&Local).naive_local();
                Some(Shot { path: s.path, at })
            })
            .collect();
        if !items.is_empty() {
            out.insert(game, items);
        }
    }
    out
}

impl StatsPage {
    /// The Collection's library and its art chains (covers and wide art).
    /// A new library has its history, status facts and screenshot listings
    /// rebuilt off the UI thread; the same one only computes again on a new
    /// day (the Collection hands it over on every visit).
    pub fn set_library(
        &mut self,
        lib: Arc<Library>,
        chains: &HashMap<SharedString, Vec<ImageRef>>,
        stages: &HashMap<SharedString, Vec<ImageRef>>,
        cx: &mut Context<Self>,
    ) {
        let same = |l: &Option<Arc<Library>>| l.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib));
        if same(&self.lib) || same(&self.loading) {
            if self.computed_on.is_some_and(|d| d != today()) {
                self.recompute(cx);
            }
            return;
        }
        self.loading = Some(lib.clone());
        let (chains, stages) = (chains.clone(), stages.clone());
        let folder = lib.settings.screenshots_folder().map(Path::to_path_buf);
        let mut played: Vec<(usize, u64)> = lib
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| g.sessions.as_ref().is_some_and(|s| !s.is_empty()))
            .map(|(i, g)| (i, g.playtime_minutes))
            .collect();
        played.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let queries: Vec<(usize, GameQuery)> = played.into_iter().map(|(i, _)| (i, GameQuery::from_game(&lib.games[i]))).collect();
        cx.spawn(async move |this, cx| {
            let built = lib.clone();
            let history = Arc::new(cx.background_spawn(async move { build_history(&built) }).await);
            // Library and history change together.
            let current = this
                .update(cx, |this, cx| {
                    let current = this.loading.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib));
                    if current {
                        this.loading = None;
                        this.thumb_cache =
                            Some(ThumbnailCache::new(lib.paths.screenshot_thumbnails_dir(), lib.settings.screenshots.cache_limit_mb.get()));
                        this.lib = Some(lib.clone());
                        this.chains = chains;
                        this.stages = stages;
                        this.history = Some(history.clone());
                        this.recompute(cx);
                        if let Some(year) = this.review_wanted.take() {
                            this.open_review(year, cx);
                        }
                        cx.notify();
                    }
                    current
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let (built, h) = (lib.clone(), history.clone());
            let (data, facts) = cx
                .background_spawn(async move {
                    let data = backlog::BacklogData::read(&built);
                    let facts = facts::GameFacts::build(&built, &data, &h, today());
                    (data, facts)
                })
                .await;
            let still = |this: &Self| this.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib));
            this.update(cx, |this, cx| {
                if still(this) {
                    this.backlog = Arc::new(data);
                    this.facts = Arc::new(facts);
                    this.recompute(cx);
                }
            })
            .ok();
            let Some(folder) = folder else { return };
            let shots = cx.background_spawn(async move { list_shots(&folder, queries) }).await;
            this.update(cx, |this, cx| {
                if still(this) {
                    this.shots = Some(Arc::new(shots));
                    this.recompute(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Computes the period off the UI thread ([`compute`]); the page shows
    /// the last result until this one is back.
    pub(super) fn recompute(&mut self, cx: &mut Context<Self>) {
        let Some(history) = self.history.clone() else { return };
        let today = today();
        self.computed_on = Some(today);
        self.job += 1;
        let (id, period) = (self.job, self.period);
        let job = compute::Job {
            history: history.clone(),
            lib: self.lib.clone(),
            shots: self.shots.clone(),
            facts: self.facts.clone(),
            period,
            album_game: self.album_game,
            today,
        };
        // An open sheet and the review follow the period (one game, one year).
        if let Some(sh) = &self.sheet {
            self.sheet = Some(Arc::new(stats::game_sheet(&history, sh.game, period, today)));
        }
        if let Some(r) = self.review.as_mut() {
            r.refresh(&history, self.lib.as_deref(), self.shots.as_deref());
        }
        cx.spawn(async move |this, cx| {
            let snap = cx.background_spawn(async move { job.run() }).await;
            this.update(cx, |this, cx| {
                if this.job == id {
                    this.apply(snap);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply(&mut self, s: compute::Snapshot) {
        self.stats = Some(s.stats);
        self.insights = Some(s.insights);
        self.milestones = Some(s.milestones);
        self.journeys = Some(s.journeys);
        self.year_summary = s.year_summary;
        self.derived = s.derived;
        self.album = s.album;
        self.months = s.months;
        self.memories = s.memories;
        self.hover = None;
    }

    /// Preview texture of the screenshot at `path` (`w`x`h` logical px); the
    /// preview is made in the background the first time.
    pub(super) fn shot(&mut self, path: &Path, w: f32, h: f32, window: &Window, cx: &mut Context<Self>) -> Option<ImageSource> {
        match self.thumbs.get(path) {
            Some(Some(thumb)) => {
                let thumb = thumb.clone();
                self.covers.get_file(&thumb, w, h, window.scale_factor())
            }
            Some(None) => None,
            None => {
                let cache = self.thumb_cache.clone()?;
                self.thumbs.insert(path.to_path_buf(), None);
                self.thumb_queue.push_back(path.to_path_buf());
                if !std::mem::replace(&mut self.thumb_loading, true) {
                    cx.spawn(async move |this, cx| {
                        while let Ok(Some(source)) = this.update(cx, |this, _| {
                            let next = this.thumb_queue.pop_front();
                            this.thumb_loading = next.is_some();
                            next
                        }) {
                            let (c, src) = (cache.clone(), source.clone());
                            let thumb = cx.background_spawn(async move { c.thumbnail_path(&src).ok() }).await;
                            this.update(cx, |this, cx| {
                                if let Some(t) = thumb {
                                    this.thumbs.insert(source, Some(t));
                                    cx.notify();
                                }
                            })
                            .ok();
                        }
                    })
                    .detach();
                }
                None
            }
        }
    }
}
