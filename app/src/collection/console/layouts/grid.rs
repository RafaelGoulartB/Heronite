//! Grid: every game of the tab in a cover grid, with Show / Sort / Status /
//! Category / Search filters on top (↑ from the first row; Y opens them all
//! in the Filters panel, `super::super::filters`). Its filters are
//! console-only state: they never change the desktop Collection's filters.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::categories::{Categories, UNCATEGORIZED};
use heronite_data::console_theme::{GridPanel, TabSpec};
use heronite_data::{CollectionGame, Library};
use heronite_i18n::{n_, tr};

use super::super::super::CollectionView;
use super::super::hero::{clip, HeroSize};
use super::super::sources::keys;
use super::super::tile::label_room;
use crate::collection::data::played_ago;
use crate::collection::view_filters::category_label;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::{Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

pub(crate) const GAP: f32 = 18.;
pub(crate) const SIDE: f32 = 46.;
pub(crate) const PAD: f32 = 10.;

pub(crate) struct Nav {
    /// Focused filter chip (`None` = the grid has the focus).
    pub toolbar: Option<usize>,
    pub index: usize,
    pub filters: Filters,
    pub scroll: Glide,
    pub reveal: bool,
    /// The filters changed: back to the top at once.
    pub restart: bool,
    pub cols: usize,
}

impl Default for Nav {
    fn default() -> Self {
        Self { toolbar: None, index: 0, filters: Filters::default(), scroll: Glide::new(Axis::Y), reveal: false, restart: false, cols: 6 }
    }
}

/// What a Grid shows (its chips and the Filters panel edit it).
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct Filters {
    /// Index in [`SHOW`].
    pub show: usize,
    /// Only the favourites (on top of `show`).
    pub favourites: bool,
    /// Index in [`SORT`].
    pub sort: usize,
    /// 0 = any, else the library's status `status - 1`.
    pub status: usize,
    /// Index in [`category_options`].
    pub category: usize,
}

pub(crate) const SHOW: [&str; 3] = [n_!("All games"), n_!("Installed"), n_!("Not installed")];
const SORT: [&str; 3] = [n_!("A\u{2013}Z"), n_!("Last played"), n_!("Most played")];
/// The steps of the Show chip (`show`, `favourites`): all, installed,
/// favourites, not installed.
const SHOW_STEPS: [(usize, bool); 4] = [(0, false), (1, false), (0, true), (2, false)];
/// Chips of the filter bar (the last one is Search).
const CHIPS: usize = 5;

/// The Category chip's values: any, each category, then the games in none
/// (only "any" while there are no categories).
pub(crate) fn category_options(cats: &Categories) -> Vec<Option<String>> {
    let mut v = vec![None];
    if !cats.is_empty() {
        v.extend(cats.names().map(|n| Some(n.to_string())));
        v.push(Some(UNCATEGORIZED.to_string()));
    }
    v
}

pub(crate) fn status_options(lib: &Library) -> Vec<SharedString> {
    std::iter::once(SharedString::from(tr!("Any status")))
        .chain(lib.statuses.iter().map(|s| SharedString::from(heronite_data::status::status_label(&s.name).to_string())))
        .collect()
}

impl Nav {
    /// Filters that show the games of a shelf of `kind` (`installed`,
    /// `favourites`, `status:<id>` with its index in the library's
    /// `status`...), the most recently played first, from the top.
    pub(super) fn show_like(&mut self, kind: &str, status: Option<usize>) {
        self.filters = Filters {
            show: usize::from(kind == "installed"),
            favourites: kind == "favourites",
            sort: 1,
            status: status.map_or(0, |i| i + 1),
            category: 0,
        };
        self.index = 0;
        self.toolbar = None;
        self.restart = true;
    }

    /// The games the grid shows, in order (best search matches first).
    fn keys(&self, games: &[std::sync::Arc<CollectionGame>], lib: &Library, cats: &Categories, search: &str) -> Vec<SharedString> {
        let f = &self.filters;
        let folded = heronite_data::fuzzy::fold(search.trim());
        let tokens = heronite_data::fuzzy::tokens(&folded);
        let status = f.status.checked_sub(1).and_then(|i| lib.statuses.get(i)).map(|s| s.id.clone());
        let category: std::collections::BTreeSet<String> = category_options(cats).get(f.category).cloned().flatten().into_iter().collect();
        let mut games: Vec<&CollectionGame> = games
            .iter()
            .map(|g| g.as_ref())
            .filter(|g| match f.show {
                1 => g.installed,
                2 => !g.installed,
                _ => true,
            })
            .filter(|g| !f.favourites || g.favourite)
            .filter(|g| status.as_ref().is_none_or(|s| &g.status_id == s))
            .filter(|g| cats.matches(&category, &g.runner, &g.app_name))
            .collect();
        match f.sort {
            1 => games.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower))),
            2 => games.sort_by(|a, b| b.playtime_minutes.cmp(&a.playtime_minutes).then_with(|| a.title_lower.cmp(&b.title_lower))),
            _ => games.sort_by(|a, b| a.title_lower.cmp(&b.title_lower)),
        }
        if !tokens.is_empty() {
            // Same search as the desktop Collection; stable, so the sort orders ties.
            let mut scored: Vec<(i32, &CollectionGame)> =
                games.into_iter().filter_map(|g| heronite_data::collection::search_score(g, &tokens).map(|s| (s, g))).collect();
            scored.sort_by_key(|s| std::cmp::Reverse(s.0));
            games = scored.into_iter().map(|(_, g)| g).collect();
        }
        keys(games)
    }

    fn chips(&self, lib: &Library, cats: &Categories, search: &str) -> [(SharedString, SharedString); CHIPS] {
        let f = &self.filters;
        let status = status_options(lib);
        let category = match category_options(cats).get(f.category).cloned().flatten() {
            None => tr!("Any category").to_string(),
            Some(name) => category_label(&name),
        };
        let show = match (f.show, f.favourites) {
            (0, true) => tr!("Favourites").to_string(),
            (s, true) => format!("{} \u{b7} {}", heronite_i18n::tr(SHOW[s]), tr!("Favourites")),
            (s, false) => heronite_i18n::tr(SHOW[s]).to_string(),
        };
        [
            (tr!("Show").into(), show.into()),
            (tr!("Sort").into(), heronite_i18n::tr(SORT[f.sort]).into()),
            (tr!("Status").into(), status.get(f.status).cloned().unwrap_or_default()),
            (tr!("Category").into(), category.into()),
            (tr!("Search").into(), if search.is_empty() { "\u{2014}".into() } else { search.to_string().into() }),
        ]
    }
}

