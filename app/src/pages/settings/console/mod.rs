//! Settings in console mode. On the left, a sidebar: the app's icon, name
//! and version, then Search and Changed, then every section under its group
//! heading in one scrolling list (`sidebar.rs`). On the right, the section
//! shown: a small "Group · Section" breadcrumb, a line about it and its
//! settings, each a card row (title and description on the left, its
//! control on the right; `RowStyle::Card`).
//!
//! ↑ ↓ pick a section (it shows at once), A or → enter its rows, B or ←
//! come back; LB / RB jump to the previous / next group. Search types on the
//! on-screen keyboard and lists the matching sections and settings; Changed
//! lists the settings that differ from their defaults where those are known
//! (`find.rs`). A on a result opens its section with that row focused.
//!
//! General, Downloads, Advanced and the Game defaults are the Heroic
//! settings panel's console rows; the other sections build their rows in
//! their own files (`console_rows` / `console_apply`).
//!
//! Debug: `HERONITE_DEBUG_PAGE=settings[:<section>]` opens on that section;
//! `HERONITE_DEBUG_SETTINGS_SEARCH=<text>` opens Search with those results
//! (`changed`: opens Changed).

mod find;
mod sidebar;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{n_, tr};

use super::{
    about, advanced, appearance, backup, console_mode, launchers, library, logs, metadata, mods, overlay, replay, saves, screenshots,
    storage, Section, SettingsPage,
};
use crate::console::kit::{self, u};
use crate::console::list::{self, Outcome, Row, RowStyle};
use crate::console::motion::{Axis, Glide};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::dialogs::settings_panel::logic::Section as HSection;
use crate::dialogs::settings_panel::{GlobalView, SettingsPanel, Target};
use crate::icons::Icon;
use crate::theme;

/// A section of the sidebar.
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

/// The sidebar's groups: a heading and its sections (as the desktop page
/// groups them).
const GROUPS: [(&str, &[CSec]); 7] = [
    (n_!("App"), &[CSec::Panel(Section::General), CSec::Rows(Section::Appearance), CSec::Rows(Section::ConsoleMode)]),
    (n_!("Games"), &[CSec::Panel(Section::Downloads), CSec::Rows(Section::Launchers), CSec::Rows(Section::Mods)]),
    (
        n_!("Game defaults"),
        &[
            CSec::Defaults(HSection::Compatibility),
            CSec::Defaults(HSection::Performance),
            CSec::Defaults(HSection::Gpu),
            CSec::Defaults(HSection::Launch),
        ],
    ),
    (n_!("Collection"), &[CSec::Rows(Section::Library), CSec::Rows(Section::Metadata)]),
    (n_!("While playing"), &[CSec::Rows(Section::Overlay), CSec::Rows(Section::Screenshots), CSec::Rows(Section::Replay)]),
    (n_!("Data"), &[CSec::Rows(Section::Storage), CSec::Rows(Section::Saves), CSec::Rows(Section::Backups)]),
    (n_!("System"), &[CSec::Panel(Section::Advanced), CSec::Extras, CSec::Rows(Section::Logs), CSec::Rows(Section::About)]),
];

impl CSec {
    fn available(self) -> bool {
        match self {
            CSec::Panel(s) | CSec::Rows(s) => s.available(),
            CSec::Defaults(_) => Section::GameDefaults.available(),
            CSec::Extras => true,
        }
    }

    fn label(self) -> String {
        match self {
            CSec::Panel(s) | CSec::Rows(s) => s.label().to_string(),
            CSec::Defaults(h) => h.label().to_string(),
            CSec::Extras => tr!("Folders and cache").into(),
        }
    }

