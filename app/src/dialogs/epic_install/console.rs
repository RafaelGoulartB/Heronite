//! The Epic install / import dialog in console mode: sizes and free space,
//! the folder (picked with the console browser), components and DLCs as
//! switches, the Wine choices with ◀ ▶, then Install (or Import).

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::util::file_size_local;
use heronite_i18n::tr;

use super::super::{text_of, DialogEvent};
use super::{EpicInstallDialog, Mode};
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

impl EpicInstallDialog {
    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let mut rows = Vec::new();
        let importing = self.mode == Mode::Import;
        if let (Some(tp), Some((_, detail))) = (&self.third_party, self.third_party_notice()) {
            let (ic, store) = match tp {
                Ok(p) => (Icon::Check, p.label().to_string()),
                Err(name) => (Icon::Warning, tr!("{name} (not supported)", name)),
            };
            rows.push(Row::info("third-party", tr!("Managed by"), store).icon(ic).desc(detail));
        } else if !importing {
            let sizes = match (&self.info, &self.info_error) {
                (Some(_), _) => tr!(
                    "Download {download}  \u{b7}  On disk {disk}",
                    download = file_size_local(self.download_bytes()),
                    disk = file_size_local(self.disk_bytes())
                ),
                (None, Some(e)) => tr!("Could not read the sizes: {error}", error = e.lines().next().unwrap_or("")),
                (None, None) => tr!("Getting the download and install size\u{2026}").into(),
            };
            rows.push(Row::info("sizes", tr!("Size"), sizes).icon(Icon::FaHardDrive));
            rows.push(
                Row::text("path", tr!("Install folder"), text_of(&self.path, cx)).icon(Icon::FolderOpen).desc(
                    self.space
                        .as_ref()
                        .map(|s| {
                            if s.writable {
                                tr!("Free space: {space}", space = s.message)
                            } else {
                                tr!("Free space: {space} (may not be writable)", space = s.message)
                            }
                        })
                        .unwrap_or_default(),
                ),
            );
            if !self.sdl.is_empty() {
                rows.push(Row::header(tr!("Components")));
                for (i, (sdl, on)) in self.sdl.iter().enumerate() {
                    rows.push(
                        Row::toggle(SharedString::from(format!("sdl{i}")), sdl.name.clone(), sdl.required || *on)
                            .enabled(!sdl.required)
                            .desc(sdl.description.clone()),
                    );
                }
            }
            if !self.dlcs.is_empty() {
                rows.push(Row::header("DLC"));
                rows.push(Row::toggle("dlc-all", tr!("Install all DLCs"), self.dlcs.iter().all(|(_, on)| *on)));
                for (i, (dlc, on)) in self.dlcs.iter().enumerate() {
                    rows.push(Row::toggle(SharedString::from(format!("dlc{i}")), dlc.title.clone(), *on));
                }
            }
        } else {
            rows.push(
                Row::text("import-path", tr!("Game folder"), text_of(&self.import_path, cx))
                    .icon(Icon::FolderOpen)
                    .desc(tr!("The folder that holds the installed game files")),
            );
        }
        if self.linux_windows_game() {
            rows.push(Row::header("Wine"));
            rows.push(
                Row::toggle("shared", tr!("Use the shared Wine prefix"), self.shared_prefix)
                    .desc(tr!("Only if you know what you are doing: games can break each other's dependencies")),
            );
            if !self.shared_prefix {
                rows.push(Row::text("prefix", tr!("Wine prefix"), text_of(&self.prefix, cx)).icon(Icon::FolderOpen));
                if self.wine_list.is_empty() {
                    rows.push(Row::info(
                        "wine",
                        tr!("Wine version"),
                        if self.wine_loaded { crate::dialogs::no_wine_text(cx) } else { tr!("Looking for Wine and Proton\u{2026}").into() },
                    ));
                } else {
                    let names = self.wine_list.iter().map(|w| SharedString::from(w.name.clone())).collect();
                    rows.push(Row::choice("wine", tr!("Wine version"), names, self.wine_pick).icon(Icon::WineBar));
                }
            }
        }
        rows.push(Row::header(""));
        if importing {
            rows.push(Row::action("import", tr!("Import")).icon(Icon::Check).enabled(!text_of(&self.import_path, cx).is_empty()));
            rows.push(Row::action("mode", tr!("Download and install instead")).icon(Icon::Download));
        } else {
            let label = if self.third_party.is_none() && self.not_enough() { tr!("Force install") } else { tr!("Install") };
            rows.push(Row::action("install", label).icon(Icon::Download).enabled(self.can_install(cx)));
            if self.third_party.is_none() {
                rows.push(Row::action("mode", tr!("Import an existing install instead")).icon(Icon::FolderOpen));
            }
        }
        rows.push(Row::action("cancel", tr!("Cancel")));
        rows
    }

    fn pick_folder(&mut self, input: Entity<InputState>, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        let start = std::path::PathBuf::from(input.read(cx).value().to_string());
        let browser = cx.new(|cx| Browser::new(title.to_string(), PickKind::Folder, Some(&start), cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| {
            if let BrowserEvent::Picked(p) = ev {
                let v = p.display().to_string();
                input.update(cx, |s, cx| s.set_value(v, window, cx));
                this.check_space(cx);
            }
            this.layer.close();
            cx.notify();
        });
        self.layer.open(&browser, sub);
    }

    pub(super) fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let kicker = if self.mode == Mode::Import { tr!("Import game") } else { tr!("Install") };
        let list = self.console_list.render("console-epic-install", &rows, !self.layer.is_open());
        div()
            .absolute()
            .inset_0()
            .bg(theme::console_bg())
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(u(76.))
                    .px(u(46.))
                    .child(kit::wordmark("EPIC GAMES"))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .px(u(46.))
                    .pb(u(16.))
                    .child(kit::kicker(kicker))
                    .child(kit::title(self.title.clone(), 40.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(u(46.))
                    .pb(u(76.))
                    .child(div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(list.flex_1().min_h_0())),
            )
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for EpicInstallDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, value) => match (id.as_ref(), value) {
                ("dlc-all", Value::Toggle(on)) => self.dlcs.iter_mut().for_each(|d| d.1 = on),
                ("shared", Value::Toggle(on)) => self.shared_prefix = on,
                ("wine", Value::Choice { index, .. }) => self.wine_pick = index,
                (id, Value::Toggle(on)) => {
                    if let Some(i) = id.strip_prefix("dlc").and_then(|n| n.parse::<usize>().ok()) {
                        if let Some(d) = self.dlcs.get_mut(i) {
                            d.1 = on;
                        }
                    } else if let Some(i) = id.strip_prefix("sdl").and_then(|n| n.parse::<usize>().ok()) {
                        if let Some(s) = self.sdl.get_mut(i) {
                            s.1 = on;
                        }
                    }
                }
                _ => {}
            },
            Outcome::Activate(id) => match id.as_ref() {
                "path" => self.pick_folder(self.path.clone(), tr!("Install folder"), window, cx),
                "import-path" => self.pick_folder(self.import_path.clone(), tr!("Game folder"), window, cx),
                "prefix" => self.pick_folder(self.prefix.clone(), tr!("Wine prefix"), window, cx),
                "install" => self.install(cx),
                "import" => self.import(cx),
                "mode" => {
                    self.mode = if self.mode == Mode::Install { Mode::Import } else { Mode::Install };
                    self.console_list.reset();
                }
                "cancel" => cx.emit(DialogEvent::Close),
                _ => {}
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change")), hint(Btn::B, tr!("Cancel"))]
    }
}
