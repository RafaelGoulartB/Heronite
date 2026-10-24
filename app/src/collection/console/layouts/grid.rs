//! Grid: every game of the tab in a cover grid, with Show / Sort / Status /
//! Category / Search filters on top (↑ from the first row). Its filters are
//! console-only state: they never change the desktop Collection's filters.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::categories::{Categories, UNCATEGORIZED};
use heronite_data::console_theme::TabSpec;
use heronite_data::{CollectionGame, Library};
use heronite_i18n::{n_, tr};

use super::super::super::CollectionView;
use super::super::backdrop::Scrim;
use super::super::sources::keys;
use super::super::tile::label_room;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::{Hint, Pad};
use crate::icons::Icon;
use crate::theme;

pub(crate) const GAP: f32 = 18.;
pub(crate) const SIDE: f32 = 46.;
pub(crate) const PAD: f32 = 10.;

pub(super) const SCRIM: Scrim = Scrim::Classic;

pub(crate) struct Nav {
    /// Focused filter chip (`None` = the grid has the focus).
    pub toolbar: Option<usize>,
    pub index: usize,
    show: usize,
    sort: usize,
    status: usize,
    /// Index in [`category_options`].
    category: usize,
    pub scroll: Glide,
    pub reveal: bool,
    /// The filters changed: back to the top at once.
    pub restart: bool,
    pub cols: usize,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            toolbar: None,
            index: 0,
            show: 0,
            sort: 0,
            status: 0,
            category: 0,
            scroll: Glide::new(Axis::Y),
            reveal: false,
            restart: false,
            cols: 6,
        }
    }
}

const SHOW: [&str; 4] = [n_!("All games"), n_!("Installed"), n_!("Favourites"), n_!("Not installed")];
const SORT: [&str; 3] = [n_!("A\u{2013}Z"), n_!("Last played"), n_!("Most played")];
/// Chips of the filter bar (the last one is Search).
const CHIPS: usize = 5;

/// The Category chip's values: any, each category, then the games in none
/// (only "any" while there are no categories).
fn category_options(cats: &Categories) -> Vec<Option<String>> {
    let mut v = vec![None];
    if !cats.is_empty() {
        v.extend(cats.names().map(|n| Some(n.to_string())));
        v.push(Some(UNCATEGORIZED.to_string()));
    }
    v
}

fn status_options(lib: &Library) -> Vec<SharedString> {
    std::iter::once(SharedString::from(tr!("Any status")))
        .chain(lib.statuses.iter().map(|s| SharedString::from(heronite_data::status::status_label(&s.name).to_string())))
        .collect()
}

impl Nav {
    /// The games the grid shows, in order (best search matches first).
    fn keys(&self, games: &[std::sync::Arc<CollectionGame>], lib: &Library, cats: &Categories, search: &str) -> Vec<SharedString> {
        let folded = heronite_data::fuzzy::fold(search.trim());
        let tokens = heronite_data::fuzzy::tokens(&folded);
        let status = self.status.checked_sub(1).and_then(|i| lib.statuses.get(i)).map(|s| s.id.clone());
        let category: std::collections::BTreeSet<String> =
            category_options(cats).get(self.category).cloned().flatten().into_iter().collect();
        let mut games: Vec<&CollectionGame> = games
            .iter()
            .map(|g| g.as_ref())
            .filter(|g| match self.show {
                1 => g.installed,
                2 => g.favourite,
                3 => !g.installed,
                _ => true,
            })
            .filter(|g| status.as_ref().is_none_or(|s| &g.status_id == s))
            .filter(|g| cats.matches(&category, &g.runner, &g.app_name))
            .collect();
        match self.sort {
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
        let status = status_options(lib);
        let category = match category_options(cats).get(self.category).cloned().flatten() {
            None => tr!("Any category").to_string(),
            Some(name) => super::super::super::view_filters::category_label(&name),
        };
        [
            (tr!("Show").into(), heronite_i18n::tr(SHOW[self.show]).into()),
            (tr!("Sort").into(), heronite_i18n::tr(SORT[self.sort]).into()),
            (tr!("Status").into(), status.get(self.status).cloned().unwrap_or_default()),
            (tr!("Category").into(), category.into()),
            (tr!("Search").into(), if search.is_empty() { "\u{2014}".into() } else { search.to_string().into() }),
        ]
    }
}

impl CollectionView {
    /// The keys the Grid of a tab shows now (also for the layouts that reuse its filters).
    pub(super) fn grid_keys(&mut self, tab: usize, spec: &TabSpec) -> Vec<SharedString> {
        let Some(lib) = self.lib.clone() else { return Vec::new() };
        let games = self.console_games(&spec.source);
        let search = self.console.search.clone();
        let cats = std::mem::take(&mut self.categories);
        let keys = self.tab_nav(tab).grid.keys(&games, &lib, &cats, &search);
        self.categories = cats;
        keys
    }

    fn grid_cols(&self, window: &Window) -> usize {
        let (tw, _) = self.console_tile_size();
        let w = f32::from(window.viewport_size().width) / kit::scale() - 2. * SIDE;
        (((w + GAP) / (tw + GAP)).floor() as usize).max(2)
    }

