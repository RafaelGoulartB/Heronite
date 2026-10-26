//! The Winetricks page of a game's settings: what the prefix already has,
//! the components winetricks offers (searchable, by kind) and a one-click
//! install that runs winetricks quietly while its output streams in. The
//! winetricks window itself is still one button away.
//!
//! On the desktop it replaces the tab's content (with a way back); in console
//! mode its rows replace the tab's rows (B goes back). The catalog and the
//! installed list come from `heronite_launch::winetricks`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::settings::GameSettings;
use heronite_launch::wine_tools::{winetricks_path, WineTool};
use heronite_launch::winetricks::{self, Category, Verb};
use heronite_launch::PlanContext;

use super::parts::{column, heading, help, rows};
use super::{Notice, SettingsPanel};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Output lines kept while a component installs.
const LOG_LINES: usize = 300;
/// Rows drawn at once (the search narrows the rest).
const SHOWN: usize = 150;

pub(super) struct WinetricksPage {
    /// `None` while the script is read.
    pub(super) catalog: Option<Arc<Vec<Verb>>>,
    pub(super) installed: Vec<String>,
    pub(super) category: Category,
    pub(super) run: Option<Run>,
    /// Why nothing can be installed (no usable Wine...).
    pub(super) problem: Option<String>,
}

pub(super) struct Run {
    pub(super) verb: String,
    pub(super) log: Vec<String>,
    /// Process group of winetricks (0 until it started).
    pid: Arc<AtomicU32>,
    cancelled: bool,
}

enum Msg {
    Line(String),
    Done(Result<(), String>),
}

pub(super) fn category_label(c: Category) -> &'static str {
    match c {
        Category::Dlls => tr!("Components"),
        Category::Fonts => tr!("Fonts"),
        Category::Settings => tr!("Prefix settings"),
    }
}

impl SettingsPanel {
    pub(super) fn winetricks_page(&self) -> Option<&WinetricksPage> {
        self.winetricks.as_ref()
    }

    pub(super) fn winetricks_open(&self) -> bool {
        self.winetricks.is_some()
    }

    /// Where the game's Wine tools act: Steam's prefix for a Steam game, the
    /// game's own settings otherwise (`None`: not ready, e.g. Steam unread).
    fn winetricks_settings(&self) -> Option<Result<GameSettings, ()>> {
        if self.is_steam() {
            return self.steam_tool_settings().map(Ok);
        }
        Some(Err(()))
    }

