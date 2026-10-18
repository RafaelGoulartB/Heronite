//! The game's recent captures: a strip on Home, a grid on the Screenshots
//! tab (↓ from "Take screenshot" enters it) and a full-size viewer.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::OverlayPanel;
use crate::console::kit::{self, u};
use crate::console::Pad;
use crate::theme;

/// Captures on Home.
const RECENT: usize = 3;
const COLS: usize = 2;

impl OverlayPanel {
    fn tile(&self, i: usize, focused: bool) -> Div {
        let shot = &self.data.shots[i];
        let mut tile = div()
            .relative()
            .w_full()
            .h(u(if focused { 112. } else { 108. }))
            .rounded(u(10.))
            .overflow_hidden()
            .bg(theme::white_a(0.06))
            .border_2()
            .border_color(if focused { theme::console_focus().into() } else { theme::white_a(0.06) });
        if let Some(thumb) = self.thumb(&shot.path) {
            tile = tile.child(img(thumb).size_full().object_fit(ObjectFit::Cover));
        }
        if let Some(when) = shot.taken_at.and_then(taken_label) {
            tile = tile.child(
                div()
                    .absolute()
                    .left(u(8.))
                    .bottom(u(6.))
                    .px(u(6.))
                    .rounded(u(6.))
                    .bg(theme::black_a(0.55))
                    .text_size(u(11.))
                    .text_color(theme::text_primary())
                    .child(when),
            );
        }
        tile
    }

    pub(super) fn render_recent(&self) -> Div {
        if self.data.shots.is_empty() {
            return div();
        }
        let n = self.data.shots.len().min(RECENT);
        div().flex().flex_col().gap(u(8.)).pt(u(6.)).child(kit::caption(tr!("Recent screenshots"))).child(
            div().flex().gap(u(8.)).px(u(12.)).children((0..n).map(|i| div().flex_1().min_w_0().child(self.tile(i, false).h(u(70.))))),
        )
    }

    pub(super) fn render_grid(&self) -> Div {
        if self.data.shots.is_empty() {
            return div()
                .px(u(22.))
                .pt(u(8.))
                .text_size(u(14.))
                .text_color(theme::text_muted())
                .child(tr!("No screenshots of this game yet."));
        }
        let mut grid =
            div().id("overlay-grid").flex().flex_col().gap(u(8.)).px(u(12.)).pb(u(12.)).overflow_y_scroll().track_scroll(&self.grid_scroll);
        for row in self.data.shots.chunks(COLS).enumerate().map(|(r, c)| (r * COLS, c.len())) {
            let (start, len) = row;
            grid = grid.child(
                div()
                    .flex()
                    .gap(u(8.))
                    .children((start..start + len).map(|i| div().flex_1().min_w_0().child(self.tile(i, self.grid == Some(i)))))
                    .when(len < COLS, |d| d.child(div().flex_1())),
            );
        }
        // Takes what the panel has left; more rows scroll.
        div().flex_1().min_h_0().child(grid.size_full())
    }

    pub(super) fn grid_pad(&mut self, pad: Pad) {
        let Some(i) = self.grid else { return };
        let n = self.data.shots.len();
        self.grid = match pad {
            Pad::Left if i % COLS > 0 => Some(i - 1),
            Pad::Right if i % COLS + 1 < COLS && i + 1 < n => Some(i + 1),
            Pad::Down if i + COLS < n => Some(i + COLS),
            // Back to "Take screenshot".
            Pad::Up if i < COLS => None,
            Pad::Up => Some(i - COLS),
            Pad::A => {
                self.viewer = Some(i);
                Some(i)
            }
            _ => Some(i),
        };
        if let Some(i) = self.grid {
            self.grid_scroll.scroll_to_item(i / COLS);
        }
    }

    pub(super) fn viewer_pad(&mut self, pad: Pad) {
        let Some(i) = self.viewer else { return };
        let n = self.data.shots.len();
        match pad {
            Pad::Left if i > 0 => self.viewer = Some(i - 1),
            Pad::Right if i + 1 < n => self.viewer = Some(i + 1),
            Pad::A | Pad::B => {
                self.grid = self.grid.map(|_| i);
                self.viewer = None;
            }
            _ => {}
        }
    }

    pub(super) fn render_viewer(&self) -> Option<AnyElement> {
        let i = self.viewer?;
        let shot = self.data.shots.get(i)?;
        let count = tr!("{n} of {total}", n = i + 1, total = self.data.shots.len());
        let when = shot.taken_at.and_then(taken_label).unwrap_or_default();
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .bg(theme::black_a(0.94))
                .child(div().flex_1().min_h_0().p(u(12.)).child(img(shot.path.clone()).size_full().object_fit(ObjectFit::Contain)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(u(24.))
                        .pb(u(8.))
                        .text_size(u(13.))
                        .text_color(theme::text_muted())
                        .child(count)
                        .child(when),
                )
                .child(self.render_hints())
                .into_any_element(),
        )
    }
}

/// "21:04" today, else the day ("12 Oct").
fn taken_label(ms: f64) -> Option<String> {
    use chrono::{Local, TimeZone};
    let t = Local.timestamp_millis_opt(ms as i64).single()?;
    if t.date_naive() == Local::now().date_naive() {
        Some(t.format("%H:%M").to_string())
    } else {
        // The day in the language's order ("10/18" / "18/10").
        Some(t.format(tr!("%m/%d")).to_string())
    }
}
