//! "Timeline" of the focus panel: each day the game was played (time played,
//! number of sessions) with the screenshots taken during those sessions, as
//! thumbnails (the same cache as the Screenshots strip). Newest day first;
//! days with screenshots but no tracked session are listed too. A switch in
//! its title turns it into the game's Journal (`journal/`); each day here
//! shows how many notes it has.

use std::collections::HashMap;

use chrono::Datelike;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::timeline::TimelineDay;

use super::focus::{h3, panel, small_tabs};
use super::game_facts::TIMELINE_DAYS;
use super::screenshots::thumb_tile;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::icon;
use heronite_i18n::{tr, trc, trn};

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
        let current = self.journal.is_for(key);
        let has_notes = current && self.journal.data.as_ref().is_some_and(|d| !d.entries.is_empty() || d.left_off.is_some());
        if current && (self.ui.journal_view || (days.is_empty() && has_notes)) {
            return self.journal_section(key, main_w, cx);
        }
        if days.is_empty() {
            return None;
        }
        let notes: HashMap<chrono::NaiveDate, usize> = match self.journal_days(key) {
            Some((_, jdays)) if current => jdays.iter().filter(|d| !d.entries.is_empty()).map(|d| (d.date, d.entries.len())).collect(),
            _ => HashMap::new(),
        };
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
            let chip = notes.get(&day.date).map(|n| self.notes_chip(i, *n, cx));
            list = list.child(day_row(day, this_year, i + 1 < shown, is_best, chip, tiles));
        }

        let more = days.len() - shown;
        let footer = if more > 0 {
            Some(button_sm("timeline-more", ButtonKind::Ghost, tr!("Show {n} more", n = more.min(TIMELINE_DAYS * 2))).on_click(
                cx.listener(|this, _, _, cx| {
                    this.facts.timeline_days += TIMELINE_DAYS * 2;
                    cx.notify();
                }),
            ))
        } else if shown > TIMELINE_DAYS {
            Some(button_sm("timeline-less", ButtonKind::Ghost, tr!("Show less")).on_click(cx.listener(|this, _, _, cx| {
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
                .child(self.timeline_header(key, cx))
                .child(
                    panel(false)
                        .gap(px(4.))
                        .child(list)
                        .children(footer.map(|b| div().flex().pl(px(DATE_W + RAIL_W + 2. * GAP - 10.)).child(b))),
                )
                .into_any_element(),
        )
    }

    /// The section's title: "Timeline" or "Journal", what it holds, and the
    /// switch between the two.
    pub(super) fn timeline_header(&mut self, key: &SharedString, cx: &mut Context<Self>) -> Div {
        let journal = self.ui.journal_view && self.journal.is_for(key);
        let this_year = chrono::Local::now().year();
        let meta = if journal {
            let n = self.journal.data.as_ref().map_or(0, |d| d.entries.len());
            if n == 0 {
                String::new()
            } else {
                trn!(n, "{n} note", "{n} notes")
            }
        } else {
            let days = self.timeline_days(key).unwrap_or_default();
            let sessions: usize = days.iter().map(|d| d.sessions).sum();
            let since = days.iter().rev().find(|d| d.sessions > 0).map(|d| date_label(d, this_year));
            match (sessions, since) {
                (0, _) | (_, None) => String::new(),
                (1, Some(s)) => tr!("1 session · {since}", since = s),
                (n, Some(s)) => tr!("{n} sessions since {since}", n, since = s),
            }
        };
        let switch = self.view_switch(journal, cx);
        h3(if journal { tr!("Journal") } else { tr!("Timeline") }).flex().justify_between().items_center().gap(px(12.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .font_family(theme::FONT_BODY)
                .text_size(px(12.5))
                .font_weight(theme::W_REGULAR)
                .text_color(theme::white_a(0.52))
                .child(meta)
                .child(switch),
        )
    }

    /// Timeline | Journal.
    fn view_switch(&mut self, journal: bool, cx: &mut Context<Self>) -> Div {
        let weak = cx.entity().downgrade();
        let pick: std::rc::Rc<dyn Fn(usize, &mut Window, &mut App)> = std::rc::Rc::new(move |i, _, cx| {
            weak.update(cx, |this, cx| this.set_journal_view(i == 1, cx)).ok();
        });
        small_tabs("timeline-view", &[tr!("Timeline").into(), tr!("Journal").into()], journal as usize, pick)
    }

    /// "3 notes" on a day of the Timeline; opens the Journal.
    fn notes_chip(&mut self, day_i: usize, n: usize, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(SharedString::from(format!("tl-notes-{day_i}")))
            .flex()
            .items_center()
            .gap(px(5.))
            .ml(px(4.))
            .px(px(7.))
            .rounded(px(5.))
            .cursor_pointer()
            .bg(theme::white_a(0.07))
            .hover(|s| s.bg(theme::white_a(0.13)))
            .text_size(px(11.5))
            .text_color(theme::white_a(0.8))
            .child(icon(Icon::Notes, px(12.), theme::accent()))
            .child(trn!(n, "{n} note", "{n} notes"))
            .on_click(cx.listener(|this, _, _, cx| this.set_journal_view(true, cx)))
            .into_any_element()
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

fn day_row(day: &TimelineDay, this_year: i32, has_next: bool, is_best: bool, notes: Option<AnyElement>, tiles: Option<Div>) -> Div {
    let sub = match (day.sessions, day.first_start) {
        (1, Some(t)) => t.format(trc!("time format", "%-I:%M %p")).to_string(),
        (0, _) => String::new(),
        (n, _) => tr!("{n} sessions", n),
    };
    let shots = match day.shots.len() {
        0 => None,
        n => Some(trn!(n, "{n} screenshot", "{n} screenshots")),
    };
    let played = (day.played_secs > 0.).then(|| format_playtime_minutes((day.played_secs / 60.).round().max(1.) as u64));
    let mut line = div().flex().items_center().flex_wrap().gap(px(6.)).text_size(px(13.6)).line_height(px(20.));
    if let Some(p) = played {
        line = line.child(div().text_color(theme::white_a(0.72)).child(tr!("Played"))).child(div().font_weight(theme::W_BOLD).child(p));
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
                .child(tr!("Longest day")),
        );
    }
    line = line.children(notes);
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
    let (month, d) = (heronite_i18n::month_short(day.date.month()), day.date.day());
    if day.date.year() == this_year {
        tr!("{month} {d}", month, d)
    } else {
        tr!("{month} {d}, {year}", month, d, year = day.date.year())
    }
}
