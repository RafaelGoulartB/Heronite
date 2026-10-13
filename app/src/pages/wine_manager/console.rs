//! Wine Manager in console mode: LB / RB switch the source, each version is a row with its state on the right, A opens
//! what can be done with it (install, update, cancel, remove after a
//! confirmation). Y searches, X refreshes the release lists.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_wine::{CheckMode, JobKind, Repository, WineVersionInfo};

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

    fn console_prefix_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::action("pfx-new", tr!("New prefix\u{2026}")).icon(Icon::Add)];
        if self.prefixes.list.is_none() {
            rows.push(Row::info("pfx-loading", tr!("Looking for prefixes\u{2026}"), ""));
        }
        for (i, p) in self.visible_prefixes().iter().enumerate() {
            let state = format!("{} \u{b7} {}", self.prefix_size(p), self.prefix_users(p));
            let folder = p.path.parent().map(|d| d.display().to_string()).unwrap_or_default();
            rows.push(Row::info(SharedString::from(format!("pfx:{i}")), p.name.clone(), state).desc(folder).icon(Icon::Folder));
        }
        rows
    }

    fn open_prefix_sheet(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(p) = self.visible_prefixes().get(index).cloned() else { return };
        let mut rows = Vec::new();
        if p.kind != heronite_wine::prefixes::PrefixKind::NotCreated && self.prefixes_copying().is_none() {
            rows.push(Row::action("dup", tr!("Duplicate\u{2026}")).icon(Icon::ContentCopy));
        }
        if self.prefixes_copying().as_ref() == Some(&p.path) {
            rows.push(Row::action("cancel-copy", tr!("Cancel the copy")).icon(Icon::Close));
        }
        if !p.default && p.kind != heronite_wine::prefixes::PrefixKind::NotCreated {
            rows.push(Row::action("trash", tr!("Move to trash\u{2026}")).icon(Icon::FaTrash).danger());
        }
        if rows.is_empty() {
            return;
        }
        let message = Some(format!("{} \u{b7} {}", p.path.display(), self.prefix_users(&p)));
        let sheet = cx.new(|_| Sheet::new(p.name.clone(), message, rows));
        let sub = cx.subscribe(&sheet, move |this, sheet, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            match id.as_ref() {
                "dup" => {
                    this.layer.close();
                    let source = p.path.clone();
                    let osk = cx.new(|_| Osk::new(tr!("Name of the copy"), &tr!("{name} copy", name = p.name)));
                    let sub = cx.subscribe(&osk, move |this, _, ev: &OskEvent, cx| {
                        if let OskEvent::Done(text) = ev {
                            let name = text.trim().to_string();
                            if !name.is_empty() {
                                this.duplicate_prefix(source.clone(), name, cx);
                            }
                        }
                        this.layer.close();
                        cx.notify();
                    });
                    this.layer.open(&osk, sub);
                }
                "cancel-copy" => {
                    this.layer.close();
                    this.cancel_prefix_copy();
                }
                "trash" => sheet.update(cx, |s, cx| {
                    s.set_message(Some(tr!(
                        "Move {name} to the trash? Games set to it get a new, empty prefix the next time they start.",
                        name = p.name
                    )));
                    s.set_rows(vec![
                        Row::action("keep", tr!("Keep it")),
                        Row::action("trash-yes", tr!("Move to trash")).icon(Icon::FaTrash).danger(),
                    ]);
                    cx.notify();
                }),
                "trash-yes" => {
                    this.layer.close();
                    this.trash_prefix(p.path.clone(), cx);
                }
                _ => this.layer.close(),
            }
            cx.notify();
        });
        self.layer.open(&sheet, sub);
    }

    fn open_new_prefix(&mut self, cx: &mut Context<Self>) {
        let osk = cx.new(|_| Osk::new(tr!("New prefix name"), ""));
        let sub = cx.subscribe(&osk, |this, _, ev: &OskEvent, cx| {
            if let OskEvent::Done(text) = ev {
                let name = text.trim().to_string();
                if !name.is_empty() {
                    this.create_prefix(name, cx);
                }
            }
            this.layer.close();
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    fn console_rows(&self) -> Vec<Row> {
        if self.show_prefixes {
            return self.console_prefix_rows();
        }
        let mut rows = vec![
            Row::toggle("steam", tr!("Install Proton into Steam"), self.download_to_steam)
                .desc(tr!("New Proton builds also go to Steam's compatibility tools")),
            Row::action("refresh", if self.checking() { tr!("Fetching releases\u{2026}") } else { tr!("Refresh the release list") })
                .icon(Icon::Refresh)
                .enabled(!self.checking() && !self.offline),
        ];
        if !self.console_query.is_empty() {
            rows.push(
                Row::action("clear-search", tr!("Search: \u{201c}{query}\u{201d}", query = self.console_query))
                    .desc(tr!("A clears the search"))
                    .icon(Icon::FaSearch),
            );
        }
        rows.push(Row::header(self.repo.kind()));
        for r in self.console_versions() {
            let job = self.job(&r.version);
            let state = match &job {
                Some(_) => size_text(job.as_ref(), &r).to_string(),
                None if r.installed() && r.has_update() => tr!("Update available \u{b7} {size}", size = size_text(None, &r)),
                None if r.installed() => tr!("Installed \u{b7} {size}", size = size_text(None, &r)),
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
            Some(j) if j.kind != JobKind::Remove => rows.push(Row::action("cancel", tr!("Cancel the download")).icon(Icon::Close)),
            Some(_) => {}
            None if r.installed() => {
                if r.has_update() {
                    rows.push(Row::action("install", tr!("Update")).icon(Icon::Update));
                }
                rows.push(Row::action("remove", tr!("Remove\u{2026}")).icon(Icon::FaTrash).danger());
            }
            None => rows.push(Row::action("install", tr!("Install")).icon(Icon::Download)),
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
                    s.set_message(Some(tr!("Remove {version}? Games set to use it fall back to the default.", version = r.version)));
                    s.set_rows(vec![
                        Row::action("keep", tr!("Keep it")),
                        Row::action("remove-yes", tr!("Remove")).icon(Icon::FaTrash).danger(),
                    ]);
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
        let osk = cx.new(|_| Osk::new(tr!("Search versions"), &self.console_query));
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
        let offered = Repository::offered();
        let tab = if self.show_prefixes { offered.len() } else { offered.iter().position(|r| *r == self.repo).unwrap_or(0) };
        let mut tabs: Vec<SharedString> = offered.iter().map(|r| SharedString::from(r.kind())).collect();
        tabs.push(tr!("Prefixes").into());
        let about = if self.show_prefixes {
            tr!("Where your games keep their Windows files and saves. A opens what can be done with a prefix; choose a game's prefix in its Wine settings.")
        } else {
            self.repo.explanation()
        };
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
                    .child(kit::wordmark(tr!("WINE MANAGER")))
                    .child(kit::tabs(&tabs, tab))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(div().px(u(46.)).pb(u(14.)).max_w(u(900.)).child(kit::body_text(about)))
            .child(div().flex().flex_col().flex_1().min_h_0().px(u(46.)).pb(u(16.)).child(
                div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(if self.loaded {
                    list.flex_1().min_h_0().into_any_element()
                } else {
                    kit::body_text(tr!("Reading the release lists\u{2026}")).into_any_element()
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
        let offered = Repository::offered();
        // The sources, then the Prefixes view.
        let tab = if self.show_prefixes { offered.len() } else { offered.iter().position(|r| *r == self.repo).unwrap_or(0) };
        let n = offered.len() + 1;
        match pad {
            Pad::LB | Pad::RB => {
                let next = if pad == Pad::RB { (tab + 1) % n } else { (tab + n - 1) % n };
                match offered.get(next) {
                    Some(repo) => self.select_source(*repo),
                    None => self.select_prefixes(cx),
                }
                self.console_list.reset();
            }
            Pad::Y => self.open_console_search(cx),
            Pad::X if self.show_prefixes => self.load_prefixes(cx),
            Pad::X if !self.checking() && !self.offline => self.check(self.repo, CheckMode::Manual),
            _ => {
                let rows = self.console_rows();
                match self.console_list.on_pad(pad, &rows) {
                    Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "steam" => self.set_download_to_steam(on, cx),
                    Outcome::Activate(id) => match id.as_ref() {
                        "refresh" => self.check(self.repo, CheckMode::Manual),
                        "pfx-new" => self.open_new_prefix(cx),
                        v if v.starts_with("pfx:") => {
                            if let Ok(i) = v[4..].parse::<usize>() {
                                self.open_prefix_sheet(i, cx);
                            }
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
        vec![hint(Btn::A, tr!("Select")), hint(Btn::Bumpers, tr!("Source")), hint(Btn::Y, tr!("Search")), hint(Btn::X, tr!("Refresh"))]
    }
}
