//! The game page as Hero: the art full screen with the logo (or name), the
//! facts, the actions as a row of buttons and tabs (LB / RB) under them:
//! Overview, Details, Screenshots and Journal (the last two only when the
//! game has some). It also holds the parts Tabs shares: the facts table, the
//! screenshot thumbnails and their full-screen viewer, and the journal notes.

use std::time::Duration;

use chrono::{Datelike, Local, TimeZone};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::hero::{clip, HeroSize};
use super::super::layouts::step;
use super::detail_text;
use super::sheet::side_sheet;
use crate::collection::vm::FocusVm;
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Tab {
    #[default]
    Overview,
    Details,
    Screenshots,
    Journal,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Overview => tr!("Overview"),
            Tab::Details => tr!("Details"),
            Tab::Screenshots => tr!("Screenshots"),
            Tab::Journal => tr!("Journal"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Button {
    Primary,
    Status,
    Favourite,
    Settings,
    More,
}

impl Button {
    /// The row of the game's actions the button stands for.
    fn row(self) -> Option<&'static str> {
        match self {
            Button::Primary => Some("primary"),
            Button::Status => Some("status"),
            Button::Favourite => Some("favourite"),
            Button::Settings => Some("settings"),
            Button::More => None,
        }
    }
}

#[derive(Default)]
pub(super) struct Nav {
    /// The focus is on the content under the tabs (else on the buttons).
    content: bool,
    button: usize,
    tab: Tab,
    /// Focused screenshot, and the first one shown.
    shot: usize,
    shot_start: usize,
    /// The sheet with the other actions.
    more: Option<ListNav>,
}

/// Screenshots shown at once.
const SHOTS: usize = 4;
/// Height of the content under the tabs, design px.
const CONTENT_H: f32 = 190.;

/// The buttons the row has for these actions.
fn buttons(rows: &[Row]) -> Vec<Button> {
    let has = |id: &str| rows.iter().any(|r| r.id.as_ref() == id);
    let mut out = vec![Button::Primary];
    if has("status") {
        out.push(Button::Status);
    }
    out.push(Button::Favourite);
    if has("settings") {
        out.push(Button::Settings);
    }
    out.push(Button::More);
    out
}

/// The actions the buttons do not cover (what "More" lists).
fn more_rows(rows: &[Row]) -> Vec<Row> {
    rows.iter().filter(|r| !["primary", "status", "favourite", "settings"].contains(&r.id.as_ref())).cloned().collect()
}

/// First item to show so that `focus` is among the `visible` from it,
/// moving as little as possible.
pub(super) fn window_start(start: usize, focus: usize, visible: usize) -> usize {
    if focus < start {
        focus
    } else if focus >= start + visible {
        focus + 1 - visible
    } else {
        start
    }
}

/// Status, score, library and the details of the Classic page's right panel.
pub(super) fn detail_facts(f: &FocusVm) -> Vec<(SharedString, String)> {
    let head = [
        f.status.as_ref().map(|s| (SharedString::from(tr!("Status")), s.name.to_string())),
        f.score.map(|s| (SharedString::from(tr!("Score")), format!("{s:.0}"))),
        Some((SharedString::from(tr!("Library")), f.library.to_string())),
    ];
    head.into_iter().flatten().chain(f.details.iter().map(|d| (d.label.clone(), detail_text(&d.value)))).collect()
}

/// The facts as label / value lines, split into `columns` columns.
pub(super) fn facts_table(rows: &[(SharedString, String)], columns: usize) -> Div {
    let per = rows.len().div_ceil(columns.max(1)).max(1);
    div().flex().gap(u(40.)).children(rows.chunks(per).map(|chunk| {
        div().flex().flex_col().flex_1().min_w_0().gap(u(8.)).children(chunk.iter().map(|(label, value)| {
            div()
                .flex()
                .gap(u(14.))
                .child(div().flex_none().w(u(140.)).text_size(u(14.)).text_color(theme::text_dim()).child(label.clone()))
                .child(div().flex_1().min_w_0().text_size(u(14.)).text_color(theme::text_primary()).child(value.clone()))
        }))
    }))
}

/// "Main story 12h" and the like, when How Long To Beat knows the game.
pub(super) fn hltb_line(f: &FocusVm) -> Option<Div> {
    let h = f.hltb.as_ref()?;
    let fmt = |v: Option<f32>| v.map_or("\u{2014}".to_string(), |h| format!("{h:.0}h"));
    Some(
        div()
            .flex()
            .gap(u(24.))
            .text_size(u(14.))
            .text_color(theme::text_muted())
            .child(tr!("Main story {hours}", hours = fmt(h.main)))
            .child(tr!("Main + extras {hours}", hours = fmt(h.extra)))
            .child(tr!("Completionist {hours}", hours = fmt(h.complete))),
    )
}

/// A small card: a caption over a value.
pub(super) fn stat_card(label: &str, value: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(6.))
        .w(u(220.))
        .p(u(16.))
        .rounded(u(14.))
        .bg(kit::shade(0.35))
        .border_1()
        .border_color(kit::ink(0.08))
        .child(kit::kicker(label))
        .child(div().text_size(u(22.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(value.into()))
}

/// The notes of a journal card: its date and text.
pub(super) fn note_card(date: &str, text: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(6.))
        .flex_1()
        .min_w_0()
        .p(u(16.))
        .rounded(u(14.))
        .bg(kit::shade(0.35))
        .border_1()
        .border_color(kit::ink(0.08))
        .child(kit::kicker(date))
        .child(div().text_size(u(15.)).line_height(u(22.)).text_color(theme::text_primary()).line_clamp(4).child(text.to_string()))
}

impl CollectionView {
    /// How many screenshots of `key` are listed.
    pub(super) fn game_shot_count(&self, key: &SharedString) -> usize {
        if self.shots.game.as_ref() == Some(key) {
            self.shots.items.len()
        } else {
            0
        }
    }

    /// The notes with words of `key`'s journal, newest first (`(date, text)`).
    pub(super) fn game_notes(&self, key: &SharedString, max: usize) -> Vec<(String, String)> {
        let Some(data) = self.journal.data.as_ref().filter(|_| self.journal.is_for(key)) else { return Vec::new() };
        let this_year = Local::now().year();
        data.entries
            .iter()
            .rev()
            .filter(|e| !e.is_moment())
            .filter_map(|e| {
                let at = Local.timestamp_millis_opt(e.at_ms()?).single()?.date_naive();
                let month = heronite_i18n::month_short(at.month());
                let date = if at.year() == this_year {
                    tr!("{month} {d}", month, d = at.day())
                } else {
                    tr!("{month} {d}, {year}", month, d = at.day(), year = at.year())
                };
                Some((date.to_string(), e.text.trim().to_string()))
            })
            .take(max)
            .collect()
    }

    /// A screenshot thumbnail `w`×`h` design px (its picture loads when drawn).
    pub(super) fn shot_thumb(&mut self, i: usize, w: f32, h: f32, focused: bool) -> Div {
        let src = self.shots.items.get(i).map(|s| s.path.clone()).and_then(|p| self.shots.thumb(&p));
        div()
            .relative()
            .flex_none()
            .w(u(w))
            .h(u(h))
            .rounded(u(12.))
            .overflow_hidden()
            .bg(kit::ink(0.08))
            .border_3()
            .border_color(if focused { theme::console_focus().into() } else { transparent_black() })
            .when(focused, |d| d.shadow(kit::shadow(0.5, 8., 24.)))
            .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
    }

    /// Opens screenshot `i` full screen.
    pub(super) fn shot_open(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.shots_open_view(i, window, cx);
        // Decoded for the whole window, drawn by `shot_viewer`.
        self.shots.fit = true;
    }

    /// The open screenshot over the page.
    pub(super) fn shot_viewer(&self) -> Option<AnyElement> {
        let i = self.shots.view?;
        let path = &self.shots.items.get(i)?.path;
        let shown = self.shots.full.as_ref().filter(|f| f.path == *path).map(|f| f.img.clone());
        let picture = match shown {
            Some(tex) => img(ImageSource::Render(tex)).size_full().object_fit(ObjectFit::Contain).into_any_element(),
            None => crate::ui::icon(Icon::FaSpinner, u(28.), theme::white_a(0.7))
                .with_animation("shot-loading", Animation::new(Duration::from_secs(1)).repeat().with_max_fps(30.), |s, t| {
                    s.with_transformation(Transformation::rotate(percentage(t)))
                })
                .into_any_element(),
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::black_a(0.94))
                .child(picture)
                .child(
                    div()
                        .absolute()
                        .bottom(u(70.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .text_size(u(14.))
                        .text_color(theme::white_a(0.78))
                        .child(format!("{}/{}", i + 1, self.shots.items.len())),
                )
                .into_any_element(),
        )
    }

    /// While a screenshot is open every press is for it: ◀ ▶ browse, A / B close.
    pub(super) fn shot_viewer_pad(&mut self, pad: Pad) -> bool {
        let Some(i) = self.shots.view else { return false };
        let n = self.shots.items.len().max(1);
        match pad {
            Pad::Left | Pad::Right => {
                let next = (i as isize + if pad == Pad::Right { 1 } else { -1 }).rem_euclid(n as isize) as usize;
                self.shots.view = Some(next);
                self.shots.reset_transform();
                self.shots.discard_full();
                self.console.game.hero.shot = next;
                self.console.game.tabs.focus_shot(next);
            }
            Pad::A | Pad::B => {
                self.shots.fit = false;
                self.shots.view = None;
                self.shots.reset_transform();
                self.shots.discard_full();
            }
            _ => {}
        }
        true
    }

    /// Hints while a screenshot is open.
    pub(super) fn shot_viewer_hints(&self) -> Option<Vec<Hint>> {
        self.shots.view.map(|_| vec![hint(Btn::LeftRight, tr!("Browse")), hint(Btn::B, tr!("Close"))])
    }

    /// Tabs of the page: Screenshots and Journal only with something in them.
    fn hero_tabs(&self, key: &SharedString) -> Vec<Tab> {
        let mut tabs = vec![Tab::Overview, Tab::Details];
        if self.game_shot_count(key) > 0 {
            tabs.push(Tab::Screenshots);
        }
        if !self.game_notes(key, 1).is_empty() {
            tabs.push(Tab::Journal);
        }
        tabs
    }

    pub(super) fn render_game_hero(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return self.render_game_classic(key, window, cx) };
        self.console_focus_game(Some(key.clone()), cx);
        let rows = self.game_rows(&g, cx);
        let btns = buttons(&rows);
        let tabs = self.hero_tabs(key);
        let shots = self.game_shot_count(key);
        let nav = &mut self.console.game.hero;
        nav.button = nav.button.min(btns.len() - 1);
        if !tabs.contains(&nav.tab) {
            nav.tab = Tab::Overview;
        }
        // Only Screenshots has anything to focus under the tabs.
        nav.content &= nav.tab == Tab::Screenshots && shots > 0;
        nav.shot = nav.shot.min(shots.saturating_sub(1));
        nav.shot_start = window_start(nav.shot_start, nav.shot, SHOTS).min(shots.saturating_sub(SHOTS));
        let (tab, button, in_content, shot, shot_start) = (nav.tab, nav.button, nav.content, nav.shot, nav.shot_start);
        let on_buttons = !in_content && nav.more.is_none();

        let name = self.console_name(&g, HeroSize { title: 58., logo: (520., 128.), ..HeroSize::default() }, window);
        let facts = div()
            .flex()
            .items_center()
            .gap(u(12.))
            .text_size(u(16.))
            .text_color(theme::text_muted())
            .child(Self::console_facts(&g))
            .children(self.console_status_badge(&g));
        let buttons_row = div()
            .flex()
            .gap(u(12.))
            .mt(u(8.))
            .children(btns.iter().enumerate().map(|(i, b)| hero_button(*b, &rows, &g, on_buttons && i == button)));
        let names: Vec<SharedString> = tabs.iter().map(|t| SharedString::from(t.label())).collect();
        let active = tabs.iter().position(|t| *t == tab).unwrap_or(0);
        let tab_strip = div().flex().mt(u(14.)).child(kit::tabs(&names, active));

        let focus = self.focus.clone().filter(|f| f.key == *key);
        let content = match tab {
            Tab::Overview => self.hero_overview(&g, focus.as_ref()).into_any_element(),
            Tab::Details => {
                let about = self.console.game.about.handle().clone();
                let table = focus.as_ref().map(|f| facts_table(&detail_facts(f), 2));
                div()
                    .id("console-game-hero-details")
                    .max_w(u(900.))
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&about)
                    .children(table)
                    .child(self.console.game.about.driver())
                    .into_any_element()
            }
            Tab::Screenshots => {
                let mut row = div().flex().gap(u(14.));
                for i in shot_start..(shot_start + SHOTS).min(shots) {
                    row = row.child(self.shot_thumb(i, 236., 133., in_content && i == shot));
                }
                row.into_any_element()
            }
            Tab::Journal => div()
                .flex()
                .gap(u(14.))
                .max_w(u(900.))
                .children(self.game_notes(key, 2).iter().map(|(date, text)| note_card(date, &clip(text, 220))))
                .into_any_element(),
        };

        let page = div()
            .flex()
            .flex_col()
            .justify_end()
            .size_full()
            .px(u(56.))
            .pb(u(24.))
            .gap(u(12.))
            .child(div().flex().child(name))
            .child(facts)
            .child(buttons_row)
            .child(tab_strip)
            .child(div().h(u(CONTENT_H)).flex_none().overflow_hidden().child(content));

        let more = self.console.game.hero.more.as_mut().map(|list| {
            let more = more_rows(&rows);
            let body = div()
                .flex()
                .flex_col()
                .size_full()
                .child(div().px(u(34.)).pt(u(30.)).pb(u(10.)).child(kit::title(tr!("More"), 26.)))
                .child(div().flex_1().min_h_0().px(u(12.)).pb(u(10.)).child(list.render("console-game-more", &more, true)));
            side_sheet("console-game-more-panel", body)
        });
        div().relative().size_full().child(page).children(more).children(self.shot_viewer()).into_any_element()
    }

    /// Playtime, last played and time to beat over the start of the overview.
    fn hero_overview(&self, g: &CollectionGame, f: Option<&FocusVm>) -> Div {
        let dash = || "\u{2014}".to_string();
        let played = if g.playtime_minutes > 0 { heronite_data::collection::format_playtime_minutes(g.playtime_minutes) } else { dash() };
        let last = f.map_or_else(dash, |f| f.last_played.as_ref().map_or_else(|| tr!("Never").to_string(), |l| l.to_string()));
        let beat = f.and_then(|f| f.hltb.as_ref()?.main).map_or_else(dash, |h| format!("~{h:.0}h"));
        div()
            .flex()
            .flex_col()
            .gap(u(14.))
            .child(
                div()
                    .flex()
                    .gap(u(14.))
                    .child(stat_card(tr!("Playtime"), played))
                    .child(stat_card(tr!("Last played"), last))
                    .child(stat_card(tr!("Time to beat"), beat)),
            )
            .children(f.and_then(|f| f.overview.clone()).map(|o| kit::body_text(clip(&o, 360)).max_w(u(820.)).line_clamp(2)))
    }

    pub(super) fn game_pad_hero(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.shot_viewer_pad(pad) {
            return true;
        }
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return false };
        if pad == Pad::X {
            self.primary_action(key, cx);
            return true;
        }
        if matches!(pad, Pad::ScrollUp | Pad::ScrollDown) {
            return self.game_pad_classic(key, pad, window, cx);
        }
        let rows = self.game_rows(&g, cx);

        if let Some(list) = self.console.game.hero.more.as_mut() {
            let more = more_rows(&rows);
            match list.on_pad(pad, &more) {
                Outcome::Activate(id) | Outcome::Changed(id, _) => self.game_press_row(key, &id, pad, window, cx),
                Outcome::Ignored if pad == Pad::B => self.console.game.hero.more = None,
                _ => {}
            }
            return true;
        }

        let btns = buttons(&rows);
        let tabs = self.hero_tabs(key);
        let shots = self.game_shot_count(key);
        let nav = &mut self.console.game.hero;
        match pad {
            Pad::LB | Pad::RB => {
                let at = tabs.iter().position(|t| *t == nav.tab).unwrap_or(0);
                nav.tab = tabs[step(at, tabs.len(), if pad == Pad::RB { 1 } else { -1 }, true)];
                nav.content = false;
                nav.shot = 0;
                nav.shot_start = 0;
                self.console.game.about.reset();
            }
            Pad::Left | Pad::Right if !nav.content => {
                nav.button = step(nav.button, btns.len(), if pad == Pad::Right { 1 } else { -1 }, false);
            }
            Pad::Down if !nav.content && nav.tab == Tab::Screenshots && shots > 0 => nav.content = true,
            Pad::Left | Pad::Right => nav.shot = step(nav.shot, shots, if pad == Pad::Right { 1 } else { -1 }, false),
            Pad::Up if nav.content => nav.content = false,
            Pad::A if nav.content => {
                let i = nav.shot;
                self.shot_open(i, window, cx);
            }
            Pad::A => match btns[nav.button.min(btns.len() - 1)] {
                Button::More => nav.more = Some(ListNav::default()),
                b => {
                    let id = b.row().unwrap_or_default();
                    self.game_press_row(key, id, Pad::A, window, cx);
                }
            },
            _ => return false,
        }
        true
    }

    pub(super) fn game_hints_hero(&self, key: &SharedString) -> Vec<Hint> {
        if let Some(h) = self.shot_viewer_hints() {
            return h;
        }
        if self.console.game.hero.more.is_some() {
            return vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change")), hint(Btn::B, tr!("Close"))];
        }
        let primary = self.lib.as_ref().and_then(|l| l.get(key)).map(|g| self.primary_label(g)).unwrap_or(tr!("Play"));
        let mut h = vec![hint(Btn::A, tr!("Select"))];
        if self.hero_tabs(key).len() > 1 {
            h.push(hint(Btn::Bumpers, tr!("Tabs")));
        }
        h.extend([hint(Btn::X, primary), hint(Btn::B, tr!("Back")), hint(Btn::Start, tr!("Menu"))]);
        h
    }
}

