//! Settings → Library: "Show hidden games", the hidden/favourite/recent
//! lists of `store/config.json` (`games.*`) and `maxRecentGames`.
//!
//! "Show hidden games" is a UI flag owned by the Collection (ui-state); the
//! page only asks for it with `PageEvent::ShowHidden` and mirrors it.

mod pager;
mod render;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ui_state::UiState;
use heronite_data::{config_games, game_config, AppPaths, HeroicPaths};
use serde_json::json;

use super::SettingsPage;
use crate::pages::{PageCtx, PageEvent};
use pager::Pager;

pub(super) use render::notice_line;

type Entries = Vec<(String, String)>;

/// Everything the section shows, read from disk.
#[derive(Default, Clone, Debug, PartialEq)]
struct Snapshot {
    show_hidden: bool,
    hidden: Entries,
    favourites: Entries,
    recent: Entries,
    max_recent: i64,
}

/// Heroic: `maxRecentGames` defaults to 5 (and 0 means "use the default").
const DEFAULT_MAX_RECENT: i64 = 5;

fn read_snapshot(heroic: &HeroicPaths, app: &AppPaths, home: &str) -> Snapshot {
    let settings = game_config::load_global(heroic, home);
    let max_recent = settings
        .get("maxRecentGames")
        .and_then(|v| v.as_i64())
        .filter(|n| *n >= 1)
        .unwrap_or(DEFAULT_MAX_RECENT)
        .min(10);
    Snapshot {
        show_hidden: UiState::load(app).show_hidden,
        hidden: config_games::list_entries(heroic, "hidden"),
        favourites: config_games::list_entries(heroic, "favourites"),
        recent: config_games::list_entries(heroic, "recent"),
        max_recent,
    }
}

pub(super) struct State {
    snap: Snapshot,
    hidden_pg: Pager,
    fav_pg: Pager,
    open_select: bool,
    busy: bool,
    notice: Option<(SharedString, bool)>,
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

impl State {
    pub(super) fn new(ctx: &PageCtx) -> Self {
        // Debug captures only keep the first frames: read in place there.
        let snap = if crate::env::value("HERONITE_DEBUG_PAGE").is_some() {
            read_snapshot(&ctx.heroic, &ctx.app, &home())
        } else {
            Snapshot { max_recent: DEFAULT_MAX_RECENT, ..Default::default() }
        };
        // Debug: `HERONITE_DEBUG_LIBRARY_PAGER=<hidden|fav>:<page index>` opens that list expanded.
        let (mut hidden_pg, mut fav_pg) = (Pager::default(), Pager::default());
        if let Some(v) = crate::env::value("HERONITE_DEBUG_LIBRARY_PAGER") {
            if let Some((which, page)) = v.split_once(':') {
                let pg = Pager::expanded_at(page.parse().unwrap_or(0));
                if which == "hidden" { hidden_pg = pg } else { fav_pg = pg }
            }
        }
        Self { snap, hidden_pg, fav_pg, open_select: false, busy: false, notice: None }
    }

    /// Re-reads the lists off the UI thread.
    pub(super) fn reload(&mut self, ctx: &PageCtx, cx: &mut Context<SettingsPage>) {
        let (heroic, app) = (ctx.heroic.clone(), ctx.app.clone());
        cx.spawn(async move |this, cx| {
            let snap = cx.background_spawn(async move { read_snapshot(&heroic, &app, &home()) }).await;
            this.update(cx, |this, cx| {
                this.library.snap = snap;
                this.library.hidden_pg.clamp(this.library.snap.hidden.len());
                this.library.fav_pg.clamp(this.library.snap.favourites.len());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

type Job = Box<dyn FnOnce(&HeroicPaths, &str) -> anyhow::Result<String> + Send>;

impl SettingsPage {
    /// Runs a write off the UI thread; on success shows its message, reloads
    /// the lists and tells the Collection.
    fn library_job(&mut self, job: Job, cx: &mut Context<Self>) {
        if self.library.busy {
            return;
        }
        self.library.busy = true;
        let heroic = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { job(&heroic, &home()) }).await;
            this.update(cx, |this, cx| {
                this.library.busy = false;
                match res {
                    Ok(text) => {
                        this.library.notice = (!text.is_empty()).then(|| (text.into(), false));
                        cx.emit(PageEvent::LibraryChanged);
                    }
                    Err(err) => this.library.notice = Some((format!("{err:#}").into(), true)),
                }
                let ctx = this.ctx.clone();
                this.library.reload(&ctx, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn library_clear(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.library_job(
            Box::new(move |heroic, _| {
                let n = config_games::clear_list(heroic, key)?;
                Ok(match key {
                    "hidden" => format!("{n} game(s) unhidden."),
                    "favourites" => format!("{n} favourite(s) removed."),
                    _ => format!("{n} recent game(s) cleared."),
                })
            }),
            cx,
        );
    }

    pub(super) fn library_unhide(&mut self, app: String, cx: &mut Context<Self>) {
        self.library_job(
            Box::new(move |heroic, _| {
                config_games::unhide_game(heroic, &app)?;
                Ok(String::new())
            }),
            cx,
        );
    }

    pub(super) fn set_show_hidden(&mut self, on: bool, cx: &mut Context<Self>) {
        self.library.snap.show_hidden = on;
        cx.emit(PageEvent::ShowHidden(on));
        cx.notify();
    }

    pub(super) fn set_max_recent(&mut self, n: i64, cx: &mut Context<Self>) {
        self.library.snap.max_recent = n;
        self.library_job(
            Box::new(move |heroic, home| {
                game_config::set_global_setting(heroic, home, "maxRecentGames", json!(n))?;
                Ok(String::new())
            }),
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{game_config, json, read_snapshot, AppPaths, HeroicPaths, UiState};

    #[test]
    fn snapshot_reads_lists_flag_and_max_recent() {
        let dir = std::env::temp_dir().join(format!("heronite-lib-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let heroic = HeroicPaths::at(dir.join("heroic"));
        let app = AppPaths { config: dir.join("cfg"), cache: dir.join("cache") };
        std::fs::create_dir_all(heroic.config().parent().unwrap()).unwrap();
        std::fs::write(
            heroic.config(),
            r#"{"games":{"hidden":[{"appName":"h","title":"H"}],"favourites":[{"appName":"f","title":"F"}],"recent":[]}}"#,
        )
        .unwrap();
        // Nothing saved yet: Heroic's default of 5 recent games, flag off.
        let snap = read_snapshot(&heroic, &app, "/home/u");
        assert_eq!(snap.hidden, [("h".to_string(), "H".to_string())]);
        assert_eq!(snap.favourites.len(), 1);
        assert!(snap.recent.is_empty());
        assert_eq!(snap.max_recent, 5);
        assert!(!snap.show_hidden);

        game_config::set_global_setting(&heroic, "/home/u", "maxRecentGames", json!(8)).unwrap();
        let ui = UiState { show_hidden: true, ..Default::default() };
        ui.save(&app).unwrap();
        let snap = read_snapshot(&heroic, &app, "/home/u");
        assert_eq!(snap.max_recent, 8);
        assert!(snap.show_hidden);
        // 0 ("use the default") and out-of-range values are clamped like the select.
        game_config::set_global_setting(&heroic, "/home/u", "maxRecentGames", json!(0)).unwrap();
        assert_eq!(read_snapshot(&heroic, &app, "/home/u").max_recent, 5);
        game_config::set_global_setting(&heroic, "/home/u", "maxRecentGames", json!(40)).unwrap();
        assert_eq!(read_snapshot(&heroic, &app, "/home/u").max_recent, 10);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
