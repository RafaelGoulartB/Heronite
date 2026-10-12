//! The console library: **Home** (the focused game large over its art, with
//! shelves below — Continue playing, Favourites, Installed and one per
//! status) and **Library** (every game in a cover grid, with Show / Sort /
//! Status / Search filters). LB / RB switch between them.
//!
//! Its filters are console-only state: they never change the desktop
//! Collection's filters.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{CollectionGame, Library};

use super::super::CollectionView;
use super::Screen;
use crate::console::kit::{self, u};
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::osk::{Osk, OskEvent};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;
use heronite_i18n::{n_, tr, trc};

const TILE_W: f32 = 150.;
const GAP: f32 = 18.;
const SIDE: f32 = 46.;
/// How much the focused cover grows (each side, share of its size).
const LIFT: f32 = 0.045;
/// Room around a row of covers for the lifted one and its ring.
const PAD: f32 = 10.;

pub(super) struct HomeNav {
    pub shelf: usize,
    /// Focused column of each shelf (kept when moving between shelves).
    cols: Vec<usize>,
    shelves: Glide,
    rows: Vec<Glide>,
}

impl Default for HomeNav {
    fn default() -> Self {
        Self { shelf: 0, cols: Vec::new(), shelves: Glide::new(Axis::Y), rows: Vec::new() }
    }
}

impl HomeNav {
    fn fit(&mut self, n: usize) {
        self.cols.resize(n, 0);
        while self.rows.len() < n {
            self.rows.push(Glide::new(Axis::X));
        }
    }
}

pub(super) struct LibraryNav {
    /// Focused filter chip (`None` = the grid has the focus).
    toolbar: Option<usize>,
    index: usize,
    show: usize,
    sort: usize,
    status: usize,
    search: String,
    scroll: Glide,
    reveal: bool,
    /// The filters changed: back to the top at once.
    restart: bool,
    cols: usize,
}

impl Default for LibraryNav {
    fn default() -> Self {
        Self {
            toolbar: None,
            index: 0,
            show: 0,
            sort: 0,
            status: 0,
            search: String::new(),
            scroll: Glide::new(Axis::Y),
            reveal: false,
            restart: false,
            cols: 6,
        }
    }
}

const SHOW: [&str; 4] = [n_!("All games"), n_!("Installed"), n_!("Favourites"), n_!("Not installed")];
const SORT: [&str; 3] = [n_!("A\u{2013}Z"), n_!("Last played"), n_!("Most played")];

pub(super) struct Shelf {
    pub title: SharedString,
    pub keys: Vec<SharedString>,
}

fn by_last_played<'a>(games: impl Iterator<Item = &'a std::sync::Arc<CollectionGame>>) -> Vec<&'a CollectionGame> {
    let mut v: Vec<&CollectionGame> = games.map(|g| g.as_ref()).collect();
    v.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower)));
    v
}

fn keys(v: Vec<&CollectionGame>) -> Vec<SharedString> {
    v.into_iter().map(|g| SharedString::from(g.key.clone())).collect()
}

/// Home shelves, empty ones left out.
pub(super) fn shelves(lib: &Library) -> Vec<Shelf> {
    let mut out = Vec::new();
    let recent: Vec<&CollectionGame> = by_last_played(lib.games.iter().filter(|g| g.last_played.is_some())).into_iter().take(20).collect();
    out.push(Shelf { title: tr!("Continue playing").into(), keys: keys(recent) });
    out.push(Shelf { title: tr!("Favourites").into(), keys: keys(by_last_played(lib.games.iter().filter(|g| g.favourite))) });
    out.push(Shelf { title: trc!("shelf", "Installed").into(), keys: keys(by_last_played(lib.games.iter().filter(|g| g.installed))) });
    for st in &lib.statuses {
        let games = by_last_played(lib.games.iter().filter(|g| g.status_id == st.id));
        out.push(Shelf { title: heronite_data::status::status_label(&st.name).to_string().into(), keys: keys(games) });
    }
    out.retain(|s| !s.keys.is_empty());
    out
}

