//! Settings in console mode: the groups on LB / RB, the
//! group's sections on the left, the focused section's rows on the right
//! (◀ ▶ move between the two columns when a row has no value to change).
//! General, Downloads, Advanced and the Game defaults are the Heroic
//! settings panel's console rows; the other sections build their rows in
//! their own files (`console_rows` / `console_apply`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{n_, tr};

use super::{about, advanced, appearance, library, logs, metadata, saves, screenshots, Section, SettingsPage};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::dialogs::settings_panel::logic::Section as HSection;
use crate::dialogs::settings_panel::{GlobalView, SettingsPanel, Target};
use crate::icons::Icon;
use crate::theme;

/// A console entry of the left column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CSec {
    /// General / Downloads / Advanced: the global panel.
    Panel(Section),
    /// One tab of the Game defaults panel.
    Defaults(HSection),
    /// Folders, cache and the settings copy (the desktop Advanced extras).
    Extras,
    /// A section with its own rows.
    Rows(Section),
}

const GROUPS: [(&str, &[CSec]); 4] = [
    (n_!("App"), &[CSec::Panel(Section::General), CSec::Rows(Section::Appearance), CSec::Panel(Section::Advanced), CSec::Extras]),
    (
        n_!("Games"),
        &[
            CSec::Defaults(HSection::Wine),
            CSec::Defaults(HSection::Other),
            CSec::Defaults(HSection::Advanced),
            CSec::Defaults(HSection::Gamescope),
            CSec::Panel(Section::Downloads),
            CSec::Rows(Section::Library),
        ],
    ),
    (n_!("Collection"), &[CSec::Rows(Section::Metadata), CSec::Rows(Section::Screenshots), CSec::Rows(Section::Backups)]),
    (n_!("Help"), &[CSec::Rows(Section::Logs), CSec::Rows(Section::About)]),
];

impl CSec {
    fn label(self) -> String {
        match self {
            CSec::Panel(s) | CSec::Rows(s) => s.label().to_string(),
            CSec::Defaults(h) => tr!("Game defaults \u{b7} {section}", section = h.label()),
            CSec::Extras => tr!("Folders and cache").into(),
        }
    }

    fn icon(self) -> Icon {
        match self {
            CSec::Panel(s) | CSec::Rows(s) => s.icon(),
            CSec::Defaults(_) => Icon::Tune,
            CSec::Extras => Icon::Folder,
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            CSec::Panel(s) | CSec::Rows(s) => s.blurb(),
            CSec::Defaults(_) => tr!("What new games start with. Change an installed game in its own settings."),
            CSec::Extras => tr!("Open the app's folders, clear the image cache, copy the settings for a bug report."),
        }
    }
}

/// Console focus of the Settings page.
#[derive(Default)]
pub struct ConsoleState {
    group: usize,
    nav: usize,
    /// The right column has the focus.
    content: bool,
    pub(super) list: crate::console::list::ListNav,
    pub(super) layer: crate::console::Layer,
}

impl SettingsPage {
    fn console_sec(&self) -> CSec {
        let (_, secs) = GROUPS[self.console.group];
        secs[self.console.nav.min(secs.len() - 1)]
    }

