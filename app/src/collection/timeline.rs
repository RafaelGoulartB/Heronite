//! "Timeline" of the focus panel: each day the game was played (time played,
//! number of sessions) with the screenshots taken during those sessions, as
//! thumbnails (the same cache as the Screenshots strip). Newest day first;
//! days with screenshots but no tracked session are listed too.

use chrono::Datelike;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::timeline::TimelineDay;

use super::focus::{h3, panel};
use super::game_facts::TIMELINE_DAYS;
use super::screenshots::thumb_tile;
use super::CollectionView;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};

const DATE_W: f32 = 86.;
const RAIL_W: f32 = 14.;
const GAP: f32 = 14.;
const TILE_W: f32 = 144.;
const TILE_H: f32 = 81.;
const TILE_GAP: f32 = 8.;
const ROW_GAP: f32 = 18.;

impl CollectionView {
    /// The section (none when the game was never played and has no screenshots).
    pub(super) fn timeline_section(&mut self, key: &SharedString, main_w: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let days = self.timeline_days(key)?;
        if days.is_empty() {
            return None;
        }
        let content_w = main_w - 2. * 16.8 - DATE_W - RAIL_W - 2. * GAP;
        let max_tiles = (((content_w + TILE_GAP) / (TILE_W + TILE_GAP)).floor() as usize).clamp(1, 6);
        let shown = self.facts.timeline_days.min(days.len());
        let best = days.iter().map(|d| d.played_secs).fold(0., f64::max);
        let played_days = days.iter().filter(|d| d.played_secs > 0.).count();
        let this_year = chrono::Local::now().year();

        let mut list = div().flex().flex_col();
        for (i, day) in days[..shown].iter().enumerate() {
            let is_best = played_days > 1 && day.played_secs > 0. && day.played_secs >= best;
            let tiles = self.day_tiles(i, day, max_tiles, cx);
            list = list.child(day_row(day, this_year, i + 1 < shown, is_best, tiles));
        }

        let sessions: usize = days.iter().map(|d| d.sessions).sum();
        let since = days.iter().rev().find(|d| d.sessions > 0).map(|d| date_label(d, this_year));
        let meta = match (sessions, since) {
            (0, _) | (_, None) => String::new(),
            (1, Some(s)) => format!("1 session · {s}"),
            (n, Some(s)) => format!("{n} sessions since {s}"),
        };

        let more = days.len() - shown;
        let footer = if more > 0 {
            Some(button_sm("timeline-more", ButtonKind::Ghost, format!("Show {} more", more.min(TIMELINE_DAYS * 2))).on_click(cx.listener(
                |this, _, _, cx| {
                    this.facts.timeline_days += TIMELINE_DAYS * 2;
                    cx.notify();
                },
            )))
        } else if shown > TIMELINE_DAYS {
            Some(button_sm("timeline-less", ButtonKind::Ghost, "Show less").on_click(cx.listener(|this, _, _, cx| {
                this.facts.timeline_days = TIMELINE_DAYS;
                cx.notify();
            })))
        } else {
            None
        };

        Some(
            div()
                .flex()
                .flex_col()
                .child(
                    h3("Timeline").flex().justify_between().items_baseline().gap(px(12.)).child(
                        div()
                            .font_family(theme::FONT_BODY)
                            .text_size(px(12.5))
                            .font_weight(theme::W_REGULAR)
                            .text_color(theme::white_a(0.52))
                            .child(meta),
                    ),
                )
                .child(
                    panel(false)
                        .gap(px(4.))
                        .child(list)
                        .children(footer.map(|b| div().flex().pl(px(DATE_W + RAIL_W + 2. * GAP - 10.)).child(b))),
                )
                .into_any_element(),
        )
    }

