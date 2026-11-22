//! Game settings in console mode, laid out like the app's Settings: the
//! game on top, its groups in a column on the left, the shown entry's rows
//! on the right (◀ ▶ move between the two columns, LB / RB step through
//! the entries). Start and Saves have two entries: their own block (Play
//! and the actions, the backups), then the settings panel's group.
//! "Edit details" is its own rows under the game's art.
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{n_, tr};
use heronite_launch::native::{ExecutableKind, Platform};
use heronite_metadata::ArtKind;

use super::details::TAG_FIELDS;
use super::{GameSettingsDialog, Tab};
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, RowStyle, Value};
use crate::console::osk::{Osk, OskEvent};
use crate::console::pointer::Clickable;
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::dialogs::covers::CoversDialog;
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;

const RUN_AS: [(&str, &str); 3] =
    [("auto", n_!("Automatic (from the file)")), ("linux", n_!("Linux native")), ("windows", n_!("Windows (Wine / Proton)"))];

/// An entry of the left column: a tab's own block, or its panel group.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Entry {
    tab: Tab,
    lower: bool,
}

impl Entry {
    fn label(self) -> &'static str {
        match (self.tab, self.lower) {
            (Tab::Start, false) => tr!("Play"),
            (Tab::Saves, false) => tr!("Backups"),
            (tab, _) => tab.label(),
        }
    }

    fn icon(self) -> Icon {
        match (self.tab, self.lower) {
            (Tab::Game, _) => Icon::Edit,
            (Tab::Start, false) => Icon::PlayArrow,
            (Tab::Start, true) => Icon::Terminal,
            (Tab::Compatibility, _) => Icon::WineBar,
            (Tab::Graphics, _) => Icon::Tv,
            (Tab::Performance, _) => Icon::Memory,
            (Tab::Saves, false) => Icon::History,
            (Tab::Saves, true) => Icon::CloudSync,
        }
    }
}

/// The left column for `tabs`: each tab's own block when it has one, then
/// its panel group when the game has it.
fn entries_of(tabs: &[Tab], upper: impl Fn(Tab) -> bool, panel: impl Fn(Tab) -> bool) -> Vec<Entry> {
    let mut entries = Vec::new();
    for &tab in tabs {
        if upper(tab) {
            entries.push(Entry { tab, lower: false });
        }
        if panel(tab) {
            entries.push(Entry { tab, lower: true });
        }
    }
    entries
}

/// Which text field a keyboard edit goes to.
#[derive(Clone, Copy)]
enum Field {
    Name,
    SortName,
    Release,
    Score,
    Websites,
    Description,
    Notes,
    Tag(usize),
}

impl GameSettingsDialog {
    /// The entries of the left column, in order: each tab's own block (Play
    /// and the actions, the backups), then its panel group (`lower`).
    fn console_entries(&self) -> Vec<Entry> {
        entries_of(&self.tab_list(), |t| self.has_upper_of(t), |t| self.has_panel(t))
    }

    fn console_entry_at(&self, entries: &[Entry]) -> usize {
        entries.iter().position(|e| e.tab == self.tab && e.lower == self.lower).unwrap_or(0)
    }

    /// Shows `entry` on the right, its rows from the top.
    fn console_show(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = entry.tab;
        self.console_list.reset();
        if let (Some(section), Some((panel, _))) = (entry.tab.section().filter(|_| self.has_panel(entry.tab)), self.panel.as_ref()) {
            panel.update(cx, |p, cx| {
                p.set_section(section, window, cx);
                p.console_reset();
            });
        }
        self.console_focus(entry.lower, cx);
        if entry.tab == Tab::Saves {
            if let Some(view) = &self.backups {
                view.update(cx, |v, cx| {
                    v.console_reset();
                    v.shown(cx);
                });
            }
        }
        cx.notify();
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        match self.tab {
            Tab::Start => {
                // The action editor takes the whole entry; "Play" is its title.
                let mut rows = Vec::new();
                if self.actions.editor.is_none() {
                    rows.extend(if self.launch.is_some() { self.console_local_play_rows(cx) } else { vec![self.console_play_row()] });
                }
                rows.extend(self.console_action_rows(cx));
                rows
            }
            _ => self.console_info_rows(cx),
        }
    }