impl LibraryNav {
    fn status_options(lib: &Library) -> Vec<SharedString> {
        std::iter::once(SharedString::from(tr!("Any status")))
            .chain(lib.statuses.iter().map(|s| SharedString::from(heronite_data::status::status_label(&s.name).to_string())))
            .collect()
    }

    /// The games the grid shows, in order.
    pub(super) fn keys(&self, lib: &Library) -> Vec<SharedString> {
        let query = self.search.trim().to_lowercase();
        let status = self.status.checked_sub(1).and_then(|i| lib.statuses.get(i)).map(|s| s.id.clone());
        let mut games: Vec<&CollectionGame> = lib
            .games
            .iter()
            .map(|g| g.as_ref())
            .filter(|g| match self.show {
                1 => g.installed,
                2 => g.favourite,
                3 => !g.installed,
                _ => true,
            })
            .filter(|g| status.as_ref().is_none_or(|s| &g.status_id == s))
            .filter(|g| query.is_empty() || g.title_lower.contains(&query))
            .collect();
        match self.sort {
            1 => games.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower))),
            2 => games.sort_by(|a, b| b.playtime_minutes.cmp(&a.playtime_minutes).then_with(|| a.title_lower.cmp(&b.title_lower))),
            _ => games.sort_by(|a, b| a.title_lower.cmp(&b.title_lower)),
        }
        keys(games)
    }

    fn chips(&self, lib: &Library) -> [(SharedString, SharedString); 4] {
        let status = Self::status_options(lib);
        [
            (tr!("Show").into(), tr(SHOW[self.show]).into()),
            (tr!("Sort").into(), tr(SORT[self.sort]).into()),
            (tr!("Status").into(), status.get(self.status).cloned().unwrap_or_default()),
            (tr!("Search").into(), if self.search.is_empty() { "\u{2014}".into() } else { self.search.clone().into() }),
        ]
    }
}

