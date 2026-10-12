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
use heronite_data::{config_games, game_config, AppPaths, DataPaths};
use heronite_i18n::{tr, trn};
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

fn read_snapshot(data_paths: &DataPaths, app: &AppPaths, home: &str) -> Snapshot {
    let settings = game_config::load_global(data_paths, home);
    let max_recent = settings.get("maxRecentGames").and_then(|v| v.as_i64()).filter(|n| *n >= 1).unwrap_or(DEFAULT_MAX_RECENT).min(10);
    Snapshot {
        show_hidden: UiState::load(app).show_hidden,
        hidden: config_games::list_entries(data_paths, "hidden"),
        favourites: config_games::list_entries(data_paths, "favourites"),
        recent: config_games::list_entries(data_paths, "recent"),
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
            read_snapshot(&ctx.data_paths, &ctx.app, &home())
        } else {
            Snapshot { max_recent: DEFAULT_MAX_RECENT, ..Default::default() }
        };
        // Debug: `HERONITE_DEBUG_LIBRARY_PAGER=<hidden|fav>:<page index>` opens that list expanded.
        let (mut hidden_pg, mut fav_pg) = (Pager::default(), Pager::default());
        if let Some(v) = crate::env::value("HERONITE_DEBUG_LIBRARY_PAGER") {
            if let Some((which, page)) = v.split_once(':') {
                let pg = Pager::expanded_at(page.parse().unwrap_or(0));
                if which == "hidden" {
                    hidden_pg = pg
                } else {
                    fav_pg = pg
                }
            }
        }
        Self { snap, hidden_pg, fav_pg, open_select: false, busy: false, notice: None }
    }

    /// Re-reads the lists off the UI thread.
    pub(super) fn reload(&mut self, ctx: &PageCtx, cx: &mut Context<SettingsPage>) {
        let (data_paths, app) = (ctx.data_paths.clone(), ctx.app.clone());
        cx.spawn(async move |this, cx| {
            let snap = cx.background_spawn(async move { read_snapshot(&data_paths, &app, &home()) }).await;
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

type Job = Box<dyn FnOnce(&DataPaths, &str) -> anyhow::Result<String> + Send>;

impl SettingsPage {
    /// Runs a write off the UI thread; on success shows its message, reloads
    /// the lists and tells the Collection.
    fn library_job(&mut self, job: Job, cx: &mut Context<Self>) {
        if self.library.busy {
            return;
        }
        self.library.busy = true;
        let data_paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { job(&data_paths, &home()) }).await;
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
            Box::new(move |data_paths, _| {
                let n = config_games::clear_list(data_paths, key)?;
                Ok(match key {
                    "hidden" => trn!(n, "{n} game unhidden.", "{n} games unhidden."),
                    "favourites" => trn!(n, "{n} favourite removed.", "{n} favourites removed."),
                    _ => trn!(n, "{n} recent game cleared.", "{n} recent games cleared."),
                })
            }),
            cx,
        );
    }

    pub(super) fn library_unhide(&mut self, app: String, cx: &mut Context<Self>) {
        self.library_job(
            Box::new(move |data_paths, _| {
                config_games::unhide_game(data_paths, &app)?;
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
            Box::new(move |data_paths, home| {
                game_config::set_global_setting(data_paths, home, "maxRecentGames", json!(n))?;
                Ok(String::new())
            }),
            cx,
        );
    }
}

/// Console mode rows of the section.
pub(super) fn console_rows(p: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let s = &p.library.snap;
    let mut rows = vec![
        Row::toggle("show-hidden", tr!("Show hidden games"), s.show_hidden).desc(tr!("Hidden games stay in the library, marked")),
        Row::number("max-recent", tr!("Recent games to keep"), s.max_recent, 1, 10, 1, ""),
        Row::header(tr!("Hidden games \u{b7} {count}", count = s.hidden.len())),
    ];
    for (app, title) in &s.hidden {
        rows.push(Row::info(SharedString::from(format!("h:{app}")), title.clone(), tr!("A shows it again")));
    }
    if !s.hidden.is_empty() {
        rows.push(Row::action("unhide-all", tr!("Show all hidden games")).enabled(!p.library.busy));
    }
    rows.push(Row::header(tr!("Favourites \u{b7} {count}", count = s.favourites.len())));
    for (app, title) in &s.favourites {
        rows.push(Row::info(SharedString::from(format!("f:{app}")), title.clone(), ""));
    }
    if !s.favourites.is_empty() {
        rows.push(Row::action("fav-clear", tr!("Clear favourites")).danger().enabled(!p.library.busy));
    }
    rows.push(Row::header(tr!("Recent \u{b7} {count}", count = s.recent.len())));
    if !s.recent.is_empty() {
        rows.push(Row::action("recent-clear", tr!("Clear recent games")).danger().enabled(!p.library.busy));
    }
    if let Some((text, _)) = &p.library.notice {
        rows.push(Row::info("notice", text.clone(), ""));
    }
    rows
}

pub(super) fn console_apply(p: &mut SettingsPage, outcome: crate::console::list::Outcome, cx: &mut Context<SettingsPage>) {
    use crate::console::list::{Outcome, Value};
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "show-hidden" => p.set_show_hidden(on, cx),
        Outcome::Changed(id, Value::Number { value, .. }) if id.as_ref() == "max-recent" => p.set_max_recent(value, cx),
        Outcome::Activate(id) => match id.as_ref() {
            "unhide-all" => p.library_clear("hidden", cx),
            "fav-clear" => p.library_clear("favourites", cx),
            "recent-clear" => p.library_clear("recent", cx),
            id => {
                if let Some(app) = id.strip_prefix("h:") {
                    p.library_unhide(app.to_string(), cx);
                }
            }
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{game_config, json, read_snapshot, AppPaths, DataPaths, UiState};

    #[test]
    fn snapshot_reads_lists_flag_and_max_recent() {
        let dir = std::env::temp_dir().join(format!("heronite-lib-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data_paths = DataPaths::at(dir.join("heroic"));
        let app = AppPaths { config: dir.join("cfg"), cache: dir.join("cache") };
        std::fs::create_dir_all(data_paths.store_config().parent().unwrap()).unwrap();
        std::fs::write(
            data_paths.store_config(),
            r#"{"games":{"hidden":[{"appName":"h","title":"H"}],"favourites":[{"appName":"f","title":"F"}],"recent":[]}}"#,
        )
        .unwrap();
        // Nothing saved yet: Heroic's default of 5 recent games, flag off.
        let snap = read_snapshot(&data_paths, &app, "/home/u");
        assert_eq!(snap.hidden, [("h".to_string(), "H".to_string())]);
        assert_eq!(snap.favourites.len(), 1);
        assert!(snap.recent.is_empty());
        assert_eq!(snap.max_recent, 5);
        assert!(!snap.show_hidden);

        game_config::set_global_setting(&data_paths, "/home/u", "maxRecentGames", json!(8)).unwrap();
        let ui = UiState { show_hidden: true, ..Default::default() };
        ui.save(&app).unwrap();
        let snap = read_snapshot(&data_paths, &app, "/home/u");
        assert_eq!(snap.max_recent, 8);
        assert!(snap.show_hidden);
        // 0 ("use the default") and out-of-range values are clamped like the select.
        game_config::set_global_setting(&data_paths, "/home/u", "maxRecentGames", json!(0)).unwrap();
        assert_eq!(read_snapshot(&data_paths, &app, "/home/u").max_recent, 5);
        game_config::set_global_setting(&data_paths, "/home/u", "maxRecentGames", json!(40)).unwrap();
        assert_eq!(read_snapshot(&data_paths, &app, "/home/u").max_recent, 10);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