/// Width of the rail of letters on the right edge, design px.
const RAIL: f32 = 30.;

/// The letters of the rail: `#` (digits and symbols), then A to Z.
fn rail_letters() -> impl Iterator<Item = char> {
    std::iter::once('#').chain('A'..='Z')
}

/// The rail letter of a title: its first letter without accents, `#` for
/// digits and symbols (they sort before A).
pub(super) fn letter_of(title: &str) -> char {
    let first = title.trim_start().chars().next().unwrap_or('#');
    heronite_data::fuzzy::fold(first.encode_utf8(&mut [0; 4]))
        .chars()
        .next()
        .filter(char::is_ascii_alphabetic)
        .map_or('#', |c| c.to_ascii_uppercase())
}

/// Where LT / RT land among games sorted A–Z whose letters are `letters`:
/// the first game of the next letter, or of the previous one (of the
/// current one when the focus is not on its first game). Stays put at the ends.
pub(super) fn jump_letter(letters: &[char], i: usize, forward: bool) -> usize {
    let n = letters.len();
    if n == 0 {
        return 0;
    }
    let i = i.min(n - 1);
    if forward {
        return (i + 1..n).find(|&j| letters[j] != letters[i]).unwrap_or(n - 1);
    }
    let mut j = i;
    if j > 0 && letters[j - 1] != letters[i] {
        j -= 1;
    }
    let l = letters[j];
    while j > 0 && letters[j - 1] == l {
        j -= 1;
    }
    j
}

