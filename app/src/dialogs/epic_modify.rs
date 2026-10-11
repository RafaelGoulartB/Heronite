//! "Modify Installation" of an installed Epic game (port of Heroic's
//! `ModifyInstallModal` -> `DLCList`): the DLCs the account owns for the game
//! with their download size, and for each one install (queued like any
//! install, into the game's folder), uninstall or cancel the install in the
//! queue.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_epic::cmd::Legendary;
use heronite_epic::info::{self, Dlc};
use heronite_epic::manager::DmState;
use heronite_epic::util::file_size;
use serde_json::Value;

use super::DialogEvent;
use crate::icons::Icon;
use crate::services::downloads::{library_entry, Downloads, DownloadsEvent, QueueState};
use crate::services::{self, widgets::fmt_percent};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{meta_text, paragraph};
use crate::ui::icon;

struct Row {
    dlc: Dlc,
    /// Library entry of the DLC (its `GameInfo`); `None` until loaded.
    info: Option<Value>,
    installed: bool,
    /// Bytes to download; `None` while `legendary info` runs.
    size: Option<f64>,
}

pub struct EpicModifyDialog {
    data_paths: DataPaths,
    svc: Entity<Downloads>,
    app_name: String,
    title: String,
    platform: String,
    install_path: String,
    rows: Vec<Row>,
    /// `legendary info` of the game has been answered.
    loaded: bool,
    error: Option<String>,
    /// DLC waiting for the answer to "Do you want to uninstall...".
    confirm: Option<usize>,
    _subs: Vec<Subscription>,
    console_list: crate::console::list::ListNav,
}

impl EventEmitter<DialogEvent> for EpicModifyDialog {}

impl EpicModifyDialog {
    /// `game` is the `GameInfo` of the installed base game.
    pub fn new(data_paths: DataPaths, game: Value, cx: &mut Context<Self>) -> Self {
        let svc = services::downloads(cx);
        let mut subs = vec![cx.observe(&svc, |_, _, cx| cx.notify())];
        // An install or uninstall that finished changes what is installed.
        subs.push(cx.subscribe(&svc, |this, _, ev: &DownloadsEvent, cx| {
            if matches!(ev, DownloadsEvent::LibraryChanged) {
                this.reload_installed(cx);
            }
        }));
        let text = |ptr: &str| game.pointer(ptr).and_then(Value::as_str).unwrap_or_default().to_string();
        let app_name = text("/app_name");
        let mut this = Self {
            data_paths,
            svc,
            title: text("/title"),
            platform: Some(text("/install/platform")).filter(|p| !p.is_empty()).unwrap_or_else(|| "Windows".into()),
            install_path: text("/install/install_path"),
            app_name,
            rows: Vec::new(),
            loaded: false,
            error: None,
            confirm: None,
            _subs: subs,
            console_list: Default::default(),
        };
        this.load(cx);
        this
    }

