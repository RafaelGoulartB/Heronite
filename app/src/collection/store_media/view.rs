//! The "Store media" section: a 16:9 stage with the picked trailer or
//! screenshot, and the strip of everything the store has under it (the
//! trailers first, each with a play mark).
//!
//! On a trailer the stage shows its still and a play button; playing, a bar
//! fades in while the pointer is over it: play / pause, the time, the seek
//! bar, the sound (Linux) and the store page. Arrows on its sides step
//! through the items.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_images::{ImageRef, Variant};

use super::{List, PlayState};
use crate::collection::focus::h3;
use crate::collection::replays::clip_length;
use crate::collection::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tooltip};

/// Thumbnails of the strip, logical px.
const THUMB_W: f32 = 128.;
const THUMB_H: f32 = 72.;
/// The stage never grows past this width (a very wide column).
const STAGE_MAX_W: f32 = 1100.;
const STAGE_GROUP: &str = "store-media-stage";

/// A round dark button on the stage.
fn stage_button(id: impl Into<ElementId>, ic: Icon, d: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(d))
        .rounded_full()
        .cursor_pointer()
        .bg(theme::rgb_a(10, 12, 14, 0.55))
        .hover(|s| s.bg(theme::rgb_a(10, 12, 14, 0.8)))
        .child(icon(ic, px(d * 0.55), rgb(0xf4f6f8)))
}

fn spinner(id: &'static str) -> impl IntoElement {
    icon(Icon::FaSpinner, px(22.), rgb(0xf4f6f8)).with_animation(
        id,
        Animation::new(Duration::from_secs(1)).repeat().with_max_fps(30.),
        |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
    )
}

impl CollectionView {
    fn media_image(&mut self, url: &str, w: f32, h: f32, scale: f32, priority: i32) -> Option<ImageSource> {
        if url.is_empty() {
            return None;
        }
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("sm:{pw}x{ph}:{url}").into();
        let chain = || vec![ImageRef::Url(url.to_string())];
        self.images.get_or_request(&id, chain, pw, ph, Variant::Cover, priority).map(ImageSource::Render)
    }

