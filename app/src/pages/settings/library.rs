//! Settings → Library: "Show hidden games", the hidden/favourite/recent
//! lists of `store/config.json` (`games.*`) and `maxRecentGames`.
//!
//! "Show hidden games" is a UI flag owned by the Collection (ui-state); the
//! page only asks for it with `PageEvent::ShowHidden` and mirrors it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ui_state::UiState;
use heronite_data::{config_games, game_config, AppPaths, HeroicPaths};
use serde_json::json;

use super::SettingsPage;
use crate::icons::Icon;
use crate::pages::{caption_with, icon_action, PageCtx, PageEvent};
use crate::theme;
use crate::ui::controls::{action_button, caption, card, row, toggle_row, ButtonKind};
use crate::ui::fields::{meta_text, select_field};
use crate::ui::icon;

pub type Entries = Vec<(String, String)>;

/// Everything the section shows, read from disk.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub show_hidden: bool,
    pub hidden: Entries,
    pub favourites: Entries,
    pub recent: Entries,
    pub max_recent: i64,
}

/// Heroic: `maxRecentGames` defaults to 5 (and 0 means "use the default").
pub const DEFAULT_MAX_RECENT: i64 = 5;

pub fn read_snapshot(heroic: &HeroicPaths, app: &AppPaths, home: &str) -> Snapshot {
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

/// Collapsed lists show this many rows.
const COLLAPSED_ROWS: usize = 5;
/// Rows per page once a list is expanded.
const PAGE_ROWS: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ListKind {
    Hidden,
    Favourites,
}

/// Expand / paginate state of one long list (not persisted).
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pager {
    expanded: bool,
    page: usize,
}

impl Pager {
    fn pages(total: usize) -> usize {
        total.div_ceil(PAGE_ROWS).max(1)
    }

    /// Keeps `page` valid after the list shrank (unhide, clear).
    fn clamp(&mut self, total: usize) {
        self.page = self.page.min(Self::pages(total) - 1);
        if total <= COLLAPSED_ROWS {
            self.expanded = false;
        }
    }

    /// Rows to draw out of `total`.
    fn range(&self, total: usize) -> std::ops::Range<usize> {
        if !self.expanded {
            return 0..total.min(COLLAPSED_ROWS);
        }
        let start = (self.page.min(Self::pages(total) - 1)) * PAGE_ROWS;
        start.min(total)..(start + PAGE_ROWS).min(total)
    }
}

pub struct State {
    pub snap: Snapshot,
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
    pub fn new(ctx: &PageCtx) -> Self {
        // Debug captures only keep the first frames: read in place there.
        let snap = if std::env::var_os("HERONITE_DEBUG_PAGE").is_some() {
            read_snapshot(&ctx.heroic, &ctx.app, &home())
        } else {
            Snapshot { max_recent: DEFAULT_MAX_RECENT, ..Default::default() }
        };
        // Debug: `HERONITE_DEBUG_LIBRARY_PAGER=<hidden|fav>:<page index>` opens that list expanded.
        let (mut hidden_pg, mut fav_pg) = (Pager::default(), Pager::default());
        if let Ok(v) = std::env::var("HERONITE_DEBUG_LIBRARY_PAGER") {
            if let Some((which, page)) = v.split_once(':') {
                let pg = Pager { expanded: true, page: page.parse().unwrap_or(0) };
                if which == "hidden" { hidden_pg = pg } else { fav_pg = pg }
            }
        }
        Self { snap, hidden_pg, fav_pg, open_select: false, busy: false, notice: None }
    }

    /// Re-reads the lists off the UI thread.
    pub fn reload(&mut self, ctx: &PageCtx, cx: &mut Context<SettingsPage>) {
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

    /// One game of a list card: title, app name below, pill action on the right.
    fn entry_row(
        &self,
        id: String,
        title: &str,
        app: &str,
        label: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let shown = if title.is_empty() { app.to_string() } else { title.to_string() };
        let desc = (!title.is_empty()).then_some(app);
        row(
            shown,
            desc,
            action_button(SharedString::from(id), ButtonKind::Glass, label, !self.library.busy, cx.listener(move |this, _, _, cx| on_click(this, cx))),
        )
        .into_any_element()
    }

    fn pager_mut(&mut self, kind: ListKind) -> &mut Pager {
        match kind {
            ListKind::Hidden => &mut self.library.hidden_pg,
            ListKind::Favourites => &mut self.library.fav_pg,
        }
    }

    /// Below a list longer than 5 rows: "Expand", or once expanded
    /// "Collapse" + "11-20 of 27" with previous / next buttons.
    fn pager_row(&self, kind: ListKind, total: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        if total <= COLLAPSED_ROWS {
            return None;
        }
        let pager = match kind {
            ListKind::Hidden => self.library.hidden_pg,
            ListKind::Favourites => self.library.fav_pg,
        };
        let tag = if kind == ListKind::Hidden { "hidden" } else { "fav" };
        let toggle = |label: &'static str, expand: bool, cx: &mut Context<Self>| {
            action_button(
                SharedString::from(format!("lib-{tag}-toggle")),
                ButtonKind::Glass,
                label,
                true,
                cx.listener(move |this, _, _, cx| {
                    let pg = this.pager_mut(kind);
                    pg.expanded = expand;
                    pg.page = 0;
                    cx.notify();
                }),
            )
        };
        let row = div().flex().items_center().justify_between().gap(px(12.)).mt(px(8.));
        if !pager.expanded {
            return Some(row.child(toggle("Expand", true, cx)).child(meta_text(format!("Showing {COLLAPSED_ROWS} of {total}"))).into_any_element());
        }
        let range = pager.range(total);
        let pages = Pager::pages(total);
        let page = pager.page.min(pages - 1);
        let step = |id: &str, ic: Icon, enabled: bool, delta: isize, cx: &mut Context<Self>| {
            let b = icon_action(SharedString::from(format!("lib-{tag}-{id}")), ic, enabled);
            if enabled {
                b.on_click(cx.listener(move |this, _, _, cx| {
                    let pg = this.pager_mut(kind);
                    pg.page = (page as isize + delta).max(0) as usize;
                    cx.notify();
                }))
                .into_any_element()
            } else {
                b.into_any_element()
            }
        };
        let nav = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(step("prev", Icon::ChevronLeft, page > 0, -1, cx))
            .child(meta_text(format!("{}\u{2013}{} of {total} \u{b7} Page {} of {pages}", range.start + 1, range.end, page + 1)))
            .child(step("next", Icon::ChevronRight, page + 1 < pages, 1, cx));
        Some(row.child(toggle("Collapse", false, cx)).child(nav).into_any_element())
    }

    pub(super) fn library_section(&mut self, cx: &mut Context<Self>) -> Div {
        let snap = self.library.snap.clone();
        let busy = self.library.busy;

        // -- Show hidden ---------------------------------------------------------
        let show = card([toggle_row(
            "lib-show-hidden",
            "Show hidden games",
            Some("Hidden games appear in the Collection, greyed with a \"Hidden\" tag, and can be unhidden from their menu. The same switch is in the filter menu of the header."),
            snap.show_hidden,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            let on = !this.library.snap.show_hidden;
            this.set_show_hidden(on, cx);
        }))
        .into_any_element()]);

        // -- Hidden games --------------------------------------------------------
        self.library.hidden_pg.clamp(snap.hidden.len());
        self.library.fav_pg.clamp(snap.favourites.len());
        let mut rows: Vec<AnyElement> = Vec::new();
        let range = self.library.hidden_pg.range(snap.hidden.len());
        for (i, (app, title)) in snap.hidden.iter().enumerate().skip(range.start).take(range.len()) {
            let app_name = app.clone();
            rows.push(self.entry_row(format!("lib-unhide-{i}"), title, app, "Unhide", cx, move |this, cx| {
                this.library_unhide(app_name.clone(), cx)
            }));
        }
        if rows.is_empty() {
            rows.push(empty_row("No hidden games. Use \"Hide\" in a game's context menu."));
        }
        let hidden_head = caption_with(
            format!("Hidden games ({})", snap.hidden.len()),
            action_button(
                "lib-unhide-all",
                ButtonKind::Glass,
                "Unhide all",
                !busy && !snap.hidden.is_empty(),
                cx.listener(|this, _, _, cx| this.library_clear("hidden", cx)),
            ),
        );
        let hidden = card(rows);
        let hidden_pager = self.pager_row(ListKind::Hidden, snap.hidden.len(), cx);

        // -- Favourites ----------------------------------------------------------
        let mut rows: Vec<AnyElement> = Vec::new();
        let range = self.library.fav_pg.range(snap.favourites.len());
        for (i, (app, title)) in snap.favourites.iter().enumerate().skip(range.start).take(range.len()) {
            let app_name = app.clone();
            rows.push(self.entry_row(format!("lib-fav-{i}"), title, app, "Remove", cx, move |this, cx| {
                let app = app_name.clone();
                this.library_job(
                    Box::new(move |heroic, _| {
                        config_games::remove_favourite(heroic, &app)?;
                        Ok(String::new())
                    }),
                    cx,
                );
            }));
        }
        if rows.is_empty() {
            rows.push(empty_row("No favourites. Use \"Add to favourites\" in a game's context menu."));
        }
        let favs_head = caption_with(
            format!("Favourites ({})", snap.favourites.len()),
            action_button(
                "lib-fav-clear",
                ButtonKind::Glass,
                "Clear favourites",
                !busy && !snap.favourites.is_empty(),
                cx.listener(|this, _, _, cx| this.library_clear("favourites", cx)),
            ),
        );
        let favs = card(rows);
        let favs_pager = self.pager_row(ListKind::Favourites, snap.favourites.len(), cx);

        // -- Recent --------------------------------------------------------------
        let options: Vec<(SharedString, bool)> =
            (1..=10).map(|n| (SharedString::from(n.to_string()), n == snap.max_recent)).collect();
        let select = select_field(
            "lib-max-recent",
            None,
            snap.max_recent.to_string(),
            false,
            self.library.open_select,
            options,
            cx,
            |this: &mut Self, _, cx| {
                this.library.open_select = !this.library.open_select;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                this.library.open_select = false;
                this.set_max_recent(i as i64 + 1, cx);
            },
        );
        let recent = card([
            row("Recent games to show", Some("How many recently played games the tray menu lists."), div().w(px(120.)).child(select)).into_any_element(),
            row(
                "Clear recent games",
                Some(&format!("{} game(s) in the recent list", snap.recent.len())),
                action_button(
                    "lib-recent-clear",
                    ButtonKind::Glass,
                    "Clear recent",
                    !busy && !snap.recent.is_empty(),
                    cx.listener(|this, _, _, cx| this.library_clear("recent", cx)),
                ),
            )
            .into_any_element(),
        ]);

        let mut col = div()
            .flex()
            .flex_col()
            .child(show)
            .child(hidden_head)
            .child(hidden)
            .children(hidden_pager)
            .child(favs_head)
            .child(favs)
            .children(favs_pager)
            .child(caption("Recent games"))
            .child(recent);
        if let Some((text, error)) = self.library.notice.clone() {
            col = col.child(notice_line(text, error));
        }
        col
    }
}