    /// The DLC list from `legendary info`, then each DLC's library entry and size.
    fn load(&mut self, cx: &mut Context<Self>) {
        let (data_paths, app, platform) = (self.data_paths.clone(), self.app_name.clone(), self.platform.clone());
        cx.spawn(async move |this, cx| {
            let first = {
                let (data_paths, app, platform) = (data_paths.clone(), app.clone(), platform.clone());
                cx.background_spawn(async move {
                    let lg = Legendary::locate(&data_paths)?;
                    info::install_info(&data_paths, &lg, &app, &platform)
                })
                .await
            };
            let dlcs = match first {
                Ok(i) => i.owned_dlc(),
                Err(err) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(format!("{err:#}"));
                        this.loaded = true;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |this, cx| {
                this.rows = dlcs.iter().cloned().map(|dlc| Row { dlc, info: None, installed: false, size: None }).collect();
                this.loaded = true;
                cx.notify();
            })
            .ok();
            // Like Heroic, one DLC after the other.
            for dlc in dlcs {
                let (data_paths, platform) = (data_paths.clone(), platform.clone());
                let name = dlc.app_name.clone();
                let (entry, size) = cx
                    .background_spawn(async move {
                        let entry = library_entry(&data_paths, &name);
                        let size = Legendary::locate(&data_paths)
                            .and_then(|lg| info::install_info(&data_paths, &lg, &name, &platform))
                            .map(|i| i.download_size())
                            .unwrap_or(0.);
                        (entry, size)
                    })
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        if let Some(row) = this.rows.iter_mut().find(|r| r.dlc.app_name == dlc.app_name) {
                            row.installed = entry.as_ref().and_then(|e| e.get("is_installed")).and_then(Value::as_bool).unwrap_or(false);
                            row.info = entry;
                            row.size = Some(size);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        })
        .detach();
    }

    /// Reads which DLCs are installed again after the library changed.
    fn reload_installed(&mut self, cx: &mut Context<Self>) {
        let data_paths = self.data_paths.clone();
        let names: Vec<String> = self.rows.iter().map(|r| r.dlc.app_name.clone()).collect();
        cx.spawn(async move |this, cx| {
            let states = cx
                .background_spawn(async move {
                    names
                        .into_iter()
                        .map(|n| {
                            let entry = library_entry(&data_paths, &n);
                            let installed = entry.as_ref().and_then(|e| e.get("is_installed")).and_then(Value::as_bool).unwrap_or(false);
                            (n, entry, installed)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                for (name, entry, installed) in states {
                    if let Some(row) = this.rows.iter_mut().find(|r| r.dlc.app_name == name) {
                        row.installed = installed;
                        if entry.is_some() {
                            row.info = entry;
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn install(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(i) else { return };
        let Some(info) = row.info.clone() else { return };
        if self.install_path.is_empty() {
            return;
        }
        let size = row.size.filter(|s| *s > 0.).map(file_size);
        let (path, platform) = (self.install_path.clone(), self.platform.clone());
        self.svc.update(cx, |s, cx| s.install_dlc(info, &path, &platform, size, cx));
        // Heroic closes the modal once the install is queued.
        cx.emit(DialogEvent::Close);
    }

    fn uninstall(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(i) else { return };
        let app = row.dlc.app_name.clone();
        self.confirm = None;
        self.svc.update(cx, |s, cx| s.uninstall(&app, false, false, cx));
        cx.notify();
    }

    /// Takes the DLC out of the queue (or stops its running download).
    fn cancel(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(i) else { return };
        let app = row.dlc.app_name.clone();
        self.svc.update(cx, |s, _| {
            let first = s.queued(&app).is_some_and(|(index, _)| index == 0);
            if first && s.queue_state(&app) == Some(QueueState::Downloading) {
                s.cancel_current(false);
            } else {
                s.remove_from_queue(&app);
            }
        });
        cx.notify();
    }

    fn action_cell(&mut self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let (queue, busy, downloading) = {
            let s = self.svc.read(cx);
            let app = &self.rows[i].dlc.app_name;
            (s.queue_state(app), s.busy_of(app).cloned(), s.snapshot().state == DmState::Running)
        };
        let row = &self.rows[i];
        let id = |what: &str| SharedString::from(format!("epic-modify-{what}-{i}"));
        let label = |text: String| div().text_size(px(13.33)).text_color(white()).child(text);
        if let Some(b) = busy {
            return div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(Icon::FaSpinner, px(14.), crate::theme::text_primary()))
                .child(label(format!("{}...", b.label)))
                .into_any_element();
        }
        if let Some(state) = queue {
            let progress = self.svc.read(cx).progress_of(&row.dlc.app_name).map(|p| p.percent);
            let text = match (state, progress) {
                (QueueState::Downloading, Some(p)) => format!("Installing {}", fmt_percent(p)),
                (QueueState::Downloading, None) => "Installing...".to_string(),
                (QueueState::Paused, _) => "Paused".to_string(),
                (QueueState::Waiting, _) => "Queued".to_string(),
            };
            return div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(label(text))
                .child(button(id("cancel"), ButtonKind::Glass, "Cancel").on_click(cx.listener(move |this, _, _, cx| this.cancel(i, cx))))
                .into_any_element();
        }
        if row.installed {
            // Legendary cannot uninstall while it is downloading something.
            return if downloading {
                label("Not while installing".to_string()).opacity(0.7).into_any_element()
            } else {
                button(id("uninstall"), ButtonKind::Danger, "Uninstall")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm = Some(i);
                        cx.notify();
                    }))
                    .into_any_element()
            };
        }
        if row.info.is_none() || row.size.is_none() {
            return icon(Icon::FaSpinner, px(14.), crate::theme::text_primary()).into_any_element();
        }
        button(id("install"), ButtonKind::Primary, "Install")
            .on_click(cx.listener(move |this, _, _, cx| this.install(i, cx)))
            .into_any_element()
    }
}

impl Render for EpicModifyDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            let rows = self.console_rows(cx);
            let message = if let Some(e) = &self.error {
                e.lines().next().unwrap_or("").to_string()
            } else if !self.loaded {
                "Getting the DLCs\u{2026}".into()
            } else if let Some(i) = self.confirm.filter(|i| *i < self.rows.len()) {
                format!("Uninstall \u{201c}{}\u{201d} (DLC)?", self.rows[i].dlc.title)
            } else if self.rows.is_empty() {
                "No DLCs found".into()
            } else if self.install_path.is_empty() {
                "The install folder of the game is unknown, DLCs cannot be installed.".into()
            } else {
                String::new()
            };
            let title = format!("Modify installation \u{b7} {}", self.title);
            return crate::console::list::simple_dialog(&mut self.console_list, "console-epic-modify", title, &message, None, &rows);
        }
        let mut body = div().flex().flex_col().gap(px(8.));
        if let Some(err) = &self.error {
            body =
                body.child(div().text_size(px(14.)).text_color(theme::danger_text()).child(err.lines().next().unwrap_or("").to_string()));
        } else if !self.loaded {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(Icon::FaSpinner, px(16.), crate::theme::text_primary()))
                    .child(div().text_size(px(15.)).text_color(crate::theme::text_primary()).child("Getting the DLCs...")),
            );
        } else if self.rows.is_empty() {
            body = body.child(paragraph("No DLCs found"));
        } else {
            let head = |text: &'static str| {
                div().text_size(px(13.33)).font_weight(theme::W_BOLD).text_color(crate::theme::text_primary()).child(text)
            };
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pb(px(6.))
                    .border_b_1()
                    .border_color(theme::white_a(0.12))
                    .child(head("Title").flex_1())
                    .child(head("Size").w(px(90.)).flex_none())
                    .child(head("Actions").w(px(190.)).flex_none().flex().justify_end()),
            );
            for i in 0..self.rows.len() {
                let size = match self.rows[i].size {
                    Some(s) => file_size(s),
                    None => "...".to_string(),
                };
                let cell = self.action_cell(i, cx);
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .py(px(5.))
                        .child(div().flex_1().text_size(px(15.)).text_color(white()).child(self.rows[i].dlc.title.clone()))
                        .child(div().w(px(90.)).flex_none().text_size(px(14.)).text_color(white()).child(size))
                        .child(div().w(px(190.)).flex_none().flex().justify_end().child(cell)),
                );
            }
            if let Some(i) = self.confirm.filter(|i| *i < self.rows.len()) {
                let title = self.rows[i].dlc.title.clone();
                body = body.child(
                    div()
                        .mt(px(10.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .p(px(12.))
                        .rounded(px(4.))
                        .bg(theme::white_a(0.06))
                        .child(paragraph(format!("Do you want to uninstall \"{title}\" (DLC)?")))
                        .child(
                            div()
                                .flex()
                                .gap(px(12.))
                                .child(button("epic-modify-confirm-no", ButtonKind::Ghost, "Cancel").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.confirm = None;
                                        cx.notify();
                                    },
                                )))
                                .child(
                                    button("epic-modify-confirm-yes", ButtonKind::Danger, "Uninstall")
                                        .on_click(cx.listener(move |this, _, _, cx| this.uninstall(i, cx))),
                                ),
                        ),
                );
            }
            if self.install_path.is_empty() {
                body = body.child(meta_text("The install folder of the game is unknown, DLCs cannot be installed."));
            }
        }
        let footer =
            button("epic-modify-close", ButtonKind::Primary, "Close").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)));
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "epic-modify".into(),
                title: format!("Modify Installation - {}", self.title).into(),
                width: 680.,
                height: None,
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}