    /// The panel shown by `sec`, set to the right view / tab.
    fn console_panel(&mut self, sec: CSec, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<SettingsPanel>> {
        match sec {
            CSec::Panel(s) => {
                let view = s.global_view().unwrap_or(GlobalView::General);
                let panel = self.panel_for(view, window, cx);
                panel.update(cx, |p, cx| p.set_global_view(view, cx));
                Some(panel)
            }
            CSec::Defaults(h) => {
                if self.defaults.is_none() {
                    let data_paths = self.ctx.data_paths.clone();
                    let panel = cx.new(|cx| SettingsPanel::new(data_paths, Target::Defaults, h, window, cx));
                    cx.observe(&panel, |_, _, cx| cx.notify()).detach();
                    self.defaults = Some(panel);
                }
                let panel = self.defaults.clone()?;
                panel.update(cx, |p, cx| p.set_section(h, window, cx));
                self.defaults_section = h;
                Some(panel)
            }
            _ => None,
        }
    }

    fn console_rows(&self, sec: CSec, cx: &App) -> Vec<Row> {
        match sec {
            CSec::Extras => advanced::console_rows(self),
            CSec::Rows(Section::Library) => library::console_rows(self),
            CSec::Rows(Section::About) => about::console_rows(self, cx),
            CSec::Rows(Section::Logs) => logs::console_rows(self),
            CSec::Rows(Section::Appearance) => appearance::console_rows(self),
            CSec::Rows(Section::Metadata) => metadata::console_rows(self, cx),
            CSec::Rows(Section::Screenshots) => screenshots::console_rows(self, cx),
            CSec::Rows(Section::Backups) => saves::console_rows(self, cx),
            _ => Vec::new(),
        }
    }

    fn console_apply(&mut self, sec: CSec, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        match sec {
            CSec::Extras => advanced::console_apply(self, outcome, cx),
            CSec::Rows(Section::Library) => library::console_apply(self, outcome, cx),
            CSec::Rows(Section::About) => about::console_apply(self, outcome, cx),
            CSec::Rows(Section::Logs) => logs::console_apply(self, outcome, cx),
            CSec::Rows(Section::Appearance) => appearance::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::Metadata) => metadata::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::Screenshots) => screenshots::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::Backups) => saves::console_apply(self, outcome, window, cx),
            _ => {}
        }
    }

    /// Starts the background work of the section that just got shown.
    fn console_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.console.list.reset();
        let section = match self.console_sec() {
            CSec::Extras => Some(Section::Advanced),
            CSec::Rows(s) => Some(s),
            _ => None,
        };
        if let Some(s) = section {
            self.section = s;
            self.on_section_shown(window, cx);
        }
    }

    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let sec = self.console_sec();
        let (_, secs) = GROUPS[self.console.group];
        let groups: Vec<SharedString> = GROUPS.iter().map(|(g, _)| SharedString::from(tr(g))).collect();
        let content_focus = self.console.content && !self.console.layer.is_open();
        let mut nav = div().flex().flex_col().gap(u(4.)).w(u(320.)).flex_none();
        for (i, s) in secs.iter().enumerate() {
            let on = i == self.console.nav;
            let focused = on && !self.console.content;
            nav = nav.child(
                kit::row_base(focused)
                    .min_h(u(50.))
                    .when(on && !focused, |d| d.bg(theme::white_a(0.08)))
                    .child(crate::ui::icon(s.icon(), u(20.), if focused { theme::console_on_focus() } else { theme::text_muted() }))
                    .child(div().flex_1().text_size(u(17.)).truncate().child(s.label())),
            );
        }
        let body: AnyElement = match self.console_panel(sec, window, cx) {
            None if sec == CSec::Rows(Section::Logs) => {
                let viewer = self.logs_viewer(cx);
                viewer.update(cx, |v, cx| v.render_console(cx))
            }
            Some(panel) => {
                panel.update(cx, |p, _| p.set_console_active(content_focus));
                div().flex().flex_col().size_full().child(panel).into_any_element()
            }
            None => {
                let rows = self.console_rows(sec, cx);
                self.console.list.render("console-settings-rows", &rows, content_focus).size_full().into_any_element()
            }
        };
        let error = self.error.clone().map(|e| kit::body_text(e).text_color(theme::danger_text()));
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
                    .child(kit::wordmark(tr!("SETTINGS")))
                    .child(kit::tabs(&groups, self.console.group))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div().flex().flex_1().min_h_0().gap(u(40.)).px(u(46.)).pb(u(16.)).child(nav).child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .max_w(u(900.))
                        .gap(u(6.))
                        .child(kit::title(sec.label(), 34.))
                        .child(div().pb(u(8.)).child(kit::body_text(sec.blurb())))
                        .child(div().flex().flex_col().flex_1().min_h_0().child(body))
                        .children(error),
                ),
            )
            .children(self.console.layer.view())
            .into_any_element()
    }
}