/// Width of the game panel for a window `vw` design px wide.
fn panel_width(vw: f32) -> f32 {
    (vw * 0.31).round().min(380.)
}

/// A "label  value" line of the facts under the art.
pub(super) fn fact(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .gap(u(12.))
        .child(div().flex_none().w(u(112.)).text_color(theme::text_dim()).child(label.into()))
        .child(div().flex_1().min_w_0().truncate().text_color(theme::text_primary()).child(value.into()))
}

/// A small label over its value (the facts of the wheel's panel).
pub(super) fn fact_cell(label: impl Into<SharedString>, value: impl IntoElement) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(u(2.))
        .child(div().text_size(u(12.)).text_color(theme::text_dim()).child(label.into()))
        .child(value)
}

/// "~12 h" for HowLongToBeat's hours.
fn hours_text(hours: f64) -> String {
    let text = if hours < 10. { format!("{hours:.1}").trim_end_matches(".0").to_string() } else { format!("{hours:.0}") };
    tr!("~{hours} h", hours = text).to_string()
}

impl CollectionView {
    /// The keys the Grid of a tab shows now (also for the layouts that reuse its filters).
    pub(crate) fn grid_keys(&mut self, tab: usize, spec: &TabSpec) -> Vec<SharedString> {
        let Some(lib) = self.lib.clone() else { return Vec::new() };
        let games = self.console_games(&spec.source);
        let search = self.console.search.clone();
        let cats = std::mem::take(&mut self.categories);
        let keys = self.tab_nav(tab).grid.keys(&games, &lib, &cats, &search);
        self.categories = cats;
        keys
    }

    /// LT / RT jump by letter: the theme asks for it and the games are
    /// sorted A–Z (not a search, which orders by match).
    fn grid_letters(&self, tab: usize) -> bool {
        look::theme().layouts.jump_letters
            && self.console.search.is_empty()
            && self.console.tab_navs.get(tab).is_none_or(|n| n.grid.filters.sort == 0)
    }

    fn grid_cols(&self, window: &Window, panel: bool, rail: bool) -> usize {
        let (tw, _) = self.console_tile_size();
        let vw = f32::from(window.viewport_size().width) / kit::scale();
        let taken = if panel { panel_width(vw) + SIDE } else { 0. } + if rail { RAIL } else { 0. };
        (((vw - 2. * SIDE - taken + GAP) / (tw + GAP)).floor() as usize).max(2)
    }