impl EpicModifyDialog {
    fn console_rows(&self, cx: &App) -> Vec<crate::console::list::Row> {
        use crate::console::list::Row as CRow;
        if self.confirm.is_some_and(|i| i < self.rows.len()) {
            return vec![CRow::action("no", "Cancel"), CRow::action("yes", "Uninstall").danger()];
        }
        let s = self.svc.read(cx);
        let downloading = s.snapshot().state == DmState::Running;
        let mut rows = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            let app = &r.dlc.app_name;
            let size = r.size.map(file_size).unwrap_or_else(|| "\u{2026}".into());
            let state = if let Some(b) = s.busy_of(app) {
                format!("{}\u{2026}", b.label)
            } else if let Some(q) = s.queue_state(app) {
                match (q, s.progress_of(app).map(|p| p.percent)) {
                    (QueueState::Downloading, Some(p)) => format!("Installing {} \u{b7} A cancels", fmt_percent(p)),
                    (QueueState::Downloading, None) => "Installing\u{2026} \u{b7} A cancels".into(),
                    (QueueState::Paused, _) => "Paused \u{b7} A cancels".into(),
                    (QueueState::Waiting, _) => "Queued \u{b7} A cancels".into(),
                }
            } else if r.installed {
                if downloading {
                    format!("Installed \u{b7} {size}")
                } else {
                    format!("Installed \u{b7} {size} \u{b7} A uninstalls")
                }
            } else {
                format!("{size} \u{b7} A installs")
            };
            rows.push(CRow::info(SharedString::from(format!("d{i}")), r.dlc.title.clone(), state));
        }
        rows.push(CRow::action("close", "Close"));
        rows
    }
}

