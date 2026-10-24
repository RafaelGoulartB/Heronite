//! The game page as Tabs: a bar with the game's small cover and name and
//! its tabs (LB / RB): Play (big cover and the main actions), About (facts
//! and the full overview, the right stick scrolls), Screenshots and Journal
//! (only when the game has some) and Manage (the rest of the actions).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::layouts::step;
use super::hero::{detail_facts, facts_table, hltb_line, note_card, window_start};
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Pad};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Tab {
    #[default]
    Play,
    About,
    Screenshots,
    Journal,
    Manage,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Play => tr!("Play"),
            Tab::About => tr!("About"),
            Tab::Screenshots => tr!("Screenshots"),
            Tab::Journal => tr!("Journal"),
            Tab::Manage => tr!("Manage"),
        }
    }
}

#[derive(Default)]
pub(super) struct Nav {
    tab: Tab,
    play: ListNav,
    manage: ListNav,
    /// Focused screenshot, and the first row of thumbnails shown.
    shot: usize,
    shot_row: usize,
}

impl Nav {
    /// The screenshot the viewer moved to.
    pub(super) fn focus_shot(&mut self, i: usize) {
        self.shot = i;
    }
}

/// Thumbnails per row, and rows shown.
const COLS: usize = 3;
const ROWS: usize = 2;
/// Notes listed on the Journal tab.
const NOTES: usize = 6;

/// The actions before the "Manage" header, and those after it.
fn split_rows(mut rows: Vec<Row>) -> (Vec<Row>, Vec<Row>) {
    let manage = tr!("Manage");
    match rows.iter().position(|r| r.value == Value::Header && r.label.as_ref() == manage) {
        Some(i) => {
            let after = rows.split_off(i + 1);
            rows.pop();
            (rows, after)
        }
        None => (rows, Vec::new()),
    }
}

/// `focus` moved by a press in a grid of `n` items, `COLS` to a row.
fn grid_step(focus: usize, n: usize, pad: Pad) -> usize {
    let to = match pad {
        Pad::Left => focus.saturating_sub(1),
        Pad::Right => focus + 1,
        Pad::Up => focus.saturating_sub(COLS),
        Pad::Down => focus + COLS,
        _ => focus,
    };
    if to < n {
        to
    } else {
        focus
    }
}

impl CollectionView {
    fn game_tabs(&self, key: &SharedString) -> Vec<Tab> {
        let mut tabs = vec![Tab::Play, Tab::About];
        if self.game_shot_count(key) > 0 {
            tabs.push(Tab::Screenshots);
        }
        if !self.game_notes(key, 1).is_empty() {
            tabs.push(Tab::Journal);
        }
        tabs.push(Tab::Manage);
        tabs
    }

    pub(super) fn render_game_tabs(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return self.render_game_classic(key, window, cx) };
        self.console_focus_game(Some(key.clone()), cx);
        let tabs = self.game_tabs(key);
        if !tabs.contains(&self.console.game.tabs.tab) {
            self.console.game.tabs.tab = Tab::Play;
        }
        let tab = self.console.game.tabs.tab;
        let names: Vec<SharedString> = tabs.iter().map(|t| SharedString::from(t.label())).collect();
        let active = tabs.iter().position(|t| *t == tab).unwrap_or(0);