    /// The section, `width` px wide; `None` while the game has no store
    /// media (or the section is off).
    pub(in crate::collection) fn store_media_section(&mut self, width: f32, scale: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let media = match &self.media.list {
            List::Ready(m) => Some(m.clone()),
            List::Loading => None,
            List::None | List::Missing => return None,
        };
        self.media.drawn = true;
        let stage_w = width.min(STAGE_MAX_W);
        let stage_h = (stage_w * 9. / 16.).round();
        let Some(media) = media else {
            let stage = div()
                .w(px(stage_w))
                .h(px(stage_h))
                .rounded(px(12.))
                .bg(theme::recess_a(0.35))
                .flex()
                .items_center()
                .justify_center()
                .child(spinner("sm-list-spin"));
            return Some(div().flex().flex_col().child(h3(tr!("Store media"))).child(stage).into_any_element());
        };
        let (videos, shots) = (media.videos.len(), media.screenshots.len());
        let total = videos + shots;
        let pick = self.media.pick.min(total.saturating_sub(1));

        let mut counts = Vec::new();
        if videos > 0 {
            counts.push(trn!(videos, "{n} video", "{n} videos"));
        }
        if shots > 0 {
            counts.push(trn!(shots, "{n} screenshot", "{n} screenshots"));
        }
        let page = media.page.clone();
        let source = div()
            .id("sm-source")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(3.))
            .rounded(px(8.))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .hover(|s| s.bg(theme::ink_a(0.08)).text_color(theme::text_primary()))
            .tooltip(tooltip(tr!("Open the store page")))
            .child(icon(Icon::BrandSteam, px(14.), theme::text_muted()))
            .child("Steam")
            .child(icon(Icon::OpenInNew, px(13.), theme::text_muted()))
            .on_click(cx.listener(move |this, _, _, cx| this.open_url(page.clone(), cx)));
        let header = div().flex().items_center().justify_between().gap(px(12.)).child(h3(tr!("Store media"))).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mb(px(8.8))
                .min_w_0()
                .child(div().min_w_0().truncate().text_size(px(12.5)).text_color(theme::text_muted()).child(counts.join(" \u{b7} ")))
                .child(source),
        );

        let stage = self.media_stage(&media, pick, stage_w, stage_h, scale, cx);
        let mut strip = div().id("sm-strip").flex().gap(px(8.)).pb(px(4.)).overflow_x_scroll().track_scroll(&self.media.strip);
        for i in 0..total {
            let (url, is_video, name) = match i.checked_sub(videos) {
                None => (media.videos[i].thumb.clone(), true, media.videos[i].name.clone()),
                Some(s) => (media.screenshots[s].thumb.clone(), false, String::new()),
            };
            let src = self.media_image(&url, THUMB_W, THUMB_H, scale, 25);
            let active = i == pick;
            let mut tile = div()
                .id(("sm-thumb", i))
                .relative()
                .flex_none()
                .w(px(THUMB_W))
                .h(px(THUMB_H))
                .rounded(px(8.))
                .overflow_hidden()
                .cursor_pointer()
                .bg(theme::recess_a(0.35))
                .border_2()
                .border_color(if active { theme::accent().into() } else { theme::ink_a(0.0) })
                .when(!active, |d| d.opacity(0.78).hover(|s| s.opacity(1.)))
                .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
                .on_click(cx.listener(move |this, _, _, cx| this.media_pick(i, cx)));
            if is_video {
                tile = tile.child(
                    div()
                        .absolute()
                        .bottom(px(5.))
                        .left(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded_full()
                        .bg(theme::rgb_a(10, 12, 14, 0.7))
                        .child(icon(Icon::PlayArrow, px(14.), rgb(0xf4f6f8))),
                );
                if !name.is_empty() {
                    tile = tile.tooltip(tooltip(name));
                }
            }
            strip = strip.child(tile);
        }
        Some(div().flex().flex_col().w(px(stage_w)).gap(px(10.)).child(header).child(stage).child(strip).into_any_element())
    }

    fn media_stage(
        &mut self,
        media: &heronite_metadata::store_media::StoreMedia,
        pick: usize,
        w: f32,
        h: f32,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let videos = media.videos.len();
        let total = videos + media.screenshots.len();
        let mut stage = div()
            .id("sm-stage")
            .group(STAGE_GROUP)
            .relative()
            .w(px(w))
            .h(px(h))
            .rounded(px(12.))
            .overflow_hidden()
            .bg(rgb(0x05070a))
            .border_1()
            .border_color(theme::ink_a(0.08));
        match pick.checked_sub(videos) {
            Some(s) => {
                let full = media.screenshots[s].full.clone();
                // The small copy shows while the full one loads.
                let src = self
                    .media_image(&full, w, h, scale, 30)
                    .or_else(|| self.media_image(&media.screenshots[s].thumb.clone(), THUMB_W, THUMB_H, scale, 25));
                stage = stage.children(src.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Contain)));
            }
            None => stage = self.media_video(stage, media, pick, w, h, scale, cx),
        }
        if total > 1 {
            let arrow = |id: &'static str, ic: Icon, to: usize, left: bool| {
                div()
                    .absolute()
                    .top(px(h / 2. - 20.))
                    .when(left, |d| d.left(px(10.)))
                    .when(!left, |d| d.right(px(10.)))
                    .opacity(0.)
                    .group_hover(STAGE_GROUP, |s| s.opacity(1.))
                    .child(stage_button(id, ic, 40.).on_click(cx.listener(move |this, _, _, cx| this.media_pick(to, cx))))
            };
            stage = stage.child(arrow("sm-prev", Icon::ChevronLeft, if pick == 0 { total - 1 } else { pick - 1 }, true)).child(arrow(
                "sm-next",
                Icon::ChevronRight,
                (pick + 1) % total,
                false,
            ));
        }
        stage
    }

    /// A trailer on the stage: its frames while it plays, else its still
    /// with the play button; the controls over its bottom.
    fn media_video(
        &mut self,
        mut stage: Stateful<Div>,
        media: &heronite_metadata::store_media::StoreMedia,
        video: usize,
        w: f32,
        h: f32,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let v = &media.videos[video];
        let player = self.media.player.as_ref().filter(|p| p.video == video);
        let frame = player.and_then(|p| p.frame.clone());
        let state = player.map(|p| p.state.clone());
        let (position, seconds, sound) = player.map_or((0., 0., false), |p| (p.position(), p.seconds, p.sound));
        match frame {
            Some(f) => stage = stage.child(img(ImageSource::Render(f)).absolute().inset_0().size_full().object_fit(ObjectFit::Contain)),
            None => {
                let still = self.media_image(&v.thumb.clone(), w, h, scale, 30);
                stage = stage.children(still.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).opacity(0.75)));
            }
        }
        let center = div().absolute().inset_0().flex().flex_col().items_center().justify_center().gap(px(10.));
        stage = match &state {
            None | Some(PlayState::Ended) => stage.child(
                center.child(
                    stage_button("sm-play-big", if state.is_some() { Icon::Replay } else { Icon::PlayArrow }, 64.)
                        .on_click(cx.listener(|this, _, _, cx| this.media_toggle_play(cx))),
                ),
            ),
            Some(PlayState::Loading) => stage.child(center.child(spinner("sm-play-spin"))),
            Some(PlayState::Failed(why)) => stage.child(
                center
                    .bg(theme::rgb_a(5, 7, 10, 0.6))
                    .px(px(24.))
                    .text_size(px(13.5))
                    .text_color(rgb(0xf4f6f8))
                    .child(div().text_center().child(why.clone()))
                    .child(
                        div()
                            .id("sm-failed-open")
                            .px(px(12.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .bg(theme::rgb_a(255, 255, 255, 0.12))
                            .hover(|s| s.bg(theme::rgb_a(255, 255, 255, 0.2)))
                            .child(tr!("Watch on the store page"))
                            .on_click(cx.listener({
                                let page = media.page.clone();
                                move |this, _, _, cx| this.open_url(page.clone(), cx)
                            })),
                    ),
            ),
            Some(PlayState::Playing | PlayState::Paused) => stage,
        };
        let title = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .px(px(14.))
            .py(px(10.))
            .bg(linear_gradient(180., linear_color_stop(theme::rgb_a(0, 0, 0, 0.55), 0.), linear_color_stop(theme::rgb_a(0, 0, 0, 0.), 1.)))
            .text_size(px(13.))
            .text_color(rgb(0xf4f6f8))
            .truncate()
            .child(v.name.clone());
        let playing = matches!(state, Some(PlayState::Playing | PlayState::Loading));
        stage = stage.child(if playing { title.opacity(0.).group_hover(STAGE_GROUP, |s| s.opacity(1.)) } else { title });
        if !matches!(state, Some(PlayState::Playing | PlayState::Paused | PlayState::Loading)) {
            return stage;
        }
        let bar = self.media_controls(position, seconds, sound, playing, cx);
        stage.child(if playing { bar.opacity(0.).group_hover(STAGE_GROUP, |s| s.opacity(1.)) } else { bar })
    }

    fn media_controls(&mut self, position: f64, seconds: f64, sound: bool, playing: bool, cx: &mut Context<Self>) -> Div {
        let fraction = if seconds > 0. { (position / seconds).clamp(0., 1.) as f32 } else { 0. };
        let cell = self.media.bar.clone();
        let seek = div()
            .id("sm-seek")
            .relative()
            .flex_1()
            .min_w_0()
            .py(px(8.))
            .cursor_pointer()
            .child(
                div()
                    .relative()
                    .h(px(4.))
                    .rounded_full()
                    .bg(theme::rgb_a(255, 255, 255, 0.25))
                    .child(div().absolute().top_0().left_0().h_full().w(relative(fraction)).rounded_full().bg(theme::accent()))
                    .child(canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().size_full()),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let b = this.media.bar.get();
                    let w = f32::from(b.size.width);
                    if w > 0. {
                        this.media_seek(f32::from(ev.position.x - b.origin.x) / w, cx);
                    }
                    cx.stop_propagation();
                }),
            );
        let time = if seconds > 0. { format!("{} / {}", clip_length(position), clip_length(seconds)) } else { clip_length(position) };
        let mut bar = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .pt(px(18.))
            .pb(px(8.))
            .bg(linear_gradient(0., linear_color_stop(theme::rgb_a(0, 0, 0, 0.7), 0.), linear_color_stop(theme::rgb_a(0, 0, 0, 0.), 1.)))
            .text_size(px(12.))
            .text_color(rgb(0xf4f6f8))
            .child(
                stage_button("sm-play", if playing { Icon::Pause } else { Icon::PlayArrow }, 32.)
                    .on_click(cx.listener(|this, _, _, cx| this.media_toggle_play(cx))),
            )
            .child(div().flex_none().child(time))
            .child(seek);
        if heronite_media::stream::sound_supported() {
            let label = if sound { tr!("Mute") } else { tr!("Sound on") };
            bar = bar.child(
                stage_button("sm-sound", if sound { Icon::VolumeUp } else { Icon::VolumeOff }, 32.)
                    .tooltip(tooltip(label))
                    .on_click(cx.listener(|this, _, _, cx| this.media_toggle_sound(cx))),
            );
        }
        bar
    }
}
