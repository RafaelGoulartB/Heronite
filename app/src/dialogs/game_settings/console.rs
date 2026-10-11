//! Game settings in console mode: full screen, the game's art
//! on top with its cover and title, tabs on LB / RB, one focused row at a
//! time. The Heroic tabs are the settings panel's console rows; Launch,
//! Details and Images have their own rows here.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::native::{ExecutableKind, Platform};
use heronite_metadata::ArtKind;

use super::super::DialogEvent;
use super::details::TAG_FIELDS;
use super::{GameSettingsDialog, Tab};
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::osk::{Osk, OskEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

const RUN_AS: [(&str, &str); 3] = [("auto", "Automatic (from the file)"), ("linux", "Linux native"), ("windows", "Windows (Wine / Proton)")];

/// Which text field a keyboard edit goes to.
#[derive(Clone, Copy)]
enum Field {
    Name,
    Release,
    Score,
    Websites,
    Description,
    Notes,
    Tag(usize),
}

impl GameSettingsDialog {
    fn console_tabs(&self) -> Vec<Tab> {
        let mut t: Vec<Tab> = self.panel.as_ref().map(|(_, secs)| secs.iter().map(|s| Tab::of_section(*s)).collect()).unwrap_or_default();
        if self.launch.is_some() {
            t.push(Tab::Launch);
        }
        t.push(Tab::Details);
        t.push(Tab::Images);
        t
    }

    fn console_set_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        self.console_list.reset();
        if let (Some(section), Some((panel, _))) = (tab.section(), self.panel.as_ref()) {
            panel.update(cx, |p, cx| {
                p.set_section(section, window, cx);
                p.console_reset();
            });
        }
        cx.notify();
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        match self.tab {
            Tab::Launch => {
                let Some(l) = self.launch.as_ref() else { return Vec::new() };
                let exe = l.executable.read(cx).value().to_string();
                let dirty = exe != l.initial_exe || l.run_as != l.initial_run_as;
                let detected = match l.detected {
                    ExecutableKind::Linux => "Linux program",
                    ExecutableKind::Windows => "Windows program",
                    ExecutableKind::Unknown => "Unknown file type",
                };
                let effective = if l.effective == Platform::Linux { "runs natively on Linux" } else { "runs through Wine / Proton" };
                let index = RUN_AS.iter().position(|(v, _)| *v == l.run_as).unwrap_or(0);
                vec![
                    Row::text("exe", "Executable", exe.clone()).icon(Icon::Description).desc(if l.exists { format!("{detected}, {effective}") } else { "File not found".into() }),
                    Row::choice("run-as", "Run as", RUN_AS.iter().map(|(_, t)| SharedString::from(*t)).collect(), index),
                    Row::action("save-launch", if l.saved && !dirty { "Saved" } else { "Save" }).icon(Icon::Save).enabled(dirty && !exe.trim().is_empty() && self.busy.is_none()),
                ]
            }
            Tab::Details => {
                let d = &self.details;
                let form = self.read_form(cx);
                let dirty = form != self.initial;
                let mut rows = vec![
                    Row::text("f:name", "Name", d.name.read(cx).value().to_string()),
                    Row::text("f:release", "Release date", d.release.read(cx).value().to_string()),
                    Row::text("f:score", "Critic score", d.score.read(cx).value().to_string()),
                    Row::text("f:websites", "Websites", d.websites.read(cx).value().to_string()),
                    Row::text("f:description", "Description", d.description.read(cx).value().to_string()),
                    Row::text("f:notes", "Notes", d.notes.read(cx).value().to_string()),
                    Row::header("Tags (comma separated)"),
                ];
                for (i, (_, label, _)) in TAG_FIELDS.iter().enumerate() {
                    let values = d.tags[i].read(cx).values().join(", ");
                    rows.push(Row::text(SharedString::from(format!("f:tag{i}")), *label, values));
                }
                rows.push(Row::header(""));
                rows.push(Row::action("save-details", if self.busy.is_some() { "Saving\u{2026}" } else { "Save" }).icon(Icon::Save).enabled(dirty && self.busy.is_none()));
                rows
            }
            Tab::Images => {
                let custom = self.art.cover_url.is_some() || self.art.hero_url.is_some();
                vec![
                    Row::info("img-cover", "Cover", if self.art.cover_url.is_some() { "Custom" } else { "Default" }),
                    Row::info("img-hero", "Background", if self.art.hero_url.is_some() { "Custom" } else { "Default" }),
                    Row::action("img-reset", "Use the default images").icon(Icon::Replay).enabled(custom && self.busy.is_none()).desc("Choosing new images from the web needs desktop mode"),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn field_text(&self, f: Field, cx: &App) -> String {
        let d = &self.details;
        match f {
            Field::Name => d.name.read(cx).value().to_string(),
            Field::Release => d.release.read(cx).value().to_string(),
            Field::Score => d.score.read(cx).value().to_string(),
            Field::Websites => d.websites.read(cx).value().to_string(),
            Field::Description => d.description.read(cx).value().to_string(),
            Field::Notes => d.notes.read(cx).value().to_string(),
            Field::Tag(i) => d.tags[i].read(cx).values().join(", "),
        }
    }

    fn edit_field(&mut self, f: Field, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.field_text(f, cx);
        let osk = cx.new(|_| Osk::new(title.to_string(), &current));
        let sub = cx.subscribe_in(&osk, window, move |this, _, ev: &OskEvent, window, cx| {
            this.layer.close();
            if let OskEvent::Done(text) = ev {
                let d = &this.details;
                let t = text.clone();
                match f {
                    Field::Name => d.name.update(cx, |s, cx| s.set_value(t, window, cx)),
                    Field::Release => d.release.update(cx, |s, cx| s.set_value(t, window, cx)),
                    Field::Score => d.score.update(cx, |s, cx| s.set_value(t, window, cx)),
                    Field::Websites => d.websites.update(cx, |s, cx| s.set_value(t, window, cx)),
                    Field::Description => d.description.update(cx, |s, cx| s.set_value(t, window, cx)),
                    Field::Notes => d.notes.update(cx, |s, cx| s.set_value(t, window, cx)),
                    Field::Tag(i) => {
                        let values = t.split([',', ';']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        d.tags[i].update(cx, |tf, cx| tf.set_values(values, cx));
                    }
                }
            }
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    fn pick_executable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.launch.as_ref().map(|l| std::path::PathBuf::from(l.executable.read(cx).value().to_string()));
        let browser = cx.new(|cx| Browser::new("Executable", PickKind::File(Vec::new()), start.as_deref(), cx));
        let sub = cx.subscribe_in(&browser, window, |this, _, ev: &BrowserEvent, window, cx| {
            this.layer.close();
            if let BrowserEvent::Picked(p) = ev {
                let text = p.display().to_string();
                if let Some(l) = this.launch.as_mut() {
                    l.executable.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                    l.saved = false;
                }
                this.refresh_launch(&text);
            }
            cx.notify();
        });
        self.layer.open(&browser, sub);
    }

    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.images.update(cx, |im, _| im.frame(window));
        let s = kit::scale();
        let scale = window.scale_factor();
        let vw = f32::from(window.viewport_size().width);
        let (hero_chain, _) = self.art_chain(ArtKind::Hero);
        let hero = if hero_chain.is_empty() {
            None
        } else {
            let (w, h) = (vw * scale, 340. * s * scale);
            self.images.update(cx, |im, _| im.get("gs-console-hero", hero_chain, w, h))
        };
        let (cover_chain, _) = self.art_chain(ArtKind::Cover);
        let cover = if cover_chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get("gs-console-cover", cover_chain, 110. * s * scale, 150. * s * scale)) };

        let tabs = self.console_tabs();
        let active = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        let labels: Vec<SharedString> = tabs.iter().map(|t| SharedString::from(t.label())).collect();
        let mut meta = vec![self.game.source_label.to_string(), if self.game.installed { "Installed".into() } else { "Not installed".into() }];
        if self.game.playtime_minutes > 0 {
            meta.push(format!("{} played", heronite_data::collection::format_playtime_minutes(self.game.playtime_minutes)));
        }

        let header = div()
            .relative()
            .flex_none()
            .h(u(330.))
            .overflow_hidden()
            .children(hero.map(|h| img(h).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(theme::console_bg(), 0.),
                linear_color_stop(theme::rgb_a(11, 14, 19, 0.15), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .left(u(46.))
                    .right(u(46.))
                    .bottom(u(16.))
                    .flex()
                    .items_end()
                    .gap(u(24.))
                    .child(
                        div()
                            .flex_none()
                            .w(u(110.))
                            .h(u(150.))
                            .rounded(u(12.))
                            .overflow_hidden()
                            .bg(theme::white_a(0.08))
                            .shadow(kit::shadow(0.6, 10., 30.))
                            .children(cover.map(|c| img(c).size_full().object_fit(ObjectFit::Cover))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(u(4.))
                            .min_w_0()
                            .child(kit::kicker("Game settings"))
                            .child(kit::title(self.title.clone(), 48.))
                            .child(div().text_size(u(16.)).text_color(theme::text_muted()).child(meta.join(" \u{b7} "))),
                    ),
            );

        let body: AnyElement = match (self.tab.section(), &self.panel) {
            (Some(_), Some((panel, _))) => div().flex().flex_col().size_full().child(panel.clone()).into_any_element(),
            _ => {
                let rows = self.console_rows(cx);
                self.console_list.render("console-gs", &rows, !self.layer.is_open()).size_full().into_any_element()
            }
        };
        let error = (!self.error.is_empty()).then(|| kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        div()
            .absolute()
            .inset_0()
            .bg(theme::console_bg())
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_none().px(u(46.)).py(u(12.)).child(kit::tabs(&labels, active)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(u(46.))
                    .pb(u(70.))
                    .child(div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(body))
                    .children(error),
            )
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for GameSettingsDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let tabs = self.console_tabs();
        let i = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        match pad {
            Pad::LB | Pad::RB => {
                let n = tabs.len();
                let next = if pad == Pad::RB { (i + 1) % n } else { (i + n - 1) % n };
                self.console_set_tab(tabs[next], window, cx);
                return true;
            }
            Pad::B => {
                cx.emit(DialogEvent::Close);
                return true;
            }
            _ => {}
        }
        if self.tab.section().is_some() {
            if let Some((panel, _)) = self.panel.clone() {
                return panel.update(cx, |p, cx| p.on_pad(pad, window, cx));
            }
            return false;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "run-as" => {
                let exe = self.launch.as_ref().map(|l| l.executable.read(cx).value().to_string()).unwrap_or_default();
                if let Some(l) = self.launch.as_mut() {
                    l.run_as = RUN_AS[index].0.to_string();
                    l.saved = false;
                }
                self.refresh_launch(&exe);
            }
            Outcome::Activate(id) => match id.as_ref() {
                "exe" => self.pick_executable(window, cx),
                "save-launch" => self.save_launch(cx),
                "save-details" => self.save_details(cx),
                "img-reset" => self.reset_all_images(cx),
                "f:name" => self.edit_field(Field::Name, "Name", window, cx),
                "f:release" => self.edit_field(Field::Release, "Release date", window, cx),
                "f:score" => self.edit_field(Field::Score, "Critic score", window, cx),
                "f:websites" => self.edit_field(Field::Websites, "Websites", window, cx),
                "f:description" => self.edit_field(Field::Description, "Description", window, cx),
                "f:notes" => self.edit_field(Field::Notes, "Notes", window, cx),
                id => {
                    if let Some(i) = id.strip_prefix("f:tag").and_then(|n| n.parse::<usize>().ok()) {
                        self.edit_field(Field::Tag(i), TAG_FIELDS[i].1, window, cx);
                    }
                }
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
        if self.tab.section().is_some() {
            if let Some((panel, _)) = &self.panel {
                let mut h = panel.read(cx).hints(cx);
                h.push(hint(Btn::Bumpers, "Tabs"));
                h.push(hint(Btn::B, "Done"));
                return h;
            }
        }
        vec![hint(Btn::A, "Select"), hint(Btn::LeftRight, "Change"), hint(Btn::Bumpers, "Tabs"), hint(Btn::B, "Done")]
    }
}
