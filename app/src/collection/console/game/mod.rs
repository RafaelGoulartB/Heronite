//! Console game page (A on a cover), in the style the theme picks
//! (`game_page`): Classic (here: the game large over its art, the actions as
//! rows on the left — Play / Install / Stop, Status ◀ ▶, Favourite, Game
//! settings, Hidden... — and what the desktop focus panel shows on the right,
//! the description scrolling with the right stick), Hero (`hero.rs`), Tabs
//! (`tabs.rs`) or a quick Sheet over the tab (`sheet.rs`). All of them use
//! the same actions ([`CollectionView::game_rows`]).

mod hero;
mod sheet;
mod tabs;

use hero::facts_table;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;

use super::super::vm::DetailValue;
use super::super::CollectionView;
use super::backdrop::Scrim;
use super::hero::card_surface;
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row, Value};
use crate::console::look;
use crate::console::motion::{self, Axis, Glide};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;
use heronite_data::console_theme::{GamePage, TabSpec};
use heronite_i18n::tr;

pub(super) struct GameNav {
    list: ListNav,
    about: Glide,
    hero: hero::Nav,
    tabs: tabs::Nav,
    sheet: sheet::Nav,
}

impl Default for GameNav {
    fn default() -> Self {
        Self {
            list: ListNav::default(),
            about: Glide::new(Axis::Y),
            hero: Default::default(),
            tabs: Default::default(),
            sheet: Default::default(),
        }
    }
}

fn detail_text(v: &DetailValue) -> String {
    match v {
        DetailValue::Text(t) => t.to_string(),
        DetailValue::Chips { values, .. } => values.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(", "),
        DetailValue::Folder { path, size } => match size {
            Some(s) => format!("{path} ({s})"),
            None => path.to_string(),
        },
    }
}

/// The facts of Classic's card, without what it shows above them: the
/// status and the store are in the header, the times in the numbers.
fn classic_facts(f: &super::super::vm::FocusVm) -> Vec<(SharedString, String)> {
    let shown = [tr!("Last Played"), tr!("Playtime")];
    f.details.iter().filter(|d| !shown.contains(&d.label.as_ref())).map(|d| (d.label.clone(), detail_text(&d.value))).collect()
}