/// Placeholder row of an empty list card.
fn empty_row(text: &str) -> AnyElement {
    div().py(px(14.)).text_size(px(13.)).text_color(theme::text_dim()).child(text.to_string()).into_any_element()
}

/// Result line under the cards (success / error).
pub(super) fn notice_line(text: SharedString, error: bool) -> Div {
    let color = if error { theme::danger() } else { theme::success() };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mt(px(14.))
        .mx(px(6.))
        .text_size(px(13.))
        .text_color(color)
        .child(icon(if error { Icon::Warning } else { Icon::Check }, px(16.), color))
        .child(text)
}


#[cfg(test)]
mod tests {
    use super::{read_snapshot, game_config, json, AppPaths, HeroicPaths, Pager, UiState};

    #[test]
    fn pager_shows_five_then_ten_per_page_and_stays_valid() {
        let mut p = Pager::default();
        assert_eq!(p.range(27), 0..5);
        assert_eq!(p.range(3), 0..3);
        p.expanded = true;
        assert_eq!((Pager::pages(27), p.range(27)), (3, 0..10));
        p.page = 2;
        assert_eq!(p.range(27), 20..27);
        // Unhiding games shrinks the list: the page index is clamped.
        p.clamp(20);
        assert_eq!((p.page, p.range(20)), (1, 10..20));
        p.clamp(4);
        assert_eq!((p.page, p.expanded, p.range(4)), (0, false, 0..4));
        assert_eq!(Pager::pages(0), 1);
    }

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