        let s = kit::scale();
        let small = self.cover(key, 30. * s, 30. * self.cover_aspect * s, window.scale_factor(), 300);
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(36.))
            .h(u(76.))
            .px(u(46.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(12.))
                    .min_w_0()
                    .max_w(u(300.))
                    .child(
                        div()
                            .flex_none()
                            .w(u(30.))
                            .h(u(30. * self.cover_aspect))
                            .rounded(u(6.))
                            .overflow_hidden()
                            .bg(theme::white_a(0.08))
                            .children(small.map(|c| img(c).size_full().object_fit(ObjectFit::Cover))),
                    )
                    .child(kit::title(g.title.clone(), 18.).truncate()),
            )
            .child(kit::tabs(&names, active))
            .child(div().flex_1())
            .child(kit::clock());

        let content = match tab {
            Tab::Play | Tab::Manage => {
                let (play, manage) = split_rows(self.game_rows(&g, cx));
                if tab == Tab::Play {
                    self.tabs_play(key, &g, &play, window)
                } else {
                    let list = self.console.game.tabs.manage.render("console-game-manage", &manage, true);
                    div().flex().flex_col().w(u(640.)).px(u(46.)).pt(u(18.)).flex_1().min_h_0().child(list.flex_1().min_h_0())
                }
            }
            Tab::About => self.tabs_about(key),
            Tab::Screenshots => self.tabs_shots(key),
            Tab::Journal => self.tabs_journal(key),
        };
        div()
            .relative()
            .size_full()
            .child(div().flex().flex_col().size_full().child(bar).child(content))
            .children(self.shot_viewer())
            .into_any_element()
    }

    /// The big cover with the status and playtime, the title and the main actions.
    fn tabs_play(&mut self, key: &SharedString, g: &heronite_data::CollectionGame, rows: &[Row], window: &mut Window) -> Div {
        let s = kit::scale();
        let (cw, ch) = (220., 220. * self.cover_aspect);
        let cover = self.cover(key, cw * s, ch * s, window.scale_factor(), 300);
        let list = self.console.game.tabs.play.render("console-game-play", rows, true);
        let left = div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(u(12.))
            .w(u(cw))
            .child(
                div()
                    .w(u(cw))
                    .h(u(ch))
                    .rounded(u(14.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.08))
                    .shadow(kit::shadow(0.55, 16., 40.))
                    .children(cover.map(|c| img(c).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(10.))
                    .text_size(u(14.))
                    .text_color(theme::text_muted())
                    .children(self.console_status_badge(g))
                    .child(if g.playtime_minutes > 0 {
                        heronite_data::collection::format_playtime_minutes(g.playtime_minutes)
                    } else {
                        String::new()
                    }),
            );
        div().flex().flex_1().min_h_0().gap(u(40.)).px(u(46.)).pt(u(18.)).child(left).child(
            div()
                .flex()
                .flex_col()
                .w(u(600.))
                .min_h_0()
                .child(div().pb(u(14.)).child(kit::title(g.title.clone(), 40.)))
                .child(list.flex_1().min_h_0()),
        )
    }

    fn tabs_about(&mut self, key: &SharedString) -> Div {
        let focus = self.focus.clone().filter(|f| f.key == *key);
        let about = self.console.game.about.handle().clone();
        let facts = focus.as_ref().map(|f| {
            div()
                .flex()
                .flex_none()
                .flex_col()
                .gap(u(14.))
                .w(u(480.))
                .p(u(22.))
                .rounded(u(18.))
                .bg(theme::black_a(0.35))
                .border_1()
                .border_color(theme::white_a(0.08))
                .child(facts_table(&detail_facts(f), 1))
                .children(hltb_line(f))
        });
        let text = focus.as_ref().and_then(|f| f.overview.clone()).map(|o| div().flex_1().min_w_0().child(kit::body_text(o)));
        let scroll = div()
            .id("console-game-about-tab")
            .flex()
            .items_start()
            .gap(u(30.))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&about)
            .children(facts)
            .children(text)
            .child(self.console.game.about.driver());
        div().flex_1().min_h_0().px(u(46.)).pt(u(18.)).pb(u(10.)).child(scroll)
    }

    fn tabs_shots(&mut self, key: &SharedString) -> Div {
        let n = self.game_shot_count(key);
        let nav = &mut self.console.game.tabs;
        nav.shot = nav.shot.min(n.saturating_sub(1));
        let total = n.div_ceil(COLS);
        nav.shot_row = window_start(nav.shot_row, nav.shot / COLS, ROWS).min(total.saturating_sub(ROWS));
        let (focus, first) = (nav.shot, nav.shot_row);
        // 1280 design px wide, 46 of margin each side, 16 between.
        let w = (1280. - 92. - 16. * (COLS as f32 - 1.)) / COLS as f32;
        let mut grid = div().flex().flex_col().gap(u(16.)).px(u(46.)).pt(u(18.));
        for r in first..(first + ROWS).min(total) {
            let mut row = div().flex().gap(u(16.));
            for i in r * COLS..((r + 1) * COLS).min(n) {
                row = row.child(self.shot_thumb(i, w, w * 9. / 16., i == focus));
            }
            grid = grid.child(row);
        }
        grid.flex_1().min_h_0()
    }

    fn tabs_journal(&self, key: &SharedString) -> Div {
        let notes = self.game_notes(key, NOTES);
        let rows = notes.chunks(2).map(|pair| {
            let mut row = div().flex().gap(u(16.));
            for (date, text) in pair {
                row = row.child(note_card(date, text));
            }
            // A lone note keeps the width of one.
            if pair.len() == 1 {
                row = row.child(div().flex_1());
            }
            row
        });
        div().flex().flex_col().gap(u(16.)).px(u(46.)).pt(u(18.)).flex_1().min_h_0().children(rows)
    }

    pub(super) fn game_pad_tabs(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
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
        let tabs = self.game_tabs(key);
        let tab = self.console.game.tabs.tab;
        if matches!(pad, Pad::LB | Pad::RB) {
            let at = tabs.iter().position(|t| *t == tab).unwrap_or(0);
            let nav = &mut self.console.game.tabs;
            nav.tab = tabs[step(at, tabs.len(), if pad == Pad::RB { 1 } else { -1 }, true)];
            nav.shot = 0;
            nav.shot_row = 0;
            self.console.game.about.reset();
            return true;
        }
        match tab {
            Tab::Play | Tab::Manage => {
                let (play, manage) = split_rows(self.game_rows(&g, cx));
                let nav = &mut self.console.game.tabs;
                let outcome = if tab == Tab::Play { nav.play.on_pad(pad, &play) } else { nav.manage.on_pad(pad, &manage) };
                match outcome {
                    Outcome::Activate(id) | Outcome::Changed(id, _) => self.game_press_row(key, &id, pad, window, cx),
                    Outcome::Moved | Outcome::Edge(_) => {}
                    Outcome::Ignored => return false,
                }
                true
            }
            Tab::Screenshots => {
                let n = self.game_shot_count(key);
                let nav = &mut self.console.game.tabs;
                match pad {
                    Pad::Left | Pad::Right | Pad::Up | Pad::Down => nav.shot = grid_step(nav.shot, n, pad),
                    Pad::A if n > 0 => {
                        let i = nav.shot.min(n - 1);
                        self.shot_open(i, window, cx);
                    }
                    _ => return false,
                }
                true
            }
            Tab::About | Tab::Journal => false,
        }
    }

    pub(super) fn game_hints_tabs(&self, key: &SharedString) -> Vec<Hint> {
        if let Some(h) = self.shot_viewer_hints() {
            return h;
        }
        let primary = self.lib.as_ref().and_then(|l| l.get(key)).map(|g| self.primary_label(g)).unwrap_or(tr!("Play"));
        let mut h = Vec::new();
        match self.console.game.tabs.tab {
            Tab::Play | Tab::Manage => h.extend([hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change"))]),
            Tab::Screenshots => h.push(hint(Btn::A, tr!("Open"))),
            Tab::About | Tab::Journal => {}
        }
        h.extend([hint(Btn::Bumpers, tr!("Tabs")), hint(Btn::X, primary), hint(Btn::B, tr!("Back")), hint(Btn::Start, tr!("Menu"))]);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_split_at_the_manage_header() {
        let rows = vec![Row::action("primary", "Play"), Row::header(tr!("Manage")), Row::action("x:ludusavi", "Back up")];
        let (play, manage) = split_rows(rows);
        assert_eq!((play.len(), manage.len()), (1, 1));
        assert_eq!(manage[0].id.as_ref(), "x:ludusavi");
        let (all, none) = split_rows(vec![Row::action("primary", "Play")]);
        assert_eq!((all.len(), none.len()), (1, 0));
    }

    #[test]
    fn the_grid_moves_by_row_and_stays_inside() {
        assert_eq!(grid_step(0, 7, Pad::Down), 3);
        assert_eq!(grid_step(4, 7, Pad::Down), 4);
        assert_eq!(grid_step(4, 7, Pad::Up), 1);
        assert_eq!(grid_step(0, 7, Pad::Left), 0);
        assert_eq!(grid_step(6, 7, Pad::Right), 6);
    }
}