    /// How a local game starts: an executable and "run as", or a command.
    fn console_local_play_rows(&self, cx: &App) -> Vec<Row> {
        {
            {
                let Some(l) = self.launch.as_ref() else { return Vec::new() };
                let exe = l.executable.read(cx).value().to_string();
                let dirty = l.dirty(cx);
                let start = Row::choice(
                    "start",
                    tr!("Start with"),
                    vec![tr!("An executable").into(), tr!("A command").into()],
                    usize::from(l.by_command),
                );
                let save = Row::action("save-launch", if l.saved && !dirty { tr!("Saved") } else { tr!("Save") })
                    .icon(Icon::Save)
                    .enabled(dirty && l.can_save(cx) && self.busy.is_none());
                if l.by_command {
                    use super::launch::LaunchState;
                    let mut command = Row::text("command", tr!("Command"), LaunchState::text(&l.command, cx)).icon(Icon::Edit);
                    command = match l.command_problem(cx) {
                        Some(p) => command.desc(p),
                        None => {
                            command.desc(tr!("Runs inside the game folder, like a terminal opened there. Example: ./start.sh --fullscreen"))
                        }
                    };
                    return vec![
                        start,
                        Row::text("folder", tr!("Game folder"), LaunchState::text(&l.folder, cx)).icon(Icon::FolderOpen),
                        command,
                        save,
                    ];
                }
                let detected = match l.detected {
                    ExecutableKind::Linux => tr!("Linux program"),
                    ExecutableKind::Windows => tr!("Windows program"),
                    ExecutableKind::Unknown => tr!("Unknown file type"),
                };
                let effective =
                    if l.effective == Platform::Linux { tr!("runs natively on Linux") } else { tr!("runs through Wine / Proton") };
                let index = RUN_AS.iter().position(|(v, _)| *v == l.run_as).unwrap_or(0);
                vec![
                    start,
                    Row::text("exe", tr!("Executable"), exe.clone()).icon(Icon::Description).desc(if l.exists {
                        tr!("{detected}, {effective}", detected, effective)
                    } else {
                        tr!("File not found").into()
                    }),
                    Row::choice("run-as", tr!("Run as"), RUN_AS.iter().map(|(_, t)| SharedString::from(tr(t))).collect(), index),
                    save,
                ]
            }
        }
    }

    fn console_info_rows(&self, cx: &App) -> Vec<Row> {
        match self.tab {
            Tab::Game => {
                let d = &self.details;
                let form = self.read_form(cx);
                let dirty = form != self.initial;
                let mut rows = vec![
                    // One way in: the Covers dialog shows every picture and
                    // has its own Reset (Start › Options).
                    Row::header(tr!("Images")),
                    Row::action("img-find", tr!("Change pictures\u{2026}"))
                        .icon(Icon::Image)
                        .enabled(self.busy.is_none())
                        .desc(tr!("Covers and backgrounds, from the web or a file")),
                    Row::header(tr!("Details")),
                    Row::text("f:name", tr!("Name"), d.name.read(cx).value().to_string()),
                    Row::text("f:sort-name", tr!("Sorting name"), d.sort_name.read(cx).value().to_string())
                        .desc(tr!("Only orders the games A–Z: the name shown stays the same.")),
                    Row::text("f:release", tr!("Release date"), d.release.read(cx).value().to_string()),
                    Row::text("f:score", tr!("Critic score"), d.score.read(cx).value().to_string()),
                    Row::text("f:websites", tr!("Websites"), d.websites.read(cx).value().to_string()),
                    Row::text("f:description", tr!("Description"), d.description.read(cx).value().to_string()),
                    Row::text("f:notes", tr!("Notes"), d.notes.read(cx).value().to_string()),
                    Row::header(tr!("Tags (comma separated)")),
                ];
                for (i, (_, label, _)) in TAG_FIELDS.iter().enumerate() {
                    let values = d.tags[i].read(cx).values().join(", ");
                    rows.push(Row::text(SharedString::from(format!("f:tag{i}")), tr(label), values));
                }
                rows.push(Row::header(""));
                rows.push(
                    Row::action("save-details", if self.busy.is_some() { tr!("Saving…") } else { tr!("Save") })
                        .icon(Icon::Save)
                        .enabled(dirty && self.busy.is_none()),
                );
                rows
            }
            _ => Vec::new(),
        }
    }

