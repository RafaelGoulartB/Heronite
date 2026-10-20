//! The Album tab: the screenshots taken in game as statistics and memories.
//! A collage of the period, the moments (many captures close together),
//! the games you stop most often to capture, when you capture against when
//! you play, a photo per month and how many captures a session leaves. A
//! chip per game filters the whole tab; any capture opens in the viewer
//! ([`super::viewer`]).

use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{capture_hours, captures_per_session, fmt_secs, moments, Moment, Span, Stats};
use heronite_i18n::{tr, trn};

use super::desktop::{eyebrow, hint, panel, panel_head, GAP};
use super::sections::{row, sentence};
use super::viewer::Photo;
use super::{day_label, StatsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// The album of the period (and of the game picked in the chips).
#[derive(Default)]
pub(super) struct AlbumView {
    /// Captures of the period of the picked game (or all), oldest first.
    pub photos: Vec<Photo>,
    /// Captures per game in the period, most first (unfiltered: the chips).
    pub games: Vec<(usize, usize)>,
    /// Indexes into `photos`.
    pub moments: Vec<Moment>,
    /// Captures per hour played: `(game, captures, seconds played)`.
    pub rate: Vec<(usize, usize, f64)>,
    /// Share of captures and of play time per hour of the day.
    pub hours: ([f64; 24], [f64; 24]),
    /// Sessions by captures: 0, 1, 2–4, 5–9, 10+.
    pub per_session: [usize; 5],
    /// The day with most captures.
    pub busiest: Option<(NaiveDate, usize)>,
    /// Play time behind the captures (the picked game, or the period).
    pub play_secs: f64,
}

const COLLAGE: usize = 6;

impl StatsPage {
    /// Rebuilds [`AlbumView`] for the period and the picked game.
    pub(super) fn recompute_album(&mut self) {
        self.album = match (&self.stats, &self.history, &self.shots) {
            (Some(st), Some(h), Some(all)) if self.tab == super::tabs::Tab::Album => {
                let mut in_period: Vec<Photo> = all
                    .iter()
                    .flat_map(|(g, list)| {
                        list.iter().filter(|s| st.period.contains(s.at.date())).map(move |s| Photo {
                            game: *g,
                            at: s.at,
                            path: s.path.clone(),
                        })
                    })
                    .collect();
                in_period.sort_by_key(|p| p.at);
                let mut games: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
                for p in &in_period {
                    *games.entry(p.game).or_default() += 1;
                }
                let mut games: Vec<(usize, usize)> = games.into_iter().collect();
                games.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                let rate = {
                    let mut v: Vec<(usize, usize, f64)> = games
                        .iter()
                        .filter_map(|(g, n)| Some((*g, *n, st.game(*g)?.secs)))
                        .filter(|(_, n, secs)| *n >= 5 && *secs >= 3600.)
                        .collect();
                    v.sort_by(|a, b| (b.1 as f64 / b.2).total_cmp(&(a.1 as f64 / a.2)));
                    v
                };
                let pick = self.album_game.filter(|g| games.iter().any(|x| x.0 == *g));
                let photos: Vec<Photo> = in_period.into_iter().filter(|p| pick.is_none_or(|g| p.game == g)).collect();
                let pairs: Vec<(usize, NaiveDateTime)> = photos.iter().map(|p| (p.game, p.at)).collect();
                let mut per_day: std::collections::BTreeMap<NaiveDate, usize> = Default::default();
                for p in &photos {
                    *per_day.entry(p.at.date()).or_default() += 1;
                }
                let busiest = per_day.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
                let play_secs = match pick {
                    Some(g) => st.game(g).map_or(0., |x| x.secs),
                    None => st.total_secs,
                };
                Some(Arc::new(AlbumView {
                    moments: moments(&pairs),
                    hours: capture_hours(h, &pairs, st.period),
                    per_session: captures_per_session(h, &pairs, st.period),
                    photos,
                    games,
                    rate,
                    busiest,
                    play_secs,
                }))
            }
            _ => None,
        };
    }

    fn set_album_game(&mut self, game: Option<usize>, cx: &mut Context<Self>) {
        self.album_game = game;
        self.moment_open = None;
        self.recompute_album();
        cx.notify();
    }

    /// The game whose art leads the tab: the most photographed of the period.
    pub(super) fn album_lead(&self) -> Option<usize> {
        let a = self.album.as_ref()?;
        self.album_game.or_else(|| a.games.first().map(|g| g.0))
    }

    /// A capture tile; a click opens `list` in the viewer at `index`.
    pub(super) fn photo_tile(
        &mut self,
        id: SharedString,
        list: Arc<Vec<Photo>>,
        index: usize,
        w: f32,
        h: f32,
        label: Option<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = &list[index];
        let src = self.shot(&p.path, w, h, window, cx);
        div()
            .id(id)
            .relative()
            .flex_none()
            .w(px(w))
            .h(px(h))
            .rounded(px(10.))
            .overflow_hidden()
            .bg(theme::white_a(0.05))
            .cursor_pointer()
            .hover(|s| s.opacity(0.88))
            .on_click(cx.listener(move |this, _, window, cx| this.open_photo(list.to_vec(), index, window, cx)))
            .children(src.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
            .children(label.map(|l| {
                div()
                    .absolute()
                    .left(px(8.))
                    .bottom(px(7.))
                    .max_w(px(w - 16.))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded_full()
                    .bg(rgba(0x0a0e14bf))
                    .text_size(px(10.5))
                    .font_weight(theme::W_BOLD)
                    .truncate()
                    .child(l)
            }))
    }

    /// Hero of the tab: the number of captures and a collage of the period.
    pub(super) fn render_album_total(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Div {
        let period = if st.period.span == Span::All { tr!("all time").to_string() } else { st.period.label() };
        let Some(a) = self.album.clone() else {
            return div()
                .mt_auto()
                .pb(px(30.))
                .child(eyebrow(tr!("Album \u{b7} {period}", period), 0.7))
                .child(div().mt(px(14.)).text_size(px(18.)).child(tr!("Reading your screenshots\u{2026}")));
        };
        let n = a.photos.len();
        let number = div()
            .flex()
            .items_baseline()
            .gap(px(14.))
            .mt(px(8.))
            .mb(px(12.))
            .font_family(theme::font_heading())
            .font_weight(theme::W_BOLD)
            .child(div().text_size(px(96.)).line_height(px(96.)).child(heronite_data::stats::group_thousands(n as u64)))
            .child(div().text_size(px(44.)).text_color(theme::white_a(0.85)).child(if n == 1 {
                tr!("screenshot")
            } else {
                tr!("screenshots")
            }));
        let sep = || div().size(px(4.)).rounded_full().bg(theme::white_a(0.35));
        let mut story = div().flex().flex_wrap().items_center().gap(px(14.)).text_size(px(16.)).text_color(theme::white_a(0.78));
        if self.album_game.is_none() && n > 0 {
            story = story.child(sentence(trn!(a.games.len(), "**{n} game** captured", "**{n} games** captured")));
        }
        if n > 0 && a.play_secs > 0. {
            story = story.child(sep()).child(sentence(tr!("one every **{time}** of play", time = fmt_secs(a.play_secs / n as f64))));
        }
        if let Some((d, c)) = a.busiest {
            story = story.child(sep()).child(sentence(tr!("most on **{day}**: {n}", day = day_label(d), n = c)));
        }
        let main = div().flex_1().min_w_0().child(eyebrow(tr!("Album \u{b7} {period}", period), 0.7)).child(number).child(story);
        // Moments first (their middle capture), at most two per game, then the rest.
        let mut picks: Vec<usize> = Vec::new();
        let mut per_game: std::collections::HashMap<usize, usize> = Default::default();
        let mut take = |i: usize, picks: &mut Vec<usize>| {
            let c = per_game.entry(a.photos[i].game).or_default();
            if *c < 2 && !picks.contains(&i) && picks.len() < COLLAGE {
                *c += 1;
                picks.push(i);
            }
        };
        for m in &a.moments {
            take(m.shots[m.shots.len() / 2], &mut picks);
        }
        for i in (0..n).rev().step_by((n / 24).max(1)) {
            take(i, &mut picks);
        }
        picks.sort_by_key(|i| a.photos[*i].at);
        let list = Arc::new(a.photos.clone());
        let tiles: Vec<Stateful<Div>> = picks
            .iter()
            .enumerate()
            .map(|(k, i)| {
                let label = Some(self.game_title(a.photos[*i].game));
                self.photo_tile(SharedString::from(format!("collage-{k}")), list.clone(), *i, 176., 99., label, window, cx)
            })
            .collect();
        let side = (!tiles.is_empty()).then(|| div().flex_none().grid().grid_cols(3).gap(px(8.)).children(tiles));
        div().mt_auto().pb(px(30.)).flex().items_end().gap(px(28.)).child(main).children(side)
    }

    pub(super) fn render_album(&mut self, st: &Stats, wide: bool, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(a) = self.album.clone() else { return Vec::new() };
        if a.games.is_empty() {
            return vec![panel()
                .items_center()
                .gap(px(8.))
                .py(px(56.))
                .child(icon(Icon::PhotoCamera, px(28.), theme::text_dim()))
                .child(div().text_size(px(15.)).child(tr!("No screenshots in this period")))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::text_dim())
                        .child(tr!("Screenshots taken while playing show up here, matched to their game.")),
                )
                .into_any_element()];
        }
        let mut out = vec![self.album_chips(&a, window, cx), self.album_moments(&a, window, cx)];
        out.push(row(wide, vec![(self.album_rate(&a, st, window, cx), 6), (self.album_hours(&a), 6)]));
        out.push(row(wide, vec![(self.album_months(&a, st, window, cx), 8), (self.album_sessions(&a), 4)]));
        out
    }

    fn album_chips(&mut self, a: &AlbumView, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let total: usize = a.games.iter().map(|g| g.1).sum();
        let chip = |id: SharedString, on: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(34.))
                .pl(px(6.))
                .pr(px(12.))
                .rounded_full()
                .text_size(px(12.5))
                .font_weight(theme::W_BOLD)
                .cursor_pointer()
                .when(on, |d| d.bg(theme::accent()).text_color(theme::on_accent()))
                .when(!on, |d| {
                    d.bg(rgba(0x161b24d6))
                        .border_1()
                        .border_color(theme::white_a(0.08))
                        .text_color(theme::text_primary())
                        .hover(|s| s.bg(theme::white_a(0.1)))
                })
        };
        let mut chips = div().flex().flex_wrap().gap(px(8.)).child(
            chip("album-all".into(), self.album_game.is_none())
                .pl(px(14.))
                .on_click(cx.listener(|this, _, _, cx| this.set_album_game(None, cx)))
                .child(tr!("All games"))
                .child(div().opacity(0.7).child(total.to_string())),
        );
        for (g, n) in a.games.iter().take(10) {
            let game = *g;
            let cover = self.cover(game, 22., 22., window);
            let on = self.album_game == Some(game);
            chips = chips.child(
                chip(SharedString::from(format!("album-game-{game}")), on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_album_game(if this.album_game == Some(game) { None } else { Some(game) }, cx)
                    }))
                    .child(
                        div()
                            .size(px(22.))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(theme::white_a(0.08))
                            .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
                    )
                    .child(div().max_w(px(220.)).truncate().child(self.game_title(game)))
                    .child(div().opacity(0.7).child(n.to_string())),
            );
        }
        chips.into_any_element()
    }

    /// Runs of captures close together, with the one open showing all of them.
    fn album_moments(&mut self, a: &AlbumView, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let head = panel_head(
            tr!("Moments"),
            hint(trn!(a.moments.len(), "{n} moment: 5 or more captures in a row", "{n} moments: 5 or more captures in a row")),
        );
        if a.moments.is_empty() {
            return panel()
                .child(head)
                .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("No run of 5 or more captures in this period.")))
                .into_any_element();
        }
        let list = Arc::new(a.photos.clone());
        let mut grid = div().grid().grid_cols(4).gap(px(GAP));
        for (k, m) in a.moments.iter().take(8).enumerate() {
            let open = self.moment_open == Some(k);
            let mid = m.shots[m.shots.len() / 2];
            let src = self.shot(&list[mid].path, 340., 191., window, cx);
            let title = self.game_title(m.game);
            grid = grid.child(
                div()
                    .id(("moment", k))
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .rounded(px(14.))
                    .overflow_hidden()
                    .bg(rgba(0x161b24d6))
                    .border_1()
                    .border_color(if open { Hsla { a: 0.6, ..Hsla::from(theme::accent()) } } else { theme::white_a(0.08) })
                    .cursor_pointer()
                    .hover(|s| s.border_color(Hsla { a: 0.4, ..Hsla::from(theme::accent()) }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.moment_open = if this.moment_open == Some(k) { None } else { Some(k) };
                        cx.notify();
                    }))
                    .child(
                        div()
                            .relative()
                            .h(px(150.))
                            .bg(theme::white_a(0.04))
                            .children(src.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
                            .child(
                                div()
                                    .absolute()
                                    .right(px(8.))
                                    .top(px(8.))
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded_full()
                                    .bg(rgba(0x0a0e14cc))
                                    .text_size(px(11.5))
                                    .font_weight(theme::W_BOLD)
                                    .child(icon(Icon::PhotoCamera, px(12.), theme::text_primary()))
                                    .child(m.shots.len().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .px(px(14.))
                            .py(px(12.))
                            .child(div().text_size(px(13.5)).font_weight(theme::W_BOLD).truncate().child(title))
                            .child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(tr!(
                                "{day} \u{b7} {n} captures in {m} min",
                                day = day_label(m.start.date()),
                                n = m.shots.len(),
                                m = m.minutes()
                            ))),
                    ),
            );
        }
        let mut p = panel().child(head).child(grid);
        if let Some(m) = self.moment_open.and_then(|k| a.moments.get(k)).cloned() {
            let photos: Arc<Vec<Photo>> = Arc::new(m.shots.iter().map(|i| list[*i].clone()).collect());
            let strip: Vec<Stateful<Div>> = (0..photos.len().min(16))
                .map(|i| self.photo_tile(SharedString::from(format!("moment-shot-{i}")), photos.clone(), i, 188., 106., None, window, cx))
                .collect();
            let game = m.game;
            p = p.child(
                div()
                    .mt(px(16.))
                    .p(px(16.))
                    .rounded(px(14.))
                    .bg(theme::white_a(0.03))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .mb(px(12.))
                            .text_size(px(13.))
                            .child(sentence(tr!(
                                "**{game}** \u{b7} {from} to {to} \u{b7} {n} captures",
                                game = self.game_title(game),
                                from = m.start.format("%H:%M").to_string(),
                                to = m.end.format("%H:%M").to_string(),
                                n = m.shots.len()
                            )))
                            .child(div().flex_1())
                            .child(
                                crate::ui::controls::button_sm(
                                    "moment-sheet",
                                    crate::ui::controls::ButtonKind::Glass,
                                    tr!("Open game sheet"),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(game, window, cx))),
                            ),
                    )
                    .child(div().flex().flex_wrap().gap(px(8.)).children(strip)),
            );
        }
        p.into_any_element()
    }
}