    fn icon(self) -> Icon {
        match self {
            CSec::Panel(s) | CSec::Rows(s) => s.icon(),
            CSec::Defaults(HSection::Compatibility) => Icon::WineBar,
            CSec::Defaults(HSection::Performance) => Icon::BarChart,
            CSec::Defaults(HSection::Gpu) => Icon::Memory,
            CSec::Defaults(_) => Icon::PlayArrow,
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

    /// The heading of its group.
    fn group(self) -> &'static str {
        GROUPS.iter().find(|(_, secs)| secs.contains(&self)).map_or("", |(g, _)| heronite_i18n::tr(g))
    }

    /// The desktop section it belongs to (where its background work runs).
    fn section(self) -> Option<Section> {
        match self {
            CSec::Extras => Some(Section::Advanced),
            CSec::Rows(s) => Some(s),
            _ => None,
        }
    }
}

/// One stop of the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Entry {
    Search,
    Changed,
    Sec(CSec),
}

/// The sidebar's stops in order, with the group of each section.
fn entries() -> Vec<(Entry, Option<usize>)> {
    let mut v = vec![(Entry::Search, None), (Entry::Changed, None)];
    for (g, (_, secs)) in GROUPS.iter().enumerate() {
        v.extend(secs.iter().filter(|s| s.available()).map(|s| (Entry::Sec(*s), Some(g))));
    }
    v
}

/// Console focus of the Settings page.
pub struct ConsoleState {
    /// The sidebar stop shown (an index in [`entries`]).
    nav: usize,
    /// The right column has the focus.
    content: bool,
    /// The first render placed the focus (on the section asked for).
    started: bool,
    pub(super) list: crate::console::list::ListNav,
    pub(super) layer: crate::console::Layer,
    /// The sidebar's scroll, kept on the stop shown.
    side: Glide,
    side_reveal: bool,
    find: find::State,
}

impl Default for ConsoleState {
    fn default() -> Self {
        let mut list = crate::console::list::ListNav::default();
        list.set_style(RowStyle::Card);
        Self {
            nav: 0,
            content: false,
            started: false,
            list,
            layer: Default::default(),
            side: Glide::new(Axis::Y),
            side_reveal: true,
            find: Default::default(),
        }
    }
}

impl SettingsPage {
    fn console_entry(&self) -> Entry {
        let all = entries();
        all[self.console.nav.min(all.len() - 1)].0
    }