impl crate::console::PadTarget for EpicModifyDialog {
    fn on_pad(&mut self, pad: crate::console::Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::console::list::Outcome;
        if pad == crate::console::Pad::B && self.confirm.is_some() {
            self.confirm = None;
            cx.notify();
            return true;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => match id.as_ref() {
                "close" => cx.emit(DialogEvent::Close),
                "no" => self.confirm = None,
                "yes" => {
                    if let Some(i) = self.confirm {
                        self.uninstall(i, cx);
                    }
                }
                id => {
                    if let Some(i) = id.strip_prefix('d').and_then(|n| n.parse::<usize>().ok()) {
                        let (queued, busy, downloading) = {
                            let s = self.svc.read(cx);
                            let app = &self.rows[i].dlc.app_name;
                            (s.queue_state(app).is_some(), s.busy_of(app).is_some(), s.snapshot().state == DmState::Running)
                        };
                        if busy {
                        } else if queued {
                            self.cancel(i, cx);
                        } else if self.rows[i].installed {
                            if !downloading {
                                self.confirm = Some(i);
                                self.console_list.reset();
                            }
                        } else if self.rows[i].info.is_some() && self.rows[i].size.is_some() {
                            self.install(i, cx);
                        }
                    }
                }
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<crate::console::Hint> {
        use crate::console::{hint, Btn};
        vec![hint(Btn::A, "Select"), hint(Btn::B, if self.confirm.is_some() { "Cancel" } else { "Close" })]
    }
}