    /// Thumbnails of one day; the last tile shows "+N" when they do not fit.
    fn day_tiles(&mut self, day_i: usize, day: &TimelineDay, max: usize, cx: &mut Context<Self>) -> Option<Div> {
        if day.shots.is_empty() {
            return None;
        }
        let n = day.shots.len();
        let visible = if n > max { max - 1 } else { n };
        let mut row = div().flex().gap(px(TILE_GAP)).mt(px(8.));
        for (j, &idx) in day.shots.iter().take(visible).enumerate() {
            let Some(path) = self.shots.items.get(idx).map(|s| s.path.clone()) else { continue };
            let src = self.shots.thumb(&path);
            row = row.child(
                thumb_tile(SharedString::from(format!("tl-{day_i}-{j}")), src, true)
                    .flex_none()
                    .w(px(TILE_W))
                    .h(px(TILE_H))
                    .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(idx, window, cx))),
            );
        }
        if n > visible {
            let idx = day.shots[visible];
            if let Some(path) = self.shots.items.get(idx).map(|s| s.path.clone()) {
                let src = self.shots.thumb(&path);
                row = row.child(
                    thumb_tile(SharedString::from(format!("tl-{day_i}-more")), src, false)
                        .flex_none()
                        .w(px(TILE_W))
                        .h(px(TILE_H))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(theme::rgb_a(8, 10, 14, 0.62))
                                .font_weight(theme::W_BOLD)
                                .text_size(px(16.))
                                .child(format!("+{}", n - visible)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(idx, window, cx))),
                );
            }
        }
        Some(row)
    }
}

fn day_row(day: &TimelineDay, this_year: i32, has_next: bool, is_best: bool, tiles: Option<Div>) -> Div {
    let sub = match (day.sessions, day.first_start) {
        (1, Some(t)) => t.format("%-I:%M %p").to_string(),
        (0, _) => String::new(),
        (n, _) => format!("{n} sessions"),
    };
    let shots = match day.shots.len() {
        0 => None,
        1 => Some("1 screenshot".to_string()),
        n => Some(format!("{n} screenshots")),
    };
    let played = (day.played_secs > 0.).then(|| format_playtime_minutes((day.played_secs / 60.).round().max(1.) as u64));
    let mut line = div().flex().items_center().flex_wrap().gap(px(6.)).text_size(px(13.6)).line_height(px(20.));
    if let Some(p) = played {
        line = line.child(div().text_color(theme::white_a(0.72)).child("Played")).child(div().font_weight(theme::W_BOLD).child(p));
    }
    if let Some(s) = shots {
        if day.played_secs > 0. {
            line = line.child(div().text_color(theme::white_a(0.4)).child("·"));
        }
        line = line.child(div().text_color(theme::white_a(0.72)).child(s));
    }
    if is_best {
        line = line.child(
            div()
                .ml(px(4.))
                .px(px(7.))
                .rounded(px(5.))
                .bg(theme::accent_tint())
                .text_size(px(11.5))
                .text_color(theme::accent())
                .child("Longest day"),
        );
    }
    div()
        .flex()
        .gap(px(GAP))
        .pb(px(ROW_GAP))
        .child(
            div()
                .flex_none()
                .w(px(DATE_W))
                .flex()
                .flex_col()
                .items_end()
                .child(
                    div()
                        .font_family(theme::FONT_HEADING)
                        .font_weight(theme::W_BOLD)
                        .text_size(px(14.4))
                        .line_height(px(20.))
                        .child(date_label(day, this_year)),
                )
                .child(div().text_size(px(12.)).text_color(theme::white_a(0.52)).child(sub)),
        )
        .child(
            div()
                .relative()
                .flex_none()
                .w(px(RAIL_W))
                .child(div().absolute().top(px(5.)).left(px(2.)).size(px(10.)).rounded_full().bg(if day.sessions > 0 {
                    theme::accent().into()
                } else {
                    theme::white_a(0.4)
                }))
                .when(has_next, |d| {
                    d.child(div().absolute().top(px(20.)).bottom(px(-ROW_GAP)).left(px(6.)).w(px(2.)).bg(theme::white_a(0.1)))
                }),
        )
        .child(div().flex_1().min_w_0().child(line).children(tiles))
}

/// "Oct 7", or "Oct 7, 2025" outside the current year.
fn date_label(day: &TimelineDay, this_year: i32) -> String {
    if day.date.year() == this_year {
        day.date.format("%b %-d").to_string()
    } else {
        day.date.format("%b %-d, %Y").to_string()
    }
}