    /// The first render: the section `HERONITE_DEBUG_PAGE` (or the desktop
    /// page) was on, or Search / Changed for `HERONITE_DEBUG_SETTINGS_SEARCH`.
    fn console_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.console.started, true) {
            return;
        }
        let all = entries();
        let want = all.iter().position(|(e, _)| match e {
            Entry::Sec(CSec::Panel(s) | CSec::Rows(s)) => *s == self.section,
            Entry::Sec(CSec::Defaults(h)) => self.section == Section::GameDefaults && *h == self.defaults_section,
            _ => false,
        });
        self.console.nav = want.unwrap_or(all.iter().position(|(e, _)| matches!(e, Entry::Sec(_))).unwrap_or(0));
        if let Some(query) = crate::env::value("HERONITE_DEBUG_SETTINGS_SEARCH").map(|q| q.trim_end_matches('!').trim().to_string()) {
            if query == "changed" {
                self.console.nav = 1;
                self.console_shown(window, cx);
            } else if !query.is_empty() {
                self.console.nav = 0;
                self.console_shown(window, cx);
                self.console_search(query, window, cx);
            }
        }
    }

    /// The global panel shown by `sec`, set to the right view.
    fn global_panel(&mut self, view: GlobalView, window: &mut Window, cx: &mut Context<Self>) -> Entity<SettingsPanel> {
        let panel = self.panel_for(view, window, cx);
        panel.update(cx, |p, cx| p.set_global_view(view, cx));
        panel
    }

    /// The Game defaults panel (made on first use, on tab `h`).
    fn defaults_panel(&mut self, h: HSection, window: &mut Window, cx: &mut Context<Self>) -> Entity<SettingsPanel> {
        if let Some(p) = &self.defaults {
            return p.clone();
        }
        let data_paths = self.ctx.data_paths.clone();
        let panel = cx.new(|cx| SettingsPanel::new(data_paths, Target::Defaults, h, window, cx));
        cx.observe(&panel, |_, _, cx| cx.notify()).detach();
        self.defaults = Some(panel.clone());
        panel
    }

    /// The panel shown by `sec`, set to the right view / tab.
    fn console_panel(&mut self, sec: CSec, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<SettingsPanel>> {
        match sec {
            CSec::Panel(s) => Some(self.global_panel(s.global_view().unwrap_or(GlobalView::General), window, cx)),
            CSec::Defaults(h) => {
                let panel = self.defaults_panel(h, window, cx);
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
            CSec::Rows(Section::Launchers) => launchers::console_rows(self, cx),
            CSec::Rows(Section::Storage) => storage::console_rows(self),
            CSec::Rows(Section::Mods) => mods::console_rows(self, cx),
            CSec::Rows(Section::Overlay) => overlay::console_rows(self, cx),
            CSec::Rows(Section::Replay) => replay::console_rows(self, cx),
            CSec::Rows(Section::About) => about::console_rows(self, cx),
            CSec::Rows(Section::Logs) => logs::console_rows(self),
            CSec::Rows(Section::Appearance) => appearance::console_rows(self),
            CSec::Rows(Section::ConsoleMode) => console_mode::console_rows(self),
            CSec::Rows(Section::Metadata) => metadata::console_rows(self, cx),
            CSec::Rows(Section::Screenshots) => screenshots::console_rows(self, cx),
            CSec::Rows(Section::Saves) => saves::console_rows(self, cx),
            CSec::Rows(Section::Backups) => backup::console_rows(self, cx),
            _ => Vec::new(),
        }
    }

    fn console_apply(&mut self, sec: CSec, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        match sec {
            CSec::Extras => advanced::console_apply(self, outcome, cx),
            CSec::Rows(Section::Library) => library::console_apply(self, outcome, cx),
            CSec::Rows(Section::Launchers) => launchers::console_apply(self, outcome, cx),
            CSec::Rows(Section::Storage) => storage::console_apply(self, outcome, cx),
            CSec::Rows(Section::Mods) => mods::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::Overlay) => overlay::console_apply(self, outcome, cx),
            CSec::Rows(Section::Replay) => replay::console_apply(self, outcome, cx),
            CSec::Rows(Section::About) => about::console_apply(self, outcome, cx),
            CSec::Rows(Section::Logs) => logs::console_apply(self, outcome, cx),
            CSec::Rows(Section::Appearance) => appearance::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::ConsoleMode) => console_mode::send(self, outcome, window, cx),
            CSec::Rows(Section::Metadata) => metadata::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::Screenshots) => screenshots::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::Saves) => saves::console_apply(self, outcome, window, cx),
            CSec::Rows(Section::Backups) => {
                backup::console_apply(self, outcome, window, cx);
                cx.notify();
            }
            _ => {}
        }
    }

    /// Starts the background work of the stop that just got shown.
    fn console_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.console.list.reset();
        self.console.side_reveal = true;
        match self.console_entry() {
            Entry::Sec(sec) => {
                if let Some(panel) = self.console_panel(sec, window, cx) {
                    panel.update(cx, |p, _| p.console_reset());
                }
                if let Some(s) = sec.section() {
                    self.section = s;
                    self.on_section_shown(window, cx);
                }
            }
            Entry::Changed => self.console_changed(window, cx),
            Entry::Search => {}
        }
    }

    /// Moves the sidebar to stop `nav` (its section shows).
    fn console_go(&mut self, nav: usize, window: &mut Window, cx: &mut Context<Self>) {
        if nav != self.console.nav {
            self.console.nav = nav;
            self.console_shown(window, cx);
        }
    }

    /// The rows of the section shown (Search and Changed: their results).
    fn console_shown_rows(&mut self, sec: CSec, window: &mut Window, cx: &mut Context<Self>) -> Vec<Row> {
        if sec == CSec::Rows(Section::Metadata) {
            metadata::ensure_rows(self, window, cx);
        }
        self.console_rows(sec, cx)
    }

    /// The breadcrumb and the line under it.
    fn console_heading(&self, entry: Entry) -> (String, String) {
        match entry {
            Entry::Search => (tr!("Search").into(), tr!("Sections and settings, by their names and descriptions.").into()),
            Entry::Changed => (tr!("Changed").into(), find::changed_blurb().into()),
            Entry::Sec(sec) => (format!("{} \u{b7} {}", sec.group(), sec.label()), sec.blurb().to_string()),
        }
    }

    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.console_start(window, cx);
        self.console.list.set_style(RowStyle::Card);
        let entry = self.console_entry();
        let layer_open = self.console.layer.is_open();
        let content_focus = self.console.content && !layer_open;
        // A click on the content's rows from the sidebar enters it first.
        let enter = (!layer_open).then_some(&[Pad::Right][..]);
        let body: AnyElement = match entry {
            Entry::Search | Entry::Changed => {
                let rows = self.console_find_rows(entry);
                self.console.list.render_entered("console-settings-find", &rows, content_focus, enter).size_full().into_any_element()
            }
            Entry::Sec(CSec::Rows(Section::Logs)) => {
                let viewer = self.logs_viewer(cx);
                viewer.update(cx, |v, cx| v.render_console(cx))
            }
            Entry::Sec(sec) => match self.console_panel(sec, window, cx) {
                Some(panel) => {
                    panel.update(cx, |p, _| {
                        p.set_console_active(content_focus);
                        p.set_console_style(RowStyle::Card);
                    });
                    div().flex().flex_col().size_full().child(panel).into_any_element()
                }
                None => {
                    let rows = self.console_shown_rows(sec, window, cx);
                    self.console.list.render_entered("console-settings-rows", &rows, content_focus, enter).size_full().into_any_element()
                }
            },
        };
        let (crumb, blurb) = self.console_heading(entry);
        let error = self.error.clone().map(|e| kit::body_text(e).text_color(theme::danger_text()));
        let side = sidebar::render(self);
        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(div().pl(u(4.)).text_size(u(14.)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(crumb))
            .child(
                div()
                    .pl(u(4.))
                    .pt(u(4.))
                    .pb(u(10.))
                    .max_w(u(760.))
                    .text_size(u(13.))
                    .line_height(u(19.))
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_color(theme::text_dim())
                    .child(blurb),
            )
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
            .children(error);
        div()
            .relative()
            .flex()
            .justify_center()
            .size_full()
            .child(div().flex().w_full().max_w(u(1320.)).h_full().gap(u(30.)).px(u(56.)).pt(u(44.)).pb(u(10.)).child(side).child(content))
            .child(div().absolute().top(u(22.)).right(u(46.)).child(kit::clock()))
            .children(self.console.layer.view())
            .into_any_element()
    }

    /// LB / RB: the first stop of the previous / next group (Search and
    /// Changed count as one, before the first).
    fn console_jump_group(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        let all = entries();
        let here = all[self.console.nav.min(all.len() - 1)].1;
        // Search and Changed (`None`), then each group with a section here.
        let mut heads: Vec<Option<usize>> = vec![None];
        for (_, g) in &all {
            if g.is_some() && heads.last() != Some(g) {
                heads.push(*g);
            }
        }
        let at = heads.iter().position(|g| *g == here).unwrap_or(0);
        let n = heads.len();
        let to = heads[if next { (at + 1) % n } else { (at + n - 1) % n }];
        let nav = all.iter().position(|(_, g)| *g == to).unwrap_or(0);
        self.console.content = false;
        self.console_go(nav, window, cx);
    }

    /// The rows of a section that draws them here (not the panels, not Logs).
    fn console_rows_pad(&mut self, sec: CSec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // X moves a shelf or menu item, LT removes a shelf.
        if sec == CSec::Rows(Section::ConsoleMode) && console_mode::console_extra_pad(self, pad, cx) {
            return true;
        }
        let rows = self.console_shown_rows(sec, window, cx);
        match self.console.list.on_pad(pad, &rows) {
            Outcome::Ignored | Outcome::Edge(_) => false,
            Outcome::Moved => true,
            o => {
                self.console_apply(sec, o, window, cx);
                true
            }
        }
    }
}