    pub(super) fn render_grid(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(lib) = self.lib.clone() else { return div().into_any_element() };
        let panel = look::theme().layouts.grid_panel == GridPanel::Right;
        let rail = self.grid_letters(tab);
        let cols = self.grid_cols(window, panel, rail);
        let keys = self.grid_keys(tab, spec);
        let search = self.console.search.clone();
        let cats = std::mem::take(&mut self.categories);
        let nav = &mut self.tab_nav(tab).grid;
        let chips = nav.chips(&lib, &cats, &search);
        self.categories = cats;
        let nav = &mut self.tab_nav(tab).grid;
        nav.cols = cols;
        nav.index = nav.index.min(keys.len().saturating_sub(1));
        let grid_focus = nav.toolbar.is_none();
        if nav.restart {
            nav.restart = false;
            nav.scroll.reset();
        }
        let (_, th) = self.console_tile_size();
        let nav = &mut self.tab_nav(tab).grid;
        if nav.reveal && !keys.is_empty() {
            nav.reveal = false;
            // The focused row in sight with a part of the next one.
            nav.scroll.go(Goal::Reveal { ix: nav.index / cols, margin: th * 0.35 });
        }
        let (index, toolbar) = (nav.index, nav.toolbar);
        let focused = keys.get(index).cloned();
        self.console_focus_game(focused.clone(), cx);

        let bar =
            div().flex().items_center().gap(u(12.)).px(u(SIDE)).pb(u(16.)).children(chips.iter().enumerate().map(|(i, (label, value))| {
                let on = toolbar == Some(i);
                kit::row_base(on)
                    .min_h(u(46.))
                    .px(u(18.))
                    .when(!on, |d| d.bg(kit::ink(0.07)))
                    .child(
                        div()
                            .text_size(u(14.))
                            .text_color(if on { theme::console_on_focus() } else { theme::text_muted() })
                            .child(label.clone()),
                    )
                    .child(div().text_size(u(16.)).font_weight(theme::W_BOLD).max_w(u(260.)).truncate().child(value.clone()))
            }));
        let game = focused.as_ref().and_then(|k| lib.get(k)).cloned();
        // The focused game's name and facts over the grid, where it is in
        // the list at the right end.
        let title_line = game.as_ref().filter(|_| !panel).map(|g| {
            let facts = look::theme().hero.facts.then(|| self.console_fact_line(g, Self::console_facts(g), 15.).flex_none());
            let about = div()
                .relative()
                .flex()
                .items_center()
                .gap(u(18.))
                .min_w_0()
                .child(kit::title(g.title.clone(), 28.).min_w_0().truncate())
                .children(facts);
            div()
                .flex()
                .items_center()
                .gap(u(14.))
                .h(u(44.))
                .px(u(SIDE))
                .pb(u(6.))
                .child(div().flex_1().min_w_0().overflow_hidden().child(motion::enter_from(
                    SharedString::from(format!("lib-title-{}", g.key)),
                    about,
                    240,
                    10.,
                    0.,
                    0.3,
                )))
                .child(div().flex_none().text_size(u(15.)).text_color(theme::text_muted()).child(tr!(
                    "{index} of {total}",
                    index = index + 1,
                    total = keys.len()
                )))
        });
        let body = if keys.is_empty() {
            kit::empty(Icon::FaSearch, tr!("No games match"), tr!("Change the filters above or the search (Y).")).into_any_element()
        } else {
            let scroller = self.grid_scroller(tab, &keys, cols, grid_focus, window);
            let current = game.as_ref().map_or('#', |g| letter_of(&g.title));
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(scroller.flex_1().min_w_0())
                .when(rail, |d| d.child(letter_rail(current)))
                .into_any_element()
        };
        let list = div().flex().flex_col().flex_1().min_w_0().child(div().flex_none().children(title_line)).child(body);
        let side = game.filter(|_| panel && !keys.is_empty()).map(|g| {
            let w = panel_width(f32::from(window.viewport_size().width) / kit::scale());
            self.grid_panel(&g, w, window)
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(div().flex().flex_1().min_h_0().child(list).children(side.map(|p| div().flex_none().mr(u(SIDE)).pb(u(14.)).child(p))))
            .into_any_element()
    }

    /// The scrolling rows of covers; only those on screen (and around the
    /// focus) hold tiles, the others are empty boxes of the same height.
    fn grid_scroller(&mut self, tab: usize, keys: &[SharedString], cols: usize, grid_focus: bool, window: &Window) -> Stateful<Div> {
        let (tw, th) = self.console_tile_size();
        let line_h = th + GAP + label_room();
        let nav = &self.tab_nav(tab).grid;
        let index = nav.index;
        let glide = nav.scroll.clone();
        let scroll = glide.handle().clone();
        let mut grid =
            div().id(("console-grid", tab)).flex().flex_col().overflow_y_scroll().track_scroll(&scroll).px(u(SIDE - PAD)).pb(u(24.));
        let seen = super::in_sight(scroll.offset().y, line_h, window.viewport_size().height);
        for (r, chunk) in keys.chunks(cols).enumerate() {
            let near = seen.start().saturating_sub(1) <= r && r <= *seen.end();
            let visible = seen.contains(&r) || r.abs_diff(index / cols) <= 1;
            let mut line = div().flex_none().flex().gap(u(GAP)).h(u(line_h)).px(u(PAD)).pt(u(PAD));
            if near || r.abs_diff(index / cols) <= 1 {
                for (c, key) in chunk.iter().enumerate() {
                    line = line.child(self.console_tile(key, tw, th, grid_focus && r * cols + c == index, visible, window));
                }
            }
            grid = grid.child(line);
        }
        grid.child(glide.driver())
    }

    /// The focused game on the right of the grid: its wide art, name,
    /// facts, when it was last played, time to beat, genres, overview.
    fn grid_panel(&mut self, g: &CollectionGame, w: f32, window: &Window) -> AnyElement {
        let theme = look::theme();
        let key = SharedString::from(g.key.clone());
        let art = self.console_wide_art(&key, w, (w * 9. / 16.).round(), 200, window);
        let size = HeroSize { title: 26., logo: (w, 64.), max_w: w, overview: 0 };
        let name = self.console_name(g, size, window);
        let last = g.last_played.as_deref().and_then(played_ago).unwrap_or_else(|| tr!("Never").to_string());
        let beat =
            self.wiki.hltb(&g.title, &g.game.title).map(|h| h.main_story).filter(|h| *h > 0.).map_or_else(|| "\u{2014}".into(), hours_text);
        let genres = if g.details.genres.is_empty() { "\u{2014}".to_string() } else { g.details.genres.join(", ") };
        let overview = theme.hero.overview.then(|| self.console_overview(&g.key)).flatten();
        let panel = div()
            .relative()
            .w(u(w))
            .flex()
            .flex_col()
            .gap(u(12.))
            .child(
                div()
                    .flex_none()
                    .w(u(w))
                    .h(u((w * 9. / 16.).round()))
                    .rounded(u(14.))
                    .overflow_hidden()
                    .bg(kit::ink(0.07))
                    .shadow(kit::shadow(0.45, 14., 30.))
                    .children(art.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(name)
            .when(theme.hero.facts, |d| d.child(self.console_fact_line(g, Self::console_facts(g), 14.).flex_wrap().gap(u(10.))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(5.))
                    .text_size(u(13.5))
                    .child(fact(tr!("Last played"), last))
                    .child(fact(tr!("Time to beat"), beat))
                    .child(fact(tr!("Genre"), genres)),
            )
            .children(overview.map(|o| {
                motion::fade_in(
                    SharedString::from(format!("grid-about-{}", g.key)),
                    kit::body_text(clip(&o, 260)).text_size(u(13.5)).line_height(u(20.)).line_clamp(4),
                    260,
                )
            }));
        motion::enter_from(SharedString::from(format!("grid-panel-{}", g.key)), panel, 280, 0., 10., 0.3)
    }

    pub(super) fn grid_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(lib) = self.lib.clone() else { return false };
        let keys = self.grid_keys(tab, spec);
        let status_n = lib.statuses.len() + 1;
        let category_n = category_options(&self.categories).len();
        let wrap = look::theme().nav.wrap;
        let by_letter = self.grid_letters(tab);
        let nav = &mut self.tab_nav(tab).grid;
        let cols = nav.cols.max(1);
        if let Some(chip) = nav.toolbar {
            match pad {
                Pad::Left => nav.toolbar = Some(chip.saturating_sub(1)),
                Pad::Right => nav.toolbar = Some((chip + 1).min(CHIPS - 1)),
                Pad::Down if !keys.is_empty() => nav.toolbar = None,
                Pad::A => {
                    let f = &mut nav.filters;
                    match chip {
                        0 => {
                            let at = SHOW_STEPS.iter().position(|s| *s == (f.show, f.favourites)).map_or(0, |i| i + 1);
                            (f.show, f.favourites) = SHOW_STEPS[at % SHOW_STEPS.len()];
                        }
                        1 => f.sort = (f.sort + 1) % SORT.len(),
                        2 => f.status = (f.status + 1) % status_n,
                        3 => f.category = (f.category + 1) % category_n,
                        _ => {
                            self.console_search(window, cx);
                            return true;
                        }
                    }
                }
                Pad::X => {
                    let f = &mut nav.filters;
                    match chip {
                        0 => (f.show, f.favourites) = (0, false),
                        1 => f.sort = 0,
                        2 => f.status = 0,
                        3 => f.category = 0,
                        _ => self.console.search.clear(),
                    }
                }
                Pad::Y => {
                    self.open_console_filters(window, cx);
                    return true;
                }
                _ => return false,
            }
            if !matches!(pad, Pad::Left | Pad::Right | Pad::Down) {
                let nav = &mut self.tab_nav(tab).grid;
                nav.index = 0;
                nav.restart = true;
            }
            return true;
        }
        let n = keys.len();
        let i = nav.index.min(n.saturating_sub(1));
        match pad {
            Pad::Left | Pad::Right => nav.index = super::step(i, n, pad.dir(), wrap),
            Pad::Up if i < cols => nav.toolbar = Some(0),
            Pad::Up => nav.index = i - cols,
            Pad::Down => nav.index = (i + cols).min(n.saturating_sub(1)),
            Pad::RT | Pad::LT if by_letter => {
                let letters: Vec<char> = keys.iter().map(|k| lib.get(k).map_or('#', |g| letter_of(&g.title))).collect();
                nav.index = jump_letter(&letters, i, pad == Pad::RT);
            }
            Pad::ScrollDown | Pad::RT => nav.index = (i + cols * 3).min(n.saturating_sub(1)),
            Pad::ScrollUp | Pad::LT => nav.index = i.saturating_sub(cols * 3),
            Pad::A | Pad::X => {
                if let Some(k) = keys.get(i) {
                    self.console_activate(k.clone(), pad, window, cx);
                }
                return true;
            }
            Pad::Y => {
                self.open_console_filters(window, cx);
                return true;
            }
            _ => return false,
        }
        nav.reveal = true;
        true
    }

    pub(super) fn grid_hints(&self, tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        if self.console.tab_navs.get(tab).is_some_and(|n| n.grid.toolbar.is_some()) {
            let mut h = vec![
                crate::console::hint(Btn::A, tr!("Change")),
                crate::console::hint(Btn::X, tr!("Reset")),
                crate::console::hint(Btn::Y, tr!("Search")),
            ];
            h.extend(self.console_tab_hints());
            return h;
        }
        let mut h = self.console_cover_hints();
        if self.grid_letters(tab) {
            h.insert(h.len() - self.console_tab_hints().len(), crate::console::hint(Btn::LT, tr!("Letter")));
        }
        h
    }
}

/// The letters down the right edge, the current one lit.
fn letter_rail(current: char) -> Div {
    div().flex_none().w(u(RAIL)).h_full().flex().flex_col().py(u(6.)).children(rail_letters().map(|l| {
        let on = l == current;
        div().flex_1().min_h_0().flex().items_center().justify_center().child(
            div()
                .w(u(20.))
                .h(u(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_size(u(11.))
                .font_weight(theme::W_BOLD)
                .when(on, |d| d.bg(theme::console_focus()).text_color(theme::console_on_focus()))
                .when(!on, |d| d.text_color(theme::text_dim()))
                .child(l.to_string()),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    #[test]
    fn letters_ignore_accents_and_group_symbols() {
        assert_eq!(letter_of("Álvaro"), 'A');
        assert_eq!(letter_of("  zelda"), 'Z');
        assert_eq!(letter_of("1942"), '#');
        assert_eq!(letter_of("!Hola"), '#');
        assert_eq!(letter_of(""), '#');
    }

    #[test]
    fn letter_jumps_land_on_the_first_game_of_a_letter() {
        let l = ['#', 'A', 'A', 'B', 'B', 'B', 'C'];
        assert_eq!(jump_letter(&l, 1, true), 3);
        assert_eq!(jump_letter(&l, 4, true), 6);
        assert_eq!(jump_letter(&l, 6, true), 6);
        assert_eq!(jump_letter(&l, 5, false), 3);
        assert_eq!(jump_letter(&l, 3, false), 1);
        assert_eq!(jump_letter(&l, 2, false), 1);
        assert_eq!(jump_letter(&l, 1, false), 0);
        assert_eq!(jump_letter(&l, 0, false), 0);
        assert_eq!(jump_letter(&[], 0, true), 0);
    }
}