/// One button of the row, from the game's action rows.
fn hero_button(b: Button, rows: &[Row], g: &CollectionGame, focused: bool) -> Div {
    let row = b.row().and_then(|id| rows.iter().find(|r| r.id.as_ref() == id));
    let fg: Hsla = if focused { theme::console_on_focus().into() } else { theme::text_primary().into() };
    let (ic, label, tint): (Icon, Option<SharedString>, Option<Hsla>) = match b {
        Button::Primary => (row.and_then(|r| r.icon).unwrap_or(Icon::PlayArrow), row.map(|r| r.label.clone()), None),
        Button::Status => {
            let name = row.and_then(|r| match &r.value {
                Value::Choice { options, index } => options.get(*index).cloned(),
                _ => None,
            });
            (Icon::Flag, name, None)
        }
        Button::Favourite => {
            let gold: Hsla = rgb(0xf4d35e).into();
            if g.favourite {
                (Icon::Favorite, None, Some(gold))
            } else {
                (Icon::FavoriteBorder, None, None)
            }
        }
        Button::Settings => (Icon::Tune, None, None),
        Button::More => (Icon::Menu, None, None),
    };
    let wide = label.is_some();
    let button = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(u(10.))
        .h(u(52.))
        .rounded_full()
        .bg(kit::ink(0.12))
        .border_1()
        .border_color(kit::ink(0.08))
        .text_size(u(18.))
        .font_weight(theme::W_BOLD)
        .text_color(fg)
        .when(wide, |d| d.px(u(26.)))
        .when(!wide, |d| d.w(u(52.)));
    let button = if focused { button.child(kit::focus_pill(26.)) } else { button };
    button.child(crate::ui::icon(ic, u(24.), tint.filter(|_| !focused).unwrap_or(fg))).children(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    #[test]
    fn the_buttons_follow_the_available_actions() {
        let play = Row::action("primary", "Play");
        let fav = Row::toggle("favourite", "Favourite", false);
        assert_eq!(buttons(&[play.clone(), fav.clone()]), [Button::Primary, Button::Favourite, Button::More]);
        let all = [play, Row::choice("status", "Status", vec!["a".into()], 0), fav, Row::action("settings", "Game settings")];
        assert_eq!(buttons(&all), [Button::Primary, Button::Status, Button::Favourite, Button::Settings, Button::More]);
    }

    #[test]
    fn more_lists_what_the_buttons_do_not() {
        let rows = [
            Row::action("primary", "Play"),
            Row::action("ua:1", "Mod"),
            Row::toggle("favourite", "Favourite", true),
            Row::header("Manage"),
        ];
        let ids: Vec<_> = more_rows(&rows).iter().map(|r| r.id.to_string()).collect();
        assert_eq!(ids, ["ua:1", "h:Manage"]);
    }

    #[test]
    fn the_window_moves_only_to_keep_the_focus_in_sight() {
        assert_eq!(window_start(0, 2, 4), 0);
        assert_eq!(window_start(0, 4, 4), 1);
        assert_eq!(window_start(3, 1, 4), 1);
        assert_eq!(window_start(3, 5, 4), 3);
    }
}