impl PadTarget for SettingsPage {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.console.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if matches!(pad, Pad::LB | Pad::RB) {
            self.console_jump_group(pad == Pad::RB, window, cx);
            cx.notify();
            return true;
        }
        let entry = self.console_entry();
        if !self.console.content {
            let len = entries().len();
            match pad {
                Pad::Up if self.console.nav > 0 => self.console_go(self.console.nav - 1, window, cx),
                Pad::Down if self.console.nav + 1 < len => self.console_go(self.console.nav + 1, window, cx),
                // A on Search types at once; → goes to its results.
                Pad::A if entry == Entry::Search => self.console_search_keyboard(window, cx),
                Pad::Right | Pad::A => {
                    self.console.content = true;
                    self.console.list.reveal_focus();
                }
                Pad::Up | Pad::Down => {}
                _ => return false,
            }
            cx.notify();
            return true;
        }
        let used = match entry {
            Entry::Search | Entry::Changed => self.console_find_pad(entry, pad, window, cx),
            Entry::Sec(CSec::Rows(Section::Logs)) => {
                let viewer = self.logs_viewer(cx);
                viewer.update(cx, |v, cx| v.on_pad(pad, window, cx))
            }
            Entry::Sec(sec @ (CSec::Panel(_) | CSec::Defaults(_))) => {
                let panel = self.console_panel(sec, window, cx);
                panel.is_some_and(|p| p.update(cx, |p, cx| p.on_pad(pad, window, cx)))
            }
            Entry::Sec(sec) => self.console_rows_pad(sec, pad, window, cx),
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
        let entry = self.console_entry();
        let mut h = vec![hint(Btn::Bumpers, tr!("Groups")), hint(Btn::B, tr!("Back"))];
        if !self.console.content {
            h.push(hint(Btn::A, if entry == Entry::Search { tr!("Search") } else { tr!("Open") }));
            return h;
        }
        match entry {
            Entry::Search | Entry::Changed => {
                let rows = self.console_find_rows(entry);
                if let Some(r) = self.console.list.focused(&rows) {
                    h.push(hint(Btn::A, if r.id.as_ref() == "find:query" { tr!("Search") } else { tr!("Open") }));
                }
            }
            Entry::Sec(sec @ (CSec::Panel(_) | CSec::Defaults(_))) => {
                let panel = match sec {
                    CSec::Defaults(_) => self.defaults.clone(),
                    _ => self.global.clone(),
                };
                h.extend(panel.map(|p| p.read(cx).hints(cx)).unwrap_or_default());
            }
            Entry::Sec(CSec::Rows(Section::ConsoleMode)) => h.extend(console_mode::console_hints(self)),
            Entry::Sec(CSec::Rows(Section::Logs)) => {
                h.extend(logs::console_hints(self, cx));
            }
            Entry::Sec(sec) => {
                let rows = self.console_rows(sec, cx);
                h.extend(self.console.list.focused(&rows).and_then(list::hint_for));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::{entries, CSec, Entry, Section, GROUPS};

    #[test]
    fn console_navigation_hides_unavailable_features() {
        let sections: Vec<CSec> = entries()
            .into_iter()
            .filter_map(|(e, _)| match e {
                Entry::Sec(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(sections.contains(&CSec::Rows(Section::Overlay)), Section::Overlay.available());
        assert_eq!(sections.contains(&CSec::Rows(Section::Replay)), Section::Replay.available());
        assert!(sections.iter().all(|s| s.available()));
        // Every section is listed once.
        let all: Vec<CSec> = GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        for s in &all {
            assert_eq!(all.iter().filter(|x| *x == s).count(), 1, "{s:?}");
        }
    }

    #[test]
    fn search_and_changed_come_first_then_the_groups_in_order() {
        let e = entries();
        assert_eq!(e[0].0, Entry::Search);
        assert_eq!(e[1].0, Entry::Changed);
        let groups: Vec<usize> = e.iter().filter_map(|(_, g)| *g).collect();
        assert!(groups.windows(2).all(|w| w[0] <= w[1]));
    }
}