impl CollectionView {
    fn game_rows(&mut self, g: &CollectionGame, cx: &mut Context<Self>) -> Vec<Row> {
        let playing = self.playing.contains_key(&g.app_name);
        let step = self.preparing.get(&g.app_name).cloned();
        let (label, ic) = self.primary_state(g);
        let mut primary = Row::action("primary", label).icon(ic);
        if let Some(step) = step {
            primary = primary.desc(step.label());
        }
        let mut rows = vec![primary];
        // The actions the user added; one that plays waits for the game to end.
        for a in self.actions_of(&g.key) {
            if a.plays() && playing {
                continue;
            }
            rows.push(
                Row::action(SharedString::from(format!("ua:{}", a.id)), a.name.clone())
                    .icon(crate::ui::game_actions::icon_of(a))
                    .desc(crate::ui::game_actions::menu_note(a)),
            );
        }
        if !playing
            && super::super::install_options::can_install_manually(g)
            && super::super::install_options::store_install_label(g).is_some()
        {
            if super::super::install_options::steam_way(g) {
                rows.push(
                    Row::action("x:steam-install", tr!("Install with Steam"))
                        .icon(Icon::SportsEsports)
                        .desc(tr!("You own it on Steam too")),
                );
            }
            rows.push(
                Row::action("x:manual-install", tr!("Install manually\u{2026}"))
                    .icon(Icon::Edit)
                    .desc(tr!("Run an installer or pick the game's files")),
            );
        }
        if let Some(lib) = &self.lib {
            let names: Vec<SharedString> =
                lib.statuses.iter().map(|s| SharedString::from(heronite_data::status::status_label(&s.name).to_string())).collect();
            let index = lib.statuses.iter().position(|s| s.id == g.status_id).unwrap_or(0);
            rows.push(Row::choice("status", tr!("Status"), names, index).icon(Icon::Flag));
        }
        rows.push(Row::toggle("favourite", tr!("Favourite"), g.favourite).icon(Icon::Favorite));
        // An Epic game's other ways to start; Play uses the one picked.
        if g.installed && !self.facts.launch_options.is_empty() && self.selected.as_deref() == Some(g.key.as_str()) {
            let choices = super::super::launch_options::choices(&self.facts.launch_options);
            let names = choices.iter().map(|c| SharedString::from(super::super::launch_options::choice_label(c))).collect();
            let index = choices.iter().position(|c| *c == self.facts.launch_current).unwrap_or(0);
            rows.push(Row::choice("launch-option", tr!("Start with"), names, index).icon(Icon::PlayArrow));
        }
        if g.installed {
            rows.push(
                Row::action("settings", tr!("Game settings"))
                    .icon(Icon::Tune)
                    .desc(tr!("Wine / Proton, launch options, Gamescope\u{2026}")),
            );
        }
        if !playing {
            rows.push(
                Row::text("playtime", tr!("Playtime"), heronite_data::collection::playtime_label(g.playtime_minutes))
                    .icon(Icon::AccessTime),
            );
        }

        // What the desktop context menu offers besides the above.
        let mut manage = self.epic_console_rows(g, cx);
        let steam_game = matches!(g.launch_kind.as_deref(), Some("steam-uri"));
        if g.installed && !steam_game && g.game.install.is_dlc != Some(true) {
            let (shortcuts, in_steam) = self.probe_shortcuts(g, cx);
            manage.push(Row::toggle("sys:shortcuts", tr!("Desktop and menu shortcuts"), shortcuts).icon(Icon::OpenInNew));
            manage.push(
                Row::toggle("sys:steam", tr!("In Steam"), in_steam)
                    .icon(Icon::SportsEsports)
                    .desc(tr!("As a non-Steam game in your Steam library")),
            );
        }
        manage.push(Row::action("x:ludusavi", tr!("Back up saves (Ludusavi)")).icon(Icon::Save));
        manage.push(
            Row::toggle("hidden", tr!("Hidden"), g.hidden).icon(Icon::VisibilityOff).desc(tr!("Hidden games stay out of the library")),
        );
        if g.recent {
            manage.push(Row::action("recent", tr!("Remove from recent")).icon(Icon::History));
        }
        if g.runner == "sideload"
            && !playing
            && !matches!(g.launch_kind.as_deref(), Some("steam-uri" | "emulator" | heronite_data::launchers::LAUNCH_KIND))
        {
            manage.push(Row::action("x:edit", tr!("Edit game")).icon(Icon::Edit));
        }
        if g.runner == "sideload" && !playing && g.launch_kind.as_deref() == Some("emulator") {
            manage.push(
                Row::action("x:as-pc", tr!("Play as a PC game\u{2026}"))
                    .icon(Icon::SportsEsports)
                    .desc(tr!("A native port or a PC version, instead of the emulator")),
            );
        }
        if !playing && g.steam_also_playable().is_some() {
            manage.push(Row::action("x:steam-play", tr!("Play with Steam")).icon(Icon::PlayArrow));
        }
        if !playing && g.installed && g.steam_installable().is_some() {
            manage.push(Row::action("x:steam-install", tr!("Install with Steam")).icon(Icon::Download));
        }
        if g.steam_installed_id().is_some() && g.installed && !playing {
            let label = if g.steam_also_playable().is_some() { tr!("Uninstall from Steam\u{2026}") } else { tr!("Uninstall\u{2026}") };
            manage.push(Row::action("x:steam-uninstall", label).icon(Icon::DeleteForever).danger());
        }
        if !playing && super::super::sideload_uninstall::can_uninstall_local(g) {
            manage.push(Row::action("x:local-uninstall", tr!("Uninstall\u{2026}")).icon(Icon::DeleteForever).danger());
        }
        if g.runner == "sideload" && !playing {
            manage.push(Row::action("x:remove", tr!("Remove from Collection\u{2026}")).icon(Icon::DeleteForever).danger());
        }
        // One switch per custom category (made on the desktop's View menu).
        if !self.categories.is_empty() {
            rows.push(Row::header(tr!("Categories")));
            for name in self.categories.names() {
                let on = self.categories.contains(name, &g.runner, &g.app_name);
                rows.push(Row::toggle(SharedString::from(format!("cat:{name}")), name.to_string(), on).icon(Icon::Label));
            }
        }
        rows.push(Row::header(tr!("Manage")));
        rows.extend(manage);
        rows
    }

