//! Wine Manager in console mode: LB / RB switch the source (Proton-CachyOS,
//! GE-Proton), each version is a row with its state on the right, A opens
//! what can be done with it (install, update, cancel, remove after a
//! confirmation). Y searches, X refreshes the release lists.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_wine::{JobKind, Repository, WineVersionInfo};

use super::render::size_text;
use super::WineManagerPage;
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::osk::{Osk, OskEvent};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;

impl WineManagerPage {
    fn console_versions(&self) -> Vec<WineVersionInfo> {
        let kind = self.repo.kind();
        let q = self.console_query.to_lowercase();
        self.releases
            .iter()
            .filter(|r| r.kind == kind && r.downloadable())
            .filter(|r| q.is_empty() || r.version.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    fn console_rows(&self) -> Vec<Row> {
        let mut rows = vec![
            Row::toggle("steam", "Install Proton into Steam", self.download_to_steam)
                .desc("New Proton builds also go to Steam's compatibility tools"),
            Row::action("refresh", if self.manager.refreshing() { "Fetching releases\u{2026}" } else { "Refresh the release lists" })
                .icon(Icon::Refresh)
                .enabled(!self.manager.refreshing() && !self.offline),
        ];
        if !self.console_query.is_empty() {
            rows.push(
                Row::action("clear-search", format!("Search: \u{201c}{}\u{201d}", self.console_query))
                    .desc("A clears the search")
                    .icon(Icon::FaSearch),
            );
        }
        rows.push(Row::header(self.repo.kind()));
        for r in self.console_versions() {
            let job = self.job(&r.version);
            let state = match &job {
                Some(_) => size_text(job.as_ref(), &r).to_string(),
                None if r.installed() && r.has_update() => format!("Update available \u{b7} {}", size_text(None, &r)),
                None if r.installed() => format!("Installed \u{b7} {}", size_text(None, &r)),
                None => format!("{} \u{b7} {}", r.date, size_text(None, &r)),
            };
            let row = Row::info(SharedString::from(format!("v:{}", r.version)), r.version.clone(), state);
            rows.push(if r.installed() { row.icon(Icon::Check) } else { row.icon(Icon::CloudDownload) });
        }
        rows
    }

    fn open_version_sheet(&mut self, version: &str, cx: &mut Context<Self>) {
        let Some(r) = self.releases.iter().find(|r| r.version == version).cloned() else { return };
        let job = self.job(version);
        let mut rows = Vec::new();
        match &job {
            Some(j) if j.kind != JobKind::Remove => rows.push(Row::action("cancel", "Cancel the download").icon(Icon::Close)),
            Some(_) => {}
            None if r.installed() => {
                if r.has_update() {
                    rows.push(Row::action("install", "Update").icon(Icon::Update));
                }
                rows.push(Row::action("remove", "Remove\u{2026}").icon(Icon::FaTrash).danger());
            }
            None => rows.push(Row::action("install", "Install").icon(Icon::Download)),
        }
        if rows.is_empty() {
            return;
        }
        let message = Some(format!("{} \u{b7} {}", r.kind, size_text(job.as_ref(), &r)));
        let sheet = cx.new(|_| Sheet::new(r.version.clone(), message, rows));
        let sub = cx.subscribe(&sheet, move |this, sheet, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            match id.as_ref() {
                "install" => {
                    this.layer.close();
                    this.manager.start_install(r.clone());
                }
                "cancel" => {
                    this.layer.close();
                    this.manager.cancel(&r.version);
                }
                "remove" => sheet.update(cx, |s, cx| {
                    s.set_message(Some(format!("Remove {}? Games set to use it fall back to the default.", r.version)));
                    s.set_rows(vec![Row::action("keep", "Keep it"), Row::action("remove-yes", "Remove").icon(Icon::FaTrash).danger()]);
                    cx.notify();
                }),
                "remove-yes" => {
                    this.layer.close();
                    this.manager.start_remove(r.clone());
                }
                _ => this.layer.close(),
            }
            cx.notify();
        });
        self.layer.open(&sheet, sub);
    }

    fn open_console_search(&mut self, cx: &mut Context<Self>) {
        let osk = cx.new(|_| Osk::new("Search versions", &self.console_query));
        let sub = cx.subscribe(&osk, |this, _, ev: &OskEvent, cx| {
            if let OskEvent::Done(text) = ev {
                this.console_query = text.trim().to_string();
                this.console_list.reset();
            }
            this.layer.close();
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows();
        let tab = Repository::LINUX.iter().position(|r| *r == self.repo).unwrap_or(0);
        let tabs: Vec<SharedString> = Repository::LINUX.iter().map(|r| SharedString::from(r.kind())).collect();
        let list = self.console_list.render("console-wine", &rows, !self.layer.is_open());
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(u(36.))
                    .h(u(76.))
                    .px(u(46.))
                    .child(kit::wordmark("WINE MANAGER"))
                    .child(kit::tabs(&tabs, tab))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(div().px(u(46.)).pb(u(14.)).max_w(u(900.)).child(kit::body_text(self.repo.explanation())))
            .child(div().flex().flex_col().flex_1().min_h_0().px(u(46.)).pb(u(16.)).child(
                div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(if self.loaded {
                    list.flex_1().min_h_0().into_any_element()
                } else {
                    kit::body_text("Reading the release lists\u{2026}").into_any_element()
                }),
            ))
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for WineManagerPage {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let tab = Repository::LINUX.iter().position(|r| *r == self.repo).unwrap_or(0);
        let n = Repository::LINUX.len();
        match pad {
            Pad::LB | Pad::RB => {
                let next = if pad == Pad::RB { (tab + 1) % n } else { (tab + n - 1) % n };
                self.repo = Repository::LINUX[next];
                self.console_list.reset();
            }
            Pad::Y => self.open_console_search(cx),
            Pad::X if !self.manager.refreshing() && !self.offline => {
                self.manager.start_refresh(Repository::LINUX.to_vec());
            }
            _ => {
                let rows = self.console_rows();
                match self.console_list.on_pad(pad, &rows) {
                    Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "steam" => self.set_download_to_steam(on, cx),
                    Outcome::Activate(id) => match id.as_ref() {
                        "refresh" => {
                            self.manager.start_refresh(Repository::LINUX.to_vec());
                        }
                        "clear-search" => self.console_query.clear(),
                        v if v.starts_with("v:") => self.open_version_sheet(&v[2..], cx),
                        _ => {}
                    },
                    Outcome::Ignored => return false,
                    _ => {}
                }
            }
        }
        cx.notify();
        true
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        vec![hint(Btn::A, "Select"), hint(Btn::Bumpers, "Source"), hint(Btn::Y, "Search"), hint(Btn::X, "Refresh")]
    }
}