    pub(super) fn open_winetricks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        let Some(steam) = self.winetricks_settings() else { return };
        self.set_input("wt.search", "", window, cx);
        self.notice = None;
        self.winetricks = Some(WinetricksPage { catalog: None, installed: Vec::new(), category: Category::Dlls, run: None, problem: None });
        self.console_reset();
        let (paths, home) = (self.paths.clone(), self.home.clone());
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let settings = match steam {
                        Ok(s) => s,
                        Err(()) => GameSettings::from_map(&heronite_data::game_config::load_game(&paths, &app, &home), &home),
                    };
                    let script = winetricks_path(&PlanContext::load(&paths));
                    let catalog = winetricks::catalog(std::path::Path::new(&script));
                    let installed = winetricks::installed(&settings.wine_prefix);
                    (catalog, installed)
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(page) = this.winetricks.as_mut() {
                    page.problem = loaded.0.is_empty().then(|| {
                        tr!("The winetricks list is not here yet (it comes with the Wine tools updates). You can still type a component's name.")
                            .to_string()
                    });
                    page.catalog = Some(Arc::new(loaded.0));
                    page.installed = loaded.1;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_winetricks(&mut self, cx: &mut Context<Self>) {
        if self.winetricks.as_ref().is_some_and(|p| p.run.is_some()) {
            return;
        }
        self.winetricks = None;
        self.console_reset();
        cx.notify();
    }

    pub(super) fn set_winetricks_category(&mut self, c: Category, cx: &mut Context<Self>) {
        if let Some(p) = self.winetricks.as_mut() {
            p.category = c;
        }
        self.console_reset();
        cx.notify();
    }

    /// Installs `verb` with `winetricks -q`, streaming its output.
    pub(super) fn winetricks_install(&mut self, verb: String, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        let Some(steam) = self.winetricks_settings() else { return };
        let Some(page) = self.winetricks.as_mut() else { return };
        if page.run.is_some() || verb.trim().is_empty() {
            return;
        }
        let pid = Arc::new(AtomicU32::new(0));
        page.run = Some(Run { verb: verb.clone(), log: Vec::new(), pid: pid.clone(), cancelled: false });
        self.notice = None;
        let (paths, home) = (self.paths.clone(), self.home.clone());
        let (tx, mut rx) = unbounded::<Msg>();
        let job_verb = verb.clone();
        std::thread::Builder::new()
            .name("winetricks".into())
            .spawn(move || {
                let line = |l: String| {
                    let _ = tx.unbounded_send(Msg::Line(l));
                };
                let res = crate::runtime::winetricks_install_blocking(&paths, &home, &app, steam, &job_verb, &pid, &|| false, &line);
                let _ = tx.unbounded_send(Msg::Done(res));
            })
            .ok();
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let done = matches!(msg, Msg::Done(_));
                let alive = this
                    .update(cx, |this, cx| {
                        this.on_winetricks_msg(msg, cx);
                        cx.notify();
                    })
                    .is_ok();
                if done || !alive {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn on_winetricks_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        let Some(page) = self.winetricks.as_mut() else { return };
        let Some(run) = page.run.as_mut() else { return };
        match msg {
            Msg::Line(line) => {
                run.log.push(line);
                if run.log.len() > LOG_LINES {
                    run.log.remove(0);
                }
            }
            Msg::Done(res) => {
                let run = page.run.take().expect("running");
                let verb = run.verb;
                self.notice = Some(match res {
                    _ if run.cancelled => Notice { text: tr!("{verb}: installation canceled.", verb).into(), error: true },
                    Ok(()) => Notice { text: tr!("{verb} installed in the prefix.", verb).into(), error: false },
                    Err(why) => Notice { text: tr!("{verb} could not be installed: {why}", verb, why).into(), error: true },
                });
                self.refresh_winetricks_installed(cx);
            }
        }
    }

    fn refresh_winetricks_installed(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        let Some(steam) = self.winetricks_settings() else { return };
        let (paths, home) = (self.paths.clone(), self.home.clone());
        cx.spawn(async move |this, cx| {
            let installed = cx
                .background_spawn(async move {
                    let prefix = match steam {
                        Ok(s) => s.wine_prefix,
                        Err(()) => GameSettings::from_map(&heronite_data::game_config::load_game(&paths, &app, &home), &home).wine_prefix,
                    };
                    winetricks::installed(&prefix)
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(p) = this.winetricks.as_mut() {
                    p.installed = installed;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn cancel_winetricks(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.winetricks.as_mut().and_then(|p| p.run.as_mut()) else { return };
        let pid = run.pid.load(Ordering::SeqCst);
        if pid == 0 {
            return;
        }
        run.cancelled = true;
        // SAFETY: signals the process group winetricks leads (started with
        // `process_group(0)`), so its Wine children stop too.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        cx.notify();
    }

    /// The catalog entries of the current kind matching the search, and the
    /// typed name when it is not a known verb (it can still be installed).
    pub(super) fn winetricks_matches(&self, cx: &App) -> (Vec<Verb>, Option<String>) {
        let Some(page) = &self.winetricks else { return (Vec::new(), None) };
        let query = self.input_text("wt.search", cx);
        let folded = heronite_data::fuzzy::fold(query.trim());
        let tokens = heronite_data::fuzzy::tokens(&folded);
        let catalog = page.catalog.clone().unwrap_or_default();
        let list: Vec<Verb> = catalog
            .iter()
            .filter(|v| (!tokens.is_empty() || v.category == page.category) && winetricks::matches(v, &tokens))
            .cloned()
            .collect();
        let typed = query.trim();
        let custom = (!typed.is_empty() && !typed.contains(char::is_whitespace) && !catalog.iter().any(|v| v.name == typed))
            .then(|| typed.to_string());
        (list, custom)
    }

    /// The desktop page (in place of the Wine / Proton tab).
    pub(super) fn winetricks_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(page) = &self.winetricks else { return div() };
        let running = page.run.as_ref().map(|r| r.verb.clone());
        let busy = running.is_some();
        let installed = page.installed.clone();
        let category = page.category;
        let loading = page.catalog.is_none();
        let problem = page.problem.clone();

        let top = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(self.outline_button("wt-back", tr!("Back"), !busy, cx, |this, _, cx| this.close_winetricks(cx)))
            .child(self.outline_button("wt-window", tr!("Open the winetricks window"), !busy, cx, |this, _, cx| {
                this.run_tool("winetricks", WineTool::Winetricks, cx)
            }));

        let have = if installed.is_empty() {
            rows().child(div().py(px(12.)).child(help(tr!("Nothing was installed with winetricks in this prefix yet."))))
        } else {
            rows().child(div().flex().flex_wrap().gap(px(6.)).py(px(12.)).children(installed.iter().map(|v| crate::ui::chip(v.clone()))))
        };

        let mut col = column().child(top).child(heading(tr!("Installed in this prefix"))).child(have.card());
        if let Some(verb) = &running {
            col = col.child(heading(tr!("Installing"))).child(rows().child(self.winetricks_progress(verb, cx)).card());
        }

        let weak = cx.entity().downgrade();
        let tabs = crate::ui::controls::tabs("wt-kind", &Category::ALL.map(|c| (c, category_label(c))), category, move |c, _, cx| {
            weak.update(cx, |this, cx| this.set_winetricks_category(c, cx)).ok();
        });
        let (list, custom) = self.winetricks_matches(cx);
        let searching = !self.input_text("wt.search", cx).trim().is_empty();
        let mut items = div().id("wt-list").flex().flex_col().max_h(px(420.)).overflow_y_scroll();
        if loading {
            items = items.child(div().py(px(12.)).child(help(tr!("Loading\u{2026}"))));
        }
        if let Some(name) = custom {
            items = items.child(self.winetricks_row(
                &Verb {
                    name: name.clone(),
                    category,
                    title: tr!("Not in the list: install it by name").to_string(),
                    publisher: String::new(),
                    year: String::new(),
                    manual: false,
                },
                false,
                busy,
                cx,
            ));
        }
        if !loading && list.is_empty() {
            items = items.child(div().py(px(12.)).child(help(tr!("No component matches the search."))));
        }
        let more = list.len().saturating_sub(SHOWN);
        for v in list.iter().take(SHOWN) {
            let have = installed.contains(&v.name);
            items = items.child(self.winetricks_row(v, have, busy, cx));
        }
        if more > 0 {
            items = items.child(div().py(px(10.)).child(help(&heronite_i18n::trn!(
                more,
                "{n} more: narrow the search",
                "{n} more: narrow the search"
            ))));
        }
        let search = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(12.))
            .child(div().flex_1().min_w_0().child(crate::ui::fields::input_box(&self.input("wt.search"), false)))
            .when(!searching, |d| d.child(tabs));
        col = col
            .child(heading(tr!("Available")))
            .child(rows().child(search).child(items).card())
            .children(problem.map(|p| help(&p)))
            .child(help(tr!(
                "Components install quietly in this game's prefix with its Wine version. Some take a few minutes; the game should be closed meanwhile."
            )));
        col
    }

    fn winetricks_progress(&self, verb: &str, cx: &mut Context<Self>) -> Div {
        let last = self
            .winetricks
            .as_ref()
            .and_then(|p| p.run.as_ref())
            .map(|r| r.log.iter().rev().take(6).rev().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .py(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Installing {verb}\u{2026}", verb)))
                    .child(self.outline_button("wt-cancel", tr!("Cancel"), true, cx, |this, _, cx| this.cancel_winetricks(cx))),
            )
            .child(crate::services::widgets::busy_bar("wt-sweep", 6.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_size(px(11.5))
                    .line_height(px(15.))
                    .font_family(theme::FONT_MONO)
                    .text_color(theme::text_dim())
                    .children(last.into_iter().map(|l| div().truncate().child(l))),
            )
    }

    fn winetricks_row(&self, v: &Verb, have: bool, busy: bool, cx: &mut Context<Self>) -> Div {
        let verb = v.name.clone();
        let mut sub = v.title.clone();
        let meta: Vec<&str> = [v.year.as_str(), v.publisher.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
        if !meta.is_empty() {
            sub = format!("{sub} \u{b7} {}", meta.join(" \u{b7} "));
        }
        let right: AnyElement = if have {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(12.5))
                .text_color(theme::success())
                .child(icon(Icon::Check, px(15.), theme::success()))
                .child(tr!("Installed"))
                .into_any_element()
        } else {
            crate::ui::controls::action_button(
                SharedString::from(format!("wt-i-{verb}")),
                crate::ui::controls::ButtonKind::Glass,
                tr!("Install").to_string(),
                !busy,
                cx.listener(move |this, _, _, cx| this.winetricks_install(verb.clone(), cx)),
            )
            .into_any_element()
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(theme::ink_a(0.06))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.5)).text_color(theme::text_primary()).child(v.name.clone()))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(sub))
                    .when(v.manual, |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::danger_text())
                                .child(tr!("Needs files downloaded by hand: use the winetricks window")),
                        )
                    }),
            )
            .child(div().flex_none().child(right))
    }
}