    pub(super) fn render_grid(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(lib) = self.lib.clone() else {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        };
        let cols = self.grid_cols(window);
        let keys = self.grid_keys(tab, spec);
        let (tw, th) = self.console_tile_size();
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
        let line_h = th + GAP + label_room();
        if nav.reveal && !keys.is_empty() {
            nav.reveal = false;
            // The focused row in sight with a part of the next one.
            nav.scroll.go(Goal::Reveal { ix: nav.index / cols, margin: th * 0.35 });
        }
        let (index, toolbar) = (nav.index, nav.toolbar);
        let glide = nav.scroll.clone();
        let scroll = glide.handle().clone();
        let focused = keys.get(index).cloned();
        self.console_focus_game(focused.clone(), cx);

        let bar =
            div().flex().items_center().gap(u(12.)).px(u(SIDE)).pb(u(16.)).children(chips.iter().enumerate().map(|(i, (label, value))| {
                let on = toolbar == Some(i);
                kit::row_base(on)
                    .min_h(u(46.))
                    .px(u(18.))
                    .when(!on, |d| d.bg(theme::white_a(0.07)))
                    .child(
                        div()
                            .text_size(u(14.))
                            .text_color(if on { theme::console_on_focus() } else { theme::text_muted() })
                            .child(label.clone()),
                    )
                    .child(div().text_size(u(16.)).font_weight(theme::W_BOLD).max_w(u(260.)).truncate().child(value.clone()))
            }));
        let mut grid =
            div().id(("console-grid", tab)).flex().flex_col().overflow_y_scroll().track_scroll(&scroll).px(u(SIDE - PAD)).pb(u(24.));
        // On screen: the rows the grid is scrolled to, and the focused one.
        // The others are empty boxes of the same height (cheap to lay out
        // on every frame of a glide).
        let row_step = f32::from(u(line_h));
        let first_row = (-f32::from(scroll.offset().y) / row_step).floor().max(0.) as usize;
        let rows_shown = (f32::from(window.viewport_size().height) / row_step).ceil() as usize + 1;
        for (r, chunk) in keys.chunks(cols).enumerate() {
            let near = first_row.saturating_sub(1) <= r && r <= first_row + rows_shown;
            let visible = (first_row..=first_row + rows_shown).contains(&r) || r.abs_diff(index / cols) <= 1;
            let mut line = div().flex_none().flex().gap(u(GAP)).h(u(line_h)).px(u(PAD)).pt(u(PAD));
            if near || r.abs_diff(index / cols) <= 1 {
                for (c, key) in chunk.iter().enumerate() {
                    line = line.child(self.console_tile(key, tw, th, grid_focus && r * cols + c == index, visible, window));
                }
            }
            grid = grid.child(line);
        }
        grid = grid.child(glide.driver());
        let title_line = focused.as_ref().and_then(|k| lib.get(k)).map(|g| {
            div()
                .flex()
                .items_baseline()
                .gap(u(14.))
                .px(u(SIDE))
                .pb(u(6.))
                .child(motion::enter_from(
                    SharedString::from(format!("lib-title-{}", g.key)),
                    kit::title(g.title.clone(), 28.).relative().truncate(),
                    240,
                    10.,
                    0.,
                    0.3,
                ))
                .child(div().flex_none().text_size(u(15.)).text_color(theme::text_muted()).child(tr!(
                    "{index} of {total}",
                    index = index + 1,
                    total = keys.len()
                )))
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .children(title_line)
            .child(if keys.is_empty() {
                kit::empty(Icon::FaSearch, tr!("No games match"), tr!("Change the filters above or the search (Y).")).into_any_element()
            } else {
                grid.flex_1().min_h_0().into_any_element()
            })
            .into_any_element()
    }

    pub(super) fn grid_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(lib) = self.lib.clone() else { return false };
        let keys = self.grid_keys(tab, spec);
        let status_n = lib.statuses.len() + 1;
        let category_n = category_options(&self.categories).len();
        let wrap = look::theme().nav.wrap;
        let nav = &mut self.tab_nav(tab).grid;
        let cols = nav.cols.max(1);
        if let Some(chip) = nav.toolbar {
            match pad {
                Pad::Left => nav.toolbar = Some(chip.saturating_sub(1)),
                Pad::Right => nav.toolbar = Some((chip + 1).min(CHIPS - 1)),
                Pad::Down if !keys.is_empty() => nav.toolbar = None,
                Pad::A => match chip {
                    0 => nav.show = (nav.show + 1) % SHOW.len(),
                    1 => nav.sort = (nav.sort + 1) % SORT.len(),
                    2 => nav.status = (nav.status + 1) % status_n,
                    3 => nav.category = (nav.category + 1) % category_n,
                    _ => {
                        self.console_search(window, cx);
                        return true;
                    }
                },
                Pad::X => match chip {
                    0 => nav.show = 0,
                    1 => nav.sort = 0,
                    2 => nav.status = 0,
                    3 => nav.category = 0,
                    _ => self.console.search.clear(),
                },
                Pad::Y => {
                    self.console_search(window, cx);
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
            Pad::Left => nav.index = super::step(i, n, -1, wrap),
            Pad::Right => nav.index = super::step(i, n, 1, wrap),
            Pad::Up if i < cols => nav.toolbar = Some(0),
            Pad::Up => nav.index = i - cols,
            Pad::Down => nav.index = (i + cols).min(n.saturating_sub(1)),
            Pad::ScrollDown | Pad::RT => nav.index = (i + cols * 3).min(n.saturating_sub(1)),
            Pad::ScrollUp | Pad::LT => nav.index = i.saturating_sub(cols * 3),
            Pad::A | Pad::X => {
                if let Some(k) = keys.get(i) {
                    self.console_activate(k.clone(), pad, window, cx);
                }
                return true;
            }
            Pad::Y => {
                self.console_search(window, cx);
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
                crate::console::hint(crate::console::Btn::A, tr!("Change")),
                crate::console::hint(crate::console::Btn::X, tr!("Reset")),
                crate::console::hint(crate::console::Btn::Y, tr!("Search")),
            ];
            h.extend(self.console_tab_hints());
            return h;
        }
        self.console_cover_hints()
    }
}