/// Cover tile. The focused one lifts: it grows a little over its slot
/// (the layout does not move), gets the white ring and a deeper shadow,
/// all on a spring so the focus glides from cover to cover.
fn tile(id: SharedString, src: Option<ImageSource>, title: &str, w: Pixels, h: Pixels, focused: bool, playing: bool) -> Div {
    let title = title.to_string();
    let face = move |t: f32| {
        let (gx, gy) = (w * (LIFT * t), h * (LIFT * t));
        let ring: Hsla = Hsla { a: t.clamp(0., 1.), ..white() };
        div()
            .absolute()
            .top(-gy)
            .bottom(-gy)
            .left(-gx)
            .right(-gx)
            .rounded(u(10.))
            .bg(theme::white_a(0.07))
            .border_3()
            .border_color(ring)
            .when(t > 0.02, |d| d.shadow(kit::shadow(0.6 * t.min(1.), 4. + 8. * t, 10. + 20. * t)))
            .child(match src {
                Some(s) => img(s).size_full().object_fit(ObjectFit::Cover).rounded(u(7.)).into_any_element(),
                // No cover: the name, wrapped inside the tile.
                None => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .p(u(12.))
                    .overflow_hidden()
                    .rounded(u(7.))
                    .bg(linear_gradient(180., linear_color_stop(theme::white_a(0.02), 0.), linear_color_stop(theme::white_a(0.09), 1.)))
                    .child(
                        div()
                            .w_full()
                            .text_size(u(15.))
                            .line_height(u(19.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .line_clamp(4)
                            .text_ellipsis()
                            .child(title),
                    )
                    .into_any_element(),
            })
            .when(playing, |d| d.child(div().absolute().top(u(8.)).left(u(8.)).child(kit::badge(tr!("Playing"), theme::console_btn_a()))))
    };
    div()
        .flex_none()
        .relative()
        .w(w)
        .h(h)
        .child(motion::spring(id, div().absolute().inset_0(), motion::LIFT, focused, move |d, t| d.child(face(t))))
}

/// A Home / Library body sliding in from the side of its tab (`dir` -1 =
/// from the left).
pub(super) fn screen_in(id: &'static str, dir: f32, body: Div) -> AnyElement {
    motion::enter(id, body.relative(), 280, 36. * dir, 0.)
}

/// The overview shortened to about `max` characters at a word, with "…".
fn clip(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rfind(' ').map_or(cut.as_str(), |i| &cut[..i]);
    format!("{}\u{2026}", cut.trim_end_matches([',', ';', ':', '.', ' ']))
}

impl CollectionView {
    /// A cover tile; `load`: it is on screen, so its image is requested.
    /// Off screen it is an empty box of the same size (nothing to look up
    /// or animate on the frames of a glide).
    fn tile_for(&mut self, key: &SharedString, focused: bool, load: bool, window: &Window) -> Div {
        let (w, h) = (TILE_W, TILE_W * self.cover_aspect);
        if !load && !focused {
            return div().flex_none().w(u(w)).h(u(h));
        }
        let s = kit::scale();
        let src = self.cover(key, w * s, h * s, window.scale_factor(), if focused { 200 } else { 50 });
        let game = self.lib.as_ref().and_then(|l| l.get(key)).cloned();
        let title = game.as_ref().map_or(String::new(), |g| g.title.clone());
        let playing = game.as_ref().is_some_and(|g| self.playing.contains_key(&g.app_name));
        tile(SharedString::from(format!("tile-{key}")), src, &title, u(w), u(h), focused, playing)
    }

    /// Big title + facts of `key` (the Home hero).
    fn hero(&self, key: Option<&SharedString>) -> Div {
        let Some(g) = key.and_then(|k| self.lib.as_ref()?.get(k)) else { return div() };
        let mut facts = vec![g.source_label.to_string()];
        facts.push(if g.installed { tr!("Installed").into() } else { tr!("Not installed").into() });
        if g.playtime_minutes > 0 {
            facts.push(tr!("{time} played", time = heronite_data::collection::format_playtime_minutes(g.playtime_minutes)));
        }
        let status = self
            .lib
            .as_ref()
            .and_then(|l| l.statuses.iter().find(|s| s.id == g.status_id))
            .map(|s| (heronite_data::status::status_label(&s.name).to_string(), s.slug.clone()));
        let overview = self.focus.as_ref().filter(|f| f.key.as_ref() == g.key).and_then(|f| f.overview.clone());
        let hero = div()
            .relative()
            .flex()
            .flex_col()
            .gap(u(10.))
            .max_w(u(760.))
            .child(kit::title(g.title.clone(), 46.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(12.))
                    .text_size(u(16.))
                    .text_color(theme::text_muted())
                    .child(facts.join(" \u{b7} "))
                    .children(status.map(|(name, slug)| kit::badge(name, theme::status_color(&slug)))),
            )
            .children(overview.map(|o| {
                motion::fade_in(SharedString::from(format!("hero-about-{}", g.key)), kit::body_text(clip(&o, 300)).line_clamp(3), 260)
            }));
        // A new game slides in a little (never from blank: holding a
        // direction keeps the text readable).
        div().child(motion::enter_from(SharedString::from(format!("hero-{}", g.key)), hero, 300, 14., 0., 0.25))
    }

    pub(super) fn render_console_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(lib) = self.lib.clone() else {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        };
        let shelves = shelves(&lib);
        if shelves.is_empty() {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(self.console_top_bar(Some((&[tr!("Home").into(), tr!("Library").into()], 0))))
                .child(kit::empty(Icon::GridView, tr!("No games yet"), tr!("Add a game from the menu (\u{2630}).")))
                .into_any_element();
        }
        let nav = &mut self.console.home;
        nav.shelf = nav.shelf.min(shelves.len() - 1);
        nav.fit(shelves.len());
        for (i, s) in shelves.iter().enumerate() {
            nav.cols[i] = nav.cols[i].min(s.keys.len() - 1);
        }
        let (si, ci) = (nav.shelf, nav.cols[nav.shelf]);
        let focused = shelves[si].keys[ci].clone();
        let rows = nav.rows.clone();
        let shelves_glide = nav.shelves.clone();
        let cols = nav.cols.clone();
        self.console_focus_game(Some(focused.clone()), cx);

        let mut list = div()
            .id("console-shelves")
            .flex()
            .flex_col()
            .gap(u(10.))
            .overflow_y_scroll()
            .track_scroll(shelves_glide.handle())
            .px(u(SIDE - PAD))
            .pb(u(20.));
        let step = f32::from(u(TILE_W + GAP));
        let per_row = (f32::from(window.viewport_size().width) / step).ceil() as usize + 1;
        for (i, shelf) in shelves.iter().enumerate() {
            let mut row = div().id(("console-shelf", i)).flex().gap(u(GAP)).p(u(PAD)).overflow_x_scroll().track_scroll(rows[i].handle());
            // On screen: the shelves around the focused one, the columns
            // each is scrolled to (and the focus, while it glides there).
            let near = i + 1 >= si && i <= si + 2;
            let first = (-f32::from(rows[i].handle().offset().x) / step).floor().max(0.) as usize;
            for (c, key) in shelf.keys.iter().enumerate() {
                let visible = near && ((first..=first + per_row).contains(&c) || c.abs_diff(cols[i]) <= 2);
                row = row.child(self.tile_for(key, i == si && c == cols[i], visible, window));
            }
            row = row.child(rows[i].driver());
            let on = i == si;
            let label = format!("{}  \u{b7}  {}", shelf.title, shelf.keys.len());
            // The focused shelf is bright; the others step back.
            list = list.child(motion::spring(("shelf-fx", i), div().flex().flex_col(), motion::SOFT, on, move |d, t| {
                let t = t.clamp(0., 1.);
                d.opacity(0.5 + 0.5 * t)
                    .child(
                        div()
                            .px(u(PAD))
                            .text_size(u(18.))
                            .font_weight(theme::W_BOLD)
                            .text_color(motion::mix(theme::text_muted(), white(), t))
                            .child(label),
                    )
                    .child(row)
            }));
        }
        list = list.child(shelves_glide.driver());
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.console_top_bar(Some((&[tr!("Home").into(), tr!("Library").into()], 0))))
            .child(screen_in(
                "screen-home",
                -1.,
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_none().h(u(250.)).px(u(SIDE)).pt(u(20.)).child(self.hero(Some(&focused))))
                    .child(list.flex_1().min_h_0()),
            ))
            .into_any_element()
    }

    pub(super) fn home_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(lib) = self.lib.clone() else { return false };
        let shelves = shelves(&lib);
        if shelves.is_empty() {
            return false;
        }
        let nav = &mut self.console.home;
        nav.fit(shelves.len());
        let si = nav.shelf.min(shelves.len() - 1);
        let ci = nav.cols[si].min(shelves[si].keys.len() - 1);
        let key = shelves[si].keys[ci].clone();
        match pad {
            Pad::Left => nav.cols[si] = ci.saturating_sub(1),
            Pad::Right => nav.cols[si] = (ci + 1).min(shelves[si].keys.len() - 1),
            Pad::Up => nav.shelf = si.saturating_sub(1),
            Pad::Down => nav.shelf = (si + 1).min(shelves.len() - 1),
            Pad::A => {
                self.open_game_page(key);
                return true;
            }
            Pad::X => {
                self.primary_action(&key, cx);
                return true;
            }
            Pad::Y => {
                self.console.screen = Screen::Library;
                self.open_console_search(window, cx);
                return true;
            }
            _ => return false,
        }
        // The focused shelf to the top, its cover in sight with a peek of
        // the next ones.
        let si = nav.shelf;
        nav.shelves.go(Goal::Start(si));
        let ci = nav.cols[si].min(shelves[si].keys.len() - 1);
        nav.rows[si].go(Goal::Reveal { ix: ci, margin: TILE_W * 0.6 });
        true
    }

    pub(super) fn home_hints(&self) -> Vec<Hint> {
        let primary = self.selected.as_ref().and_then(|k| self.lib.as_ref()?.get(k)).map(|g| self.primary_label(g)).unwrap_or(tr!("Play"));
        vec![
            hint(Btn::A, tr!("Details")),
            hint(Btn::X, primary),
            hint(Btn::Y, tr!("Search")),
            hint(Btn::Bumpers, tr!("Library")),
            hint(Btn::Start, tr!("Menu")),
        ]
    }

    /// "Play", "Install" or "Stop" for the X shortcut.
    pub(super) fn primary_label(&self, g: &CollectionGame) -> &'static str {
        if self.playing.contains_key(&g.app_name) {
            tr!("Stop")
        } else if g.installed {
            tr!("Play")
        } else {
            tr!("Install")
        }
    }

    fn library_cols(&self, window: &Window) -> usize {
        let w = f32::from(window.viewport_size().width) / kit::scale() - 2. * SIDE;
        (((w + GAP) / (TILE_W + GAP)).floor() as usize).max(2)
    }

    pub(super) fn render_console_library(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(lib) = self.lib.clone() else {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        };
        let cols = self.library_cols(window);
        let keys = self.console.library.keys(&lib);
        let tile_h = TILE_W * self.cover_aspect;
        let nav = &mut self.console.library;
        nav.cols = cols;
        nav.index = nav.index.min(keys.len().saturating_sub(1));
        let grid_focus = nav.toolbar.is_none();
        if nav.restart {
            nav.restart = false;
            nav.scroll.reset();
        }
        if nav.reveal && !keys.is_empty() {
            nav.reveal = false;
            // The focused row in sight with a part of the next one.
            nav.scroll.go(Goal::Reveal { ix: nav.index / cols, margin: tile_h * 0.35 });
        }
        let (index, toolbar) = (nav.index, nav.toolbar);
        let chips = nav.chips(&lib);
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
        let mut grid = div().id("console-grid").flex().flex_col().overflow_y_scroll().track_scroll(&scroll).px(u(SIDE - PAD)).pb(u(24.));
        // On screen: the rows the grid is scrolled to, and the focused one.
        // The others are empty boxes of the same height (cheap to lay out
        // on every frame of a glide).
        let line_h = tile_h + GAP;
        let row_step = f32::from(u(line_h));
        let first_row = (-f32::from(scroll.offset().y) / row_step).floor().max(0.) as usize;
        let rows_shown = (f32::from(window.viewport_size().height) / row_step).ceil() as usize + 1;
        for (r, chunk) in keys.chunks(cols).enumerate() {
            let near = first_row.saturating_sub(1) <= r && r <= first_row + rows_shown;
            let visible = (first_row..=first_row + rows_shown).contains(&r) || r.abs_diff(index / cols) <= 1;
            let mut line = div().flex_none().flex().gap(u(GAP)).h(u(line_h)).px(u(PAD)).pt(u(PAD));
            if near || r.abs_diff(index / cols) <= 1 {
                for (c, key) in chunk.iter().enumerate() {
                    line = line.child(self.tile_for(key, grid_focus && r * cols + c == index, visible, window));
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
            .child(self.console_top_bar(Some((&[tr!("Home").into(), tr!("Library").into()], 1))))
            .child(screen_in(
                "screen-library",
                1.,
                div().flex().flex_col().flex_1().min_h_0().child(bar).children(title_line).child(if keys.is_empty() {
                    kit::empty(Icon::FaSearch, tr!("No games match"), tr!("Change the filters above or the search (Y).")).into_any_element()
                } else {
                    grid.flex_1().min_h_0().into_any_element()
                }),
            ))
            .into_any_element()
    }

    pub(super) fn library_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(lib) = self.lib.clone() else { return false };
        let keys = self.console.library.keys(&lib);
        let status_n = lib.statuses.len() + 1;
        let nav = &mut self.console.library;
        let cols = nav.cols.max(1);
        if let Some(chip) = nav.toolbar {
            match pad {
                Pad::Left => nav.toolbar = Some(chip.saturating_sub(1)),
                Pad::Right => nav.toolbar = Some((chip + 1).min(3)),
                Pad::Down if !keys.is_empty() => nav.toolbar = None,
                Pad::A => match chip {
                    0 => nav.show = (nav.show + 1) % SHOW.len(),
                    1 => nav.sort = (nav.sort + 1) % SORT.len(),
                    2 => nav.status = (nav.status + 1) % status_n,
                    _ => {
                        self.open_console_search(window, cx);
                        return true;
                    }
                },
                Pad::X => match chip {
                    0 => nav.show = 0,
                    1 => nav.sort = 0,
                    2 => nav.status = 0,
                    _ => nav.search.clear(),
                },
                Pad::Y => {
                    self.open_console_search(window, cx);
                    return true;
                }
                _ => return false,
            }
            if !matches!(pad, Pad::Left | Pad::Right | Pad::Down) {
                nav.index = 0;
                nav.restart = true;
            }
            return true;
        }
        let n = keys.len();
        let i = nav.index.min(n.saturating_sub(1));
        match pad {
            Pad::Left => nav.index = i.saturating_sub(1),
            Pad::Right => nav.index = (i + 1).min(n.saturating_sub(1)),
            Pad::Up if i < cols => nav.toolbar = Some(0),
            Pad::Up => nav.index = i - cols,
            Pad::Down => nav.index = (i + cols).min(n.saturating_sub(1)),
            Pad::ScrollDown => nav.index = (i + cols * 3).min(n.saturating_sub(1)),
            Pad::ScrollUp => nav.index = i.saturating_sub(cols * 3),
            Pad::A => {
                if let Some(k) = keys.get(i) {
                    self.open_game_page(k.clone());
                }
                return true;
            }
            Pad::X => {
                if let Some(k) = keys.get(i) {
                    self.primary_action(k, cx);
                }
                return true;
            }
            Pad::Y => {
                self.open_console_search(window, cx);
                return true;
            }
            _ => return false,
        }
        nav.reveal = true;
        true
    }

    pub(super) fn library_hints(&self) -> Vec<Hint> {
        if self.console.library.toolbar.is_some() {
            return vec![
                hint(Btn::A, tr!("Change")),
                hint(Btn::X, tr!("Reset")),
                hint(Btn::Y, tr!("Search")),
                hint(Btn::Bumpers, tr!("Home")),
                hint(Btn::Start, tr!("Menu")),
            ];
        }
        let primary = self.selected.as_ref().and_then(|k| self.lib.as_ref()?.get(k)).map(|g| self.primary_label(g)).unwrap_or(tr!("Play"));
        vec![
            hint(Btn::A, tr!("Details")),
            hint(Btn::X, primary),
            hint(Btn::Y, tr!("Search")),
            hint(Btn::Bumpers, tr!("Home")),
            hint(Btn::Start, tr!("Menu")),
        ]
    }

    fn open_console_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let current = self.console.library.search.clone();
        let osk = cx.new(|_| Osk::new(tr!("Search games"), &current));
        let sub = cx.subscribe(&osk, |this, _, ev: &OskEvent, cx| {
            if let OskEvent::Done(text) = ev {
                let lib = &mut this.console.library;
                lib.search = text.trim().to_string();
                lib.index = 0;
                lib.toolbar = None;
                lib.restart = true;
            }
            this.close_console_overlay(cx);
        });
        self.open_console_overlay(&osk, sub, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{LibraryNav, SHOW};

    #[test]
    fn library_filter_labels() {
        let nav = LibraryNav::default();
        assert_eq!(SHOW[nav.show], "All games");
        assert_eq!(nav.search, "");
    }
}
