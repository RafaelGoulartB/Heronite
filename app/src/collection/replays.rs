//! Replays saved in-game, as the Timeline and the Journal show them: a
//! poster (a frame of the video, `screenshots::Shots::thumb`) with the replay
//! yellow ring, a play button and the length; a click plays it. The replays
//! come from the journal: each one is an entry with a `clip`.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::NaiveDate;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::trn;

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// What a poster shows besides the picture.
pub(super) struct ClipTile {
    pub id: SharedString,
    pub path: PathBuf,
    pub w: f32,
    pub h: f32,
    /// The time it was saved, in the top corner.
    pub time: Option<String>,
    /// The note written about it, over the bottom.
    pub caption: Option<String>,
}

impl CollectionView {
    /// The entries with a replay on each day of `key`'s journal (indices into
    /// its entries, oldest first). Empty until the journal is read.
    pub(super) fn day_replays(&mut self, key: &SharedString) -> HashMap<NaiveDate, Vec<usize>> {
        let Some(data) = self.journal.data.clone().filter(|_| self.journal.is_for(key)) else { return HashMap::new() };
        let Some((_, days)) = self.journal_days(key) else { return HashMap::new() };
        days.iter()
            .filter_map(|d| {
                let clips: Vec<usize> = d.entries.iter().copied().filter(|i| data.entries[*i].clip.is_some()).collect();
                (!clips.is_empty()).then_some((d.date, clips))
            })
            .collect()
    }

    /// A replay's poster; a click plays it.
    pub(super) fn clip_tile(&mut self, t: ClipTile, cx: &mut Context<Self>) -> Stateful<Div> {
        let src = self.shots.thumb(&t.path);
        let seconds = self.shots.clip_seconds(&t.path);
        let group: SharedString = format!("{}-g", t.id).into();
        let radius = if t.w >= 160. { 9. } else { 7. };
        let play = if t.h >= 90. { 36. } else { 26. };
        let path = t.path.clone();
        let mut tile = div()
            .id(t.id)
            .group(group.clone())
            .relative()
            .flex_none()
            .w(px(t.w))
            .h(px(t.h))
            .rounded(px(radius))
            .overflow_hidden()
            .bg(theme::rgb_a(14, 16, 20, 1.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.open_replay(path.clone(), cx)));
        tile = match src {
            Some(src) => tile.child(img(src).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover)),
            // Still being made, or no ffmpeg: the camera says what it is.
            None => tile.child(div().absolute().inset_0().flex().items_center().justify_center().child(icon(
                Icon::Videocam,
                px(play * 0.75),
                theme::replay_alpha(0.55),
            ))),
        };
        // A shade under the labels, so they read on a bright frame.
        tile = tile.child(div().absolute().inset_0().bg(linear_gradient(
            180.,
            linear_color_stop(theme::black_a(0.), 0.45),
            linear_color_stop(theme::black_a(0.6), 1.),
        )));
        let circle = div()
            .absolute()
            .top(relative(0.5))
            .left(relative(0.5))
            .mt(px(-play / 2.))
            .ml(px(-play / 2.))
            .size(px(play))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme::black_a(0.5))
            .border_1()
            .border_color(theme::white_a(0.7))
            .group_hover(group.clone(), |s| s.bg(theme::replay()).border_color(theme::replay()))
            // Under the pointer the circle turns yellow and the arrow dark.
            .child(div().flex().group_hover(group.clone(), |s| s.opacity(0.)).child(icon(Icon::PlayArrow, px(play * 0.6), white())))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .child(icon(Icon::PlayArrow, px(play * 0.6), theme::black_a(0.85))),
            );
        tile = tile.child(circle);
        if let Some(time) = t.time {
            tile = tile.child(
                label_box()
                    .top(px(6.))
                    .left(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(theme::replay())
                    .child(icon(Icon::Videocam, px(11.), theme::replay()))
                    .child(time),
            );
        }
        let length = seconds.map(clip_length);
        if let Some(caption) = t.caption.filter(|_| t.w >= 140.) {
            tile = tile.child(
                div()
                    .absolute()
                    .left(px(8.))
                    .right(px(if length.is_some() { 46. } else { 8. }))
                    .bottom(px(6.))
                    .text_size(px(11.5))
                    .line_height(px(16.))
                    .text_color(white())
                    .truncate()
                    .child(caption),
            );
        }
        if let Some(length) = length {
            tile = tile.child(label_box().right(px(5.)).bottom(px(5.)).text_color(white()).child(length));
        }
        // The ring goes last, over the picture's edge.
        tile.child(div().absolute().inset_0().rounded(px(radius)).border_1().border_color(theme::replay_alpha(0.6)))
    }
}

/// A small dark label over a poster.
fn label_box() -> Div {
    div().absolute().px(px(5.)).rounded(px(4.)).bg(theme::black_a(0.62)).text_size(px(10.5)).line_height(px(16.)).font_weight(theme::W_BOLD)
}

/// "N replays", in the replay yellow.
pub(super) fn replays_badge(n: usize) -> Div {
    let ink = theme::readable(theme::replay());
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .px(px(7.))
        .rounded(px(5.))
        .bg(theme::replay_alpha(0.14))
        .text_size(px(11.5))
        .line_height(px(18.))
        .font_family(theme::font_body())
        .font_weight(theme::W_REGULAR)
        .text_color(ink)
        .child(icon(Icon::Videocam, px(12.), ink))
        .child(trn!(n, "{n} replay", "{n} replays"))
}

/// [`replays_badge`] that acts on a click.
pub(super) fn replays_chip(id: SharedString, n: usize) -> Stateful<Div> {
    replays_badge(n).id(id).cursor_pointer().hover(|s| s.bg(theme::replay_alpha(0.24)))
}

/// A replay's length: "0:30", "12:05", "1:02:03".
pub(super) fn clip_length(seconds: f64) -> String {
    let s = seconds.round().max(0.) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::clip_length;

    #[test]
    fn lengths_read_like_a_player() {
        assert_eq!(clip_length(30.2), "0:30");
        assert_eq!(clip_length(59.6), "1:00");
        assert_eq!(clip_length(725.), "12:05");
        assert_eq!(clip_length(3723.), "1:02:03");
    }
}