    /// What the main action of `g` is now: "Cancel launch", "Stop", "Play" or "Install", with its icon.
    pub(super) fn primary_state(&self, g: &CollectionGame) -> (&'static str, Icon) {
        if self.preparing.contains_key(&g.app_name) {
            (tr!("Cancel launch"), Icon::Close)
        } else if self.playing.contains_key(&g.app_name) {
            (tr!("Stop"), Icon::Stop)
        } else if g.installed {
            (tr!("Play"), Icon::PlayArrow)
        } else {
            (tr!("Install"), Icon::Download)
        }
    }

    /// "Play", "Install" or "Stop" for the X shortcut.
    pub(super) fn primary_label(&self, g: &CollectionGame) -> &'static str {
        self.primary_state(g).0
    }

    /// The game page in the theme's style.
    pub(super) fn render_console_game(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match look::theme().game_page {
            GamePage::Classic => self.render_game_classic(key, window, cx),
            GamePage::Hero => self.render_game_hero(key, window, cx),
            GamePage::Tabs => self.render_game_tabs(key, window, cx),
            GamePage::Sheet => self.render_game_sheet(key, window, cx),
        }
    }

    /// What the game page draws over the whole window (the Sheet's floating panel).
    pub(super) fn render_console_game_layer(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match look::theme().game_page {
            GamePage::Sheet => self.render_game_sheet_layer(key, window, cx),
            _ => None,
        }
    }

    /// `false` lets the shell use the press (B goes back).
    pub(super) fn game_pad(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match look::theme().game_page {
            GamePage::Classic => self.game_pad_classic(key, pad, window, cx),
            GamePage::Hero => self.game_pad_hero(key, pad, window, cx),
            GamePage::Tabs => self.game_pad_tabs(key, pad, window, cx),
            GamePage::Sheet => self.game_pad_sheet(key, pad, window, cx),
        }
    }

    pub(super) fn game_hints(&self, key: &SharedString) -> Vec<Hint> {
        match look::theme().game_page {
            GamePage::Classic => self.game_hints_classic(key),
            GamePage::Hero => self.game_hints_hero(key),
            GamePage::Tabs => self.game_hints_tabs(key),
            GamePage::Sheet => self.game_hints_sheet(key),
        }
    }

    /// The background shading of the game page (the Sheet keeps the tab's).
    pub(super) fn game_scrim(&self, spec: &TabSpec) -> Scrim {
        match look::theme().game_page {
            GamePage::Classic | GamePage::Tabs => Scrim::Classic,
            GamePage::Hero => Scrim::Spot,
            GamePage::Sheet if self.console.game.sheet.full() => Scrim::Classic,
            GamePage::Sheet => super::layouts::scrim(spec.layout),
        }
    }

    /// The Sheet opens over the tab: its top bar stays.
    pub(super) fn game_page_keeps_tabs(&self) -> bool {
        look::theme().game_page == GamePage::Sheet && !self.console.game.sheet.full()
    }

    fn render_game_classic(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else {
            return kit::empty(Icon::Warning, tr!("This game is gone"), tr!("Press B to go back.")).into_any_element();
        };
        self.console_focus_game(Some(key.clone()), cx);
        let rows = self.game_rows(&g, cx);
        let list = self.console.game.list.render("console-game-actions", &rows, true);
        let s = kit::scale();
        let (cw, ch) = (136., 136. * self.cover_aspect);
        let cover = self.cover(key, cw * s, ch * s, window.scale_factor(), 300);
        let focus = self.focus.clone().filter(|f| f.key == *key);

        let title = super::hero::fitted_title(52., g.title.chars().count());
        let header = div()
            .flex()
            .items_end()
            .gap(u(28.))
            .px(u(46.))
            .pt(u(64.))
            .pb(u(24.))
            .child(
                div()
                    .flex_none()
                    .w(u(cw))
                    .h(u(ch))
                    .rounded(u(look::tile_radius() + 2.))
                    .overflow_hidden()
                    .bg(kit::ink(0.08))
                    .border_1()
                    .border_color(kit::ink(0.12))
                    .shadow(kit::shadow(0.6, 12., 34.))
                    .children(cover.map(|c| img(c).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(motion::enter(
                SharedString::from(format!("game-head-{key}")),
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(u(8.))
                    .min_w_0()
                    .max_w(u(900.))
                    .pb(u(4.))
                    .child(kit::title(g.title.clone(), title).line_clamp(2))
                    .child(self.console_fact_line(&g, Self::console_facts(&g), 16.))
                    .children(
                        focus
                            .as_ref()
                            .map(|f| f.meta_line.clone())
                            .filter(|m| !m.is_empty())
                            .map(|m| div().text_size(u(14.)).text_color(theme::text_dim()).truncate().child(m)),
                    ),
                320,
                0.,
                10.,
            ));

        // Wide enough for the facts in two columns (design px of the card).
        let vw = f32::from(window.viewport_size().width) / s;
        let card_w = vw - 2. * 46. - 560. - 40.;
        let mut about =
            div().id("console-game-about").flex().flex_col().gap(u(20.)).overflow_y_scroll().track_scroll(self.console.game.about.handle());
        about = about.child(self.classic_numbers(&g, focus.as_ref()));
        if let Some(f) = &focus {
            if let Some(o) = &f.overview {
                // Lines a reader can follow on a wide screen.
                about = about.child(
                    kit::body_text(o.clone()).max_w(u(980.)).text_size(u(16.)).line_height(u(25.)).text_color(theme::text_primary()),
                );
            }
            let facts = classic_facts(f);
            if !facts.is_empty() {
                about = about
                    .child(div().h(px(1.)).flex_none().bg(kit::ink(0.08)))
                    .child(facts_table(&facts, if card_w > 760. { 2 } else { 1 }));
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(self.console.game.about.driver())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(u(40.))
                    .px(u(46.))
                    .pb(u(20.))
                    .child(div().flex_none().w(u(560.)).flex().flex_col().child(list.flex_1().min_h_0()))
                    .child(card_surface().flex_1().min_w_0().p(u(26.)).rounded(u(20.)).flex().flex_col().child(motion::fade_in(
                        SharedString::from(format!("game-about-{key}")),
                        about.flex_1().min_h_0(),
                        360,
                    ))),
            )
            .into_any_element()
    }

    /// The numbers at the top of Classic's card: the time played (once
    /// played), when it was last played, How Long To Beat's main story and
    /// the score when known.
    fn classic_numbers(&self, g: &CollectionGame, f: Option<&super::super::vm::FocusVm>) -> Div {
        let played = super::hero::played_time(g);
        let last = g.last_played.as_deref().and_then(super::super::data::played_ago).unwrap_or_else(|| tr!("Never").to_string());
        let beat = f.and_then(|f| f.hltb.as_ref()?.main).filter(|h| *h > 0.).map(|h| format!("{h:.0}h"));
        let score = g.details.critic_score.filter(|s| *s > 0.);
        let cell = |label: &str, value: String, colour: Option<Rgba>| {
            div()
                .flex_1()
                .min_w_0()
                .max_w(u(260.))
                .flex()
                .flex_col()
                .gap(u(4.))
                .px(u(16.))
                .py(u(12.))
                .rounded(u(14.))
                .bg(kit::ink(0.05))
                .child(div().text_size(u(13.)).text_color(theme::text_dim()).truncate().child(label.to_string()))
                .child(
                    div()
                        .text_size(u(22.))
                        .font_weight(theme::W_BOLD)
                        .text_color(colour.map_or_else(theme::text_primary, kit::tint))
                        .truncate()
                        .child(value),
                )
        };
        div()
            .flex()
            .flex_none()
            .gap(u(12.))
            .children(played.map(|p| cell(tr!("Playtime"), p, None)))
            .child(cell(tr!("Last played"), last, None))
            .children(beat.map(|b| cell(tr!("Main story"), b, None)))
            .children(score.map(|s| cell(tr!("Score"), format!("{s:.0}"), Some(super::super::focus_left::score_color(s)))))
    }

    fn game_pad_classic(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return false };
        if matches!(pad, Pad::ScrollUp | Pad::ScrollDown) {
            let step = f32::from(u(120.));
            self.console.game.about.by(if pad == Pad::ScrollDown { step } else { -step });
            return true;
        }
        if pad == Pad::X {
            self.primary_action(key, cx);
            return true;
        }
        let rows = self.game_rows(&g, cx);
        match self.console.game.list.on_pad(pad, &rows) {
            Outcome::Activate(id) if id.starts_with("epic:") => self.epic_console_activate(&id, key, cx),
            Outcome::Changed(id, _) if id.as_ref() == "epic:eos" => self.epic_console_activate(&id, key, cx),
            Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("sys:") => {
                let (runner, app) = (g.runner.clone(), g.app_name.clone());
                let steam = id.as_ref() == "sys:steam";
                self.system_op(g.key.clone(), cx, move |data_paths| {
                    use crate::system::shortcuts as sc;
                    match (steam, on) {
                        (false, true) => sc::add_shortcuts(data_paths, &runner, &app),
                        (false, false) => sc::remove_shortcuts(data_paths, &runner, &app),
                        (true, true) => sc::add_to_steam(data_paths, &runner, &app),
                        (true, false) => sc::remove_from_steam(data_paths, &runner, &app),
                    }
                });
            }
            Outcome::Activate(id) if id.starts_with("ua:") => self.run_user_action(key, &id["ua:".len()..], cx),
            Outcome::Activate(id) => match id.as_ref() {
                "primary" => self.install_or_play(key, window, cx),
                "x:manual-install" => self.open_manual_install(key, window, cx),
                "x:ludusavi" => self.open_ludusavi_backup(key, cx),
                "x:steam-uninstall" => self.open_steam_uninstall(key, window, cx),
                "x:steam-play" => self.play_with_steam(key, cx),
                "x:steam-install" => self.install_with_steam(key, cx),
                "x:local-uninstall" => self.open_sideload_uninstall(key, window, cx),
                "x:remove" => self.open_remove_game(key, cx),
                "x:edit" => {
                    self.open_edit_game(key, window, cx);
                }
                "x:as-pc" => self.open_play_as_pc(key, window, cx),
                "settings" => self.console_game_settings(key, window, cx),
                "playtime" => self.console_edit_playtime(key, window, cx),
                "recent" => {
                    if let Err(err) = heronite_data::config_games::remove_recent(&self.data_paths, &g.app_name) {
                        self.show_toast(format!("{err:#}"), true, cx);
                    }
                    self.reload(cx);
                }
                _ => {}
            },
            Outcome::Changed(id, value) => match (id.as_ref(), value) {
                ("launch-option", Value::Choice { index, .. }) => {
                    let choice = super::super::launch_options::choices(&self.facts.launch_options).into_iter().nth(index).flatten();
                    self.set_launch_option(key, choice, cx);
                }
                ("status", Value::Choice { index, .. }) => {
                    if let Some(st) = self.lib.as_ref().and_then(|l| l.statuses.get(index)).cloned() {
                        self.set_status(key, st.id, cx);
                    }
                }
                ("favourite", _) => {
                    if let Err(err) = heronite_data::config_games::toggle_favourite(&self.data_paths, &g.app_name, &g.title) {
                        self.show_toast(format!("{err:#}"), true, cx);
                    }
                    self.reload(cx);
                }
                ("hidden", _) => {
                    if let Err(err) = heronite_data::config_games::toggle_hidden(&self.data_paths, &g.app_name, &g.title) {
                        self.show_toast(format!("{err:#}"), true, cx);
                    }
                    self.reload(cx);
                }
                (id, Value::Toggle(on)) if id.starts_with("cat:") => self.set_game_category(key, &id["cat:".len()..], on, cx),
                _ => {}
            },
            Outcome::Moved | Outcome::Edge(_) => {}
            Outcome::Ignored => return false,
        }
        true
    }

    fn game_hints_classic(&self, key: &SharedString) -> Vec<Hint> {
        let primary = self.lib.as_ref().and_then(|l| l.get(key)).map(|g| self.primary_label(g)).unwrap_or(tr!("Play"));
        vec![
            hint(Btn::A, tr!("Select")),
            hint(Btn::LeftRight, tr!("Change")),
            hint(Btn::X, primary),
            hint(Btn::B, tr!("Back")),
            hint(Btn::Start, tr!("Menu")),
        ]
    }
}