impl PadTarget for SettingsPage {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.console.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let n_groups = GROUPS.len();
        match pad {
            Pad::LB | Pad::RB => {
                let g = self.console.group;
                self.console.group = if pad == Pad::RB { (g + 1) % n_groups } else { (g + n_groups - 1) % n_groups };
                self.console.nav = 0;
                self.console.content = false;
                self.console_shown(window, cx);
                cx.notify();
                return true;
            }
            _ => {}
        }
        if !self.console.content {
            let len = GROUPS[self.console.group].1.len();
            match pad {
                Pad::Up if self.console.nav > 0 => {
                    self.console.nav -= 1;
                    self.console_shown(window, cx);
                }
                Pad::Down if self.console.nav + 1 < len => {
                    self.console.nav += 1;
                    self.console_shown(window, cx);
                }
                Pad::Right | Pad::A => self.console.content = true,
                Pad::Up | Pad::Down => {}
                _ => return false,
            }
            cx.notify();
            return true;
        }
        let sec = self.console_sec();
        let used = match sec {
            CSec::Rows(Section::Logs) => {
                let viewer = self.logs_viewer(cx);
                viewer.update(cx, |v, cx| v.on_pad(pad, window, cx))
            }
            CSec::Panel(_) | CSec::Defaults(_) => {
                let panel = self.console_panel(sec, window, cx);
                panel.is_some_and(|p| p.update(cx, |p, cx| p.on_pad(pad, window, cx)))
            }
            _ => {
                let rows = self.console_rows(sec, cx);
                match self.console.list.on_pad(pad, &rows) {
                    Outcome::Ignored | Outcome::Edge(_) => false,
                    Outcome::Moved => true,
                    o => {
                        self.console_apply(sec, o, window, cx);
                        true
                    }
                }
            }
        };
        if !used && matches!(pad, Pad::Left | Pad::B) {
            self.console.content = false;
            cx.notify();
            return true;
        }
        cx.notify();
        used
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.console.layer.hints(cx) {
            return h;
        }
        if !self.console.content {
            return vec![hint(Btn::A, tr!("Open")), hint(Btn::Bumpers, tr!("Group"))];
        }
        let mut h = match self.console_sec() {
            CSec::Panel(_) | CSec::Defaults(_) => {
                let panel = match self.console_sec() {
                    CSec::Defaults(_) => self.defaults.clone(),
                    _ => self.global.clone(),
                };
                panel.map(|p| p.read(cx).hints(cx)).unwrap_or_default()
            }
            _ => vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change"))],
        };
        h.push(hint(Btn::Bumpers, tr!("Group")));
        h
    }
}

impl SettingsPage {
    /// Opens the on-screen keyboard; `done` gets the text.
    pub(super) fn console_osk(
        &mut self,
        title: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let osk = cx.new(|_| crate::console::osk::Osk::new(title.to_string(), initial));
        let sub = cx.subscribe_in(&osk, window, move |this, _, ev: &crate::console::osk::OskEvent, window, cx| {
            this.console.layer.close();
            if let crate::console::osk::OskEvent::Done(text) = ev {
                done(this, text.clone(), window, cx);
            }
            cx.notify();
        });
        self.console.layer.open(&osk, sub);
    }

    /// Opens the console folder / file picker; `done` gets the path.
    pub(super) fn console_browse(
        &mut self,
        title: &str,
        kind: crate::console::browser::PickKind,
        start: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let browser = cx.new(|cx| crate::console::browser::Browser::new(title.to_string(), kind, start.as_deref(), cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &crate::console::browser::BrowserEvent, window, cx| {
            this.console.layer.close();
            if let crate::console::browser::BrowserEvent::Picked(p) = ev {
                done(this, p.display().to_string(), window, cx);
            }
            cx.notify();
        });
        self.console.layer.open(&browser, sub);
    }
}