    fn field_text(&self, f: Field, cx: &App) -> String {
        let d = &self.details;
        match f {
            Field::Name => d.name.read(cx).value().to_string(),
            Field::SortName => d.sort_name.read(cx).value().to_string(),
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
                    Field::SortName => d.sort_name.update(cx, |s, cx| s.set_value(t, window, cx)),
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

    /// "Change pictures…": the Covers dialog over this page. What it saves
    /// shows in the rows at once and reaches the Collection.
    fn console_find_pictures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ctx, game) = (self.ctx.clone(), self.game.clone());
        let view = cx.new(|cx| CoversDialog::new(ctx, game, None, window, cx));
        let sub = cx.subscribe(&view, |this: &mut Self, view, ev: &DialogEvent, cx| {
            if !matches!(ev, DialogEvent::Close) {
                this.art = view.read(cx).art().clone();
                cx.emit(DialogEvent::Changed);
            }
            if !matches!(ev, DialogEvent::Changed) {
                this.layer.close();
            }
            cx.notify();
        });
        self.layer.open(&view, sub);
    }

    fn pick_launch_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.launch.as_ref().map(|l| std::path::PathBuf::from(l.folder.read(cx).value().to_string()));
        let browser = cx.new(|cx| Browser::new(tr!("Game folder"), PickKind::Folder, start.as_deref(), cx));
        let sub = cx.subscribe_in(&browser, window, |this, _, ev: &BrowserEvent, window, cx| {
            this.layer.close();
            if let BrowserEvent::Picked(p) = ev {
                if let Some(l) = this.launch.as_mut() {
                    l.folder.update(cx, |s, cx| s.set_value(p.display().to_string(), window, cx));
                    l.saved = false;
                }
            }
            cx.notify();
        });
        self.layer.open(&browser, sub);
    }

    fn edit_launch_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.launch.as_ref().map(|l| l.command.read(cx).value().to_string()).unwrap_or_default();
        let osk = cx.new(|_| Osk::new(tr!("Command").to_string(), &current));
        let sub = cx.subscribe_in(&osk, window, |this, _, ev: &OskEvent, window, cx| {
            this.layer.close();
            if let OskEvent::Done(text) = ev {
                if let Some(l) = this.launch.as_mut() {
                    l.command.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                    l.saved = false;
                }
            }
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    fn pick_executable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.launch.as_ref().map(|l| std::path::PathBuf::from(l.executable.read(cx).value().to_string()));
        let browser = cx.new(|cx| Browser::new(tr!("Executable"), PickKind::File(Vec::new()), start.as_deref(), cx));
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

    /// A pad press for the rows of Play and the actions, or the details.
    fn console_rows_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) if self.console_action_activate(&id, window, cx) => {}
            Outcome::Changed(id, value) if self.console_action_changed(&id, &value, cx) => {}
            Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "run-as" => {
                let exe = self.launch.as_ref().map(|l| l.executable.read(cx).value().to_string()).unwrap_or_default();
                if let Some(l) = self.launch.as_mut() {
                    l.run_as = RUN_AS[index].0.to_string();
                    l.saved = false;
                }
                self.refresh_launch(&exe);
            }
            Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "start" => self.set_launch_by_command(index == 1, cx),
            Outcome::Activate(id) => match id.as_ref() {
                "exe" => self.pick_executable(window, cx),
                "folder" => self.pick_launch_folder(window, cx),
                "command" => self.edit_launch_command(window, cx),
                "save-launch" => self.save_launch(cx),
                "save-details" => self.save_details(cx),
                "img-find" => self.console_find_pictures(window, cx),
                "f:name" => self.edit_field(Field::Name, tr!("Name"), window, cx),
                "f:sort-name" => self.edit_field(Field::SortName, tr!("Sorting name"), window, cx),
                "f:release" => self.edit_field(Field::Release, tr!("Release date"), window, cx),
                "f:score" => self.edit_field(Field::Score, tr!("Critic score"), window, cx),
                "f:websites" => self.edit_field(Field::Websites, tr!("Websites"), window, cx),
                "f:description" => self.edit_field(Field::Description, tr!("Description"), window, cx),
                "f:notes" => self.edit_field(Field::Notes, tr!("Notes"), window, cx),
                id => {
                    if let Some(i) = id.strip_prefix("f:tag").and_then(|n| n.parse::<usize>().ok()) {
                        self.edit_field(Field::Tag(i), tr(TAG_FIELDS[i].1), window, cx);
                    }
                }
            },
            Outcome::Ignored | Outcome::Edge(_) => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    /// The game's art behind the top of the screen, fading into the background.
    fn console_hero(&mut self, height: f32, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let s = kit::scale();
        let scale = window.scale_factor();
        let vw = f32::from(crate::ui::content_size(window).width);
        let (chain, _) = self.art_chain(ArtKind::Hero);
        let hero = if chain.is_empty() {
            None
        } else {
            self.images.update(cx, |im, _| im.get("gs-console-hero", chain, vw * scale, height * s * scale))
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(u(height))
            .overflow_hidden()
            .children(hero.map(|h| img(h).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(theme::console_bg(), 0.),
                linear_color_stop(theme::rgb_a(11, 14, 19, 0.15), 1.),
            )))
    }

    fn console_cover(&mut self, w: f32, h: f32, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let s = kit::scale();
        let scale = window.scale_factor();
        let (chain, _) = self.art_chain(ArtKind::Cover);
        let cover = if chain.is_empty() {
            None
        } else {
            self.images.update(cx, |im, _| im.get("gs-console-cover", chain, w * s * scale, h * s * scale))
        };
        div()
            .flex_none()
            .w(u(w))
            .h(u(h))
            .rounded(u(10.))
            .overflow_hidden()
            .bg(theme::white_a(0.08))
            .shadow(kit::shadow(0.6, 10., 30.))
            .children(cover.map(|c| img(c).size_full().object_fit(ObjectFit::Cover)))
    }

    fn console_meta(&self) -> String {
        let mut meta = vec![
            self.game.source_label.to_string(),
            if self.game.installed { tr!("Installed").into() } else { tr!("Not installed").into() },
        ];
        if self.game.playtime_minutes > 0 {
            meta.push(tr!("{time} played", time = heronite_data::collection::format_playtime_minutes(self.game.playtime_minutes)));
        }
        meta.join(" \u{b7} ")
    }

    /// Like the app's Settings: the game on top, the groups in a column on
    /// the left, the shown group's rows on the right.
    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.images.update(cx, |im, _| im.frame(window));
        if self.details_only {
            return self.render_console_details(window, cx);
        }
        let entries = self.console_entries();
        let at = self.console_entry_at(&entries);
        let entry = entries.get(at).copied().unwrap_or(Entry { tab: self.tab, lower: self.lower });
        let layer_open = self.layer.is_open();
        let nav_focus = self.console_nav && !layer_open;
        let rows_focus = !self.console_nav && !layer_open;
        let mut nav = div().flex().flex_col().gap(u(4.)).w(u(320.)).flex_none();
        for (i, e) in entries.iter().enumerate() {
            let on = i == at;
            let focused = on && nav_focus;
            // A click: back from the rows (B), ↑ ↓ to the entry; on the
            // focused one, → into it. Passing over an entry shows it while
            // the column has the focus.
            let mut pads = if self.console_nav { Vec::new() } else { vec![Pad::B] };
            pads.extend(crate::console::pointer::along(at, i, Pad::Up, Pad::Down));
            let hover = if self.console_nav { pads.clone() } else { Vec::new() };
            if pads.is_empty() {
                pads.push(Pad::Right);
            }
            // A thin gap between the groups: Play and Start, Backups and Saves stay together.
            let first_of_group = i > 0 && entries[i - 1].tab != e.tab;
            nav = nav.child(
                kit::row_base(focused)
                    .when(first_of_group, |d| d.mt(u(10.)))
                    .when(!layer_open, |d| d.on_hover_pads(hover).on_pads(pads))
                    .min_h(u(50.))
                    .when(on && !focused, |d| d.bg(theme::white_a(0.08)))
                    .child(crate::ui::icon(e.icon(), u(20.), if focused { theme::console_on_focus() } else { theme::text_muted() }))
                    .child(div().flex_1().text_size(u(17.)).truncate().child(e.label())),
            );
        }

        // The rows are settings cards, as in Settings.
        let body: AnyElement = if entry.lower {
            match self.panel.as_ref() {
                Some((panel, _)) => {
                    panel.update(cx, |p, _| p.set_console_style(RowStyle::Card));
                    div().flex().flex_col().size_full().child(panel.clone()).into_any_element()
                }
                None => div().into_any_element(),
            }
        } else if entry.tab == Tab::Saves {
            self.backups.clone().map(|b| div().size_full().child(b).into_any_element()).unwrap_or_else(|| div().into_any_element())
        } else {
            let rows = self.console_rows(cx);
            let enter = (!layer_open).then_some(&[Pad::Right][..]);
            self.console_list.set_style(RowStyle::Card);
            self.console_list.render_entered("console-gs", &rows, rows_focus, enter).size_full().into_any_element()
        };

        let cover = self.console_cover(54., 72., window, cx);
        let meta = self.console_meta();
        let top = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(20.))
            .h(u(112.))
            .px(u(46.))
            .child(cover)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(2.))
                    .flex_1()
                    .min_w_0()
                    .child(kit::kicker(tr!("Game settings")))
                    .child(kit::title(self.title.clone(), 28.).truncate())
                    .child(div().text_size(u(15.)).text_color(theme::text_muted()).truncate().child(meta)),
            )
            .child(kit::clock());
        let error = (!self.error.is_empty()).then(|| kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        let hero = self.console_hero(300., window, cx);
        div()
            .absolute()
            .inset_0()
            .bg(theme::console_bg())
            .child(hero)
            .child(
                div().relative().flex().flex_col().size_full().child(top).child(
                    div().flex().flex_1().min_h_0().gap(u(40.)).px(u(46.)).pt(u(8.)).pb(u(70.)).child(nav).child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .max_w(u(900.))
                            .gap(u(6.))
                            .child(div().pb(u(8.)).child(kit::title(entry.label(), 34.)))
                            .child(div().relative().flex().flex_col().flex_1().min_h_0().child(body).child(kit::fade_bottom()))
                            .children(error),
                    ),
                ),
            )
            .children(self.layer.view())
            .into_any_element()
    }

    /// "Edit details": the game's art on top with its cover and title, the
    /// rows under it.
    fn render_console_details(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let header_h = 330.;
        let hero = self.console_hero(header_h, window, cx);
        let cover = self.console_cover(110., 150., window, cx);
        let meta = self.console_meta();
        let header = div().relative().flex_none().h(u(header_h)).overflow_hidden().child(hero).child(
            div().absolute().left(u(46.)).right(u(46.)).bottom(u(16.)).flex().items_end().gap(u(24.)).child(cover).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .min_w_0()
                    .child(kit::kicker(tr!("Edit details")))
                    .child(kit::title(self.title.clone(), 48.))
                    .child(div().text_size(u(16.)).text_color(theme::text_muted()).child(meta)),
            ),
        );
        let rows = self.console_rows(cx);
        let active = !self.layer.is_open();
        self.console_list.set_style(RowStyle::Card);
        let list = self.console_list.render("console-gs", &rows, active).size_full();
        let error = (!self.error.is_empty()).then(|| kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        div()
            .absolute()
            .inset_0()
            .bg(theme::console_bg())
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(u(46.))
                    .pt(u(12.))
                    .pb(u(70.))
                    .child(kit::settings_column(list))
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
        // The backups' keyboard and backup menu keep B and the bumpers.
        if let (Tab::Saves, Some(view), false, false) = (self.tab, self.backups.clone(), self.lower, self.console_nav) {
            if view.read(cx).captures(pad) {
                return view.update(cx, |v, cx| v.on_pad(pad, window, cx));
            }
        }
        if self.details_only {
            if pad == Pad::B {
                // What the details have pending is saved first.
                self.close(cx);
                return true;
            }
            return self.console_rows_pad(pad, window, cx);
        }
        let entries = self.console_entries();
        let at = self.console_entry_at(&entries);
        let n = entries.len();
        if matches!(pad, Pad::LB | Pad::RB) {
            if n > 1 {
                let next = if pad == Pad::RB { (at + 1) % n } else { (at + n - 1) % n };
                self.console_show(entries[next], window, cx);
            }
            return true;
        }
        // The left column: ↑ ↓ show an entry, → or A go into its rows.
        if self.console_nav {
            match pad {
                Pad::Up if at > 0 => self.console_show(entries[at - 1], window, cx),
                Pad::Down if at + 1 < n => self.console_show(entries[at + 1], window, cx),
                Pad::Up | Pad::Down => {}
                Pad::Right | Pad::A => {
                    self.console_nav = false;
                    self.console_sync(cx);
                }
                Pad::B => self.close(cx),
                _ => return false,
            }
            cx.notify();
            return true;
        }
        let used = if self.lower {
            self.panel.clone().is_some_and(|(p, _)| p.update(cx, |p, cx| p.on_pad(pad, window, cx)))
        } else if self.tab == Tab::Saves {
            self.backups.clone().is_some_and(|v| v.update(cx, |v, cx| v.on_pad(pad, window, cx)))
        } else if pad == Pad::B && self.tab == Tab::Start && self.close_action_editor(cx) {
            true
        } else {
            self.console_rows_pad(pad, window, cx)
        };
        // ◀ or B the rows don't use go back to the left column.
        if !used && matches!(pad, Pad::Left | Pad::B) {
            self.console_nav = true;
            self.console_sync(cx);
            cx.notify();
            return true;
        }
        used
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        let rows = || vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change"))];
        if self.details_only {
            let mut h = rows();
            h.push(hint(Btn::B, tr!("Done")));
            return h;
        }
        if self.console_nav {
            return vec![hint(Btn::A, tr!("Open")), hint(Btn::B, tr!("Done"))];
        }
        let mut h = match (self.lower, self.tab, &self.panel, &self.backups) {
            (true, _, Some((panel, _)), _) => panel.read(cx).hints(cx),
            (false, Tab::Saves, _, Some(view)) => view.read(cx).hints(cx),
            _ => rows(),
        };
        h.push(hint(Btn::B, tr!("Back")));
        h
    }
}

#[cfg(test)]
mod tests {
    use super::{entries_of, Entry, Tab};

    #[test]
    fn start_and_saves_get_their_own_block_then_the_panel() {
        let tabs = [Tab::Start, Tab::Graphics, Tab::Saves];
        let entries = entries_of(&tabs, |t| matches!(t, Tab::Start | Tab::Saves), |t| t != Tab::Saves);
        let e = |tab, lower| Entry { tab, lower };
        assert_eq!(entries, vec![e(Tab::Start, false), e(Tab::Start, true), e(Tab::Graphics, true), e(Tab::Saves, false)]);
        assert_eq!(entries[0].label(), "Play");
        assert_eq!(entries[3].label(), "Backups");
    }
}
