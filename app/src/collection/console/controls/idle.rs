//! The idle screen of console mode (`idle` in console.json): after
//! [`IDLE_AFTER`] without a press, Attract fades in a slow slideshow of game
//! art with the clock, and Dim darkens the screen. The next press wakes it
//! and is swallowed. Timers run only while the screen is shown.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::Idle as IdleMode;
use heronite_images::Variant;

use super::super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::{look, motion};
use crate::theme;

/// Quiet time before the idle screen shows.
pub(super) const IDLE_AFTER: Duration = Duration::from_secs(5 * 60);
/// Seconds each picture stays.
const SLIDE_SECS: u64 = 14;
/// The slideshow is slow: a low frame rate is enough.
const SLIDE_FPS: f32 = 24.;
/// Pictures in one round.
const MAX_SLIDES: usize = 60;

pub(super) struct Idle {
    mode: IdleMode,
    /// Games whose art is shown, in a shuffled order.
    order: Vec<SharedString>,
    at: usize,
    /// The art on screen (and the one before it, under the fade).
    cur: Option<(SharedString, Arc<RenderImage>)>,
    prev: Option<Arc<RenderImage>>,
    title: SharedString,
    _slides: Task<()>,
}

/// `keys` in a random order for `seed` (xorshift Fisher-Yates), at most `MAX_SLIDES`.
fn shuffled(mut keys: Vec<SharedString>, seed: u64) -> Vec<SharedString> {
    let mut x = seed | 1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in (1..keys.len()).rev() {
        keys.swap(i, (next() % (i as u64 + 1)) as usize);
    }
    keys.truncate(MAX_SLIDES);
    keys
}

impl CollectionView {
    /// Shows the idle screen of `mode` (Attract falls back to Dim without art).
    pub(super) fn start_idle(&mut self, mode: IdleMode, cx: &mut Context<Self>) {
        if mode == IdleMode::Off || self.console.controls.idle.is_some() {
            return;
        }
        let mut keys: Vec<SharedString> = self.stage_chains.iter().filter(|(_, c)| !c.is_empty()).map(|(k, _)| k.clone()).collect();
        keys.sort();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
        let order = if mode == IdleMode::Attract { shuffled(keys, seed) } else { Vec::new() };
        let mode = if order.is_empty() { IdleMode::Dim } else { mode };
        let slides = cx.spawn(async move |this, cx| {
            // Dim has nothing to advance.
            if mode == IdleMode::Dim {
                return;
            }
            loop {
                cx.background_executor().timer(Duration::from_secs(SLIDE_SECS)).await;
                if this.update(cx, |this, cx| this.next_slide(cx)).is_err() {
                    break;
                }
            }
        });
        let title = order.first().map_or(SharedString::default(), |k| self.game_title(k));
        self.console.controls.idle = Some(Idle { mode, order, at: 0, cur: None, prev: None, title, _slides: slides });
        cx.notify();
    }

    fn game_title(&self, key: &SharedString) -> SharedString {
        self.lib.as_ref().and_then(|l| l.get(key)).map_or(SharedString::default(), |g| g.title.clone().into())
    }

    fn next_slide(&mut self, cx: &mut Context<Self>) {
        let Some(idle) = self.console.controls.idle.as_mut().filter(|i| !i.order.is_empty()) else { return };
        idle.at = (idle.at + 1) % idle.order.len();
        let key = idle.order[idle.at].clone();
        let title = self.game_title(&key);
        if let Some(idle) = self.console.controls.idle.as_mut() {
            idle.title = title;
        }
        cx.notify();
    }

    /// The idle layer, drawn over everything while it shows.
    pub(super) fn render_idle(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let idle = self.console.controls.idle.as_ref()?;
        let (mode, key) = (idle.mode, idle.order.get(idle.at).cloned());
        let layer = div()
            .id("console-idle")
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::black_a(if mode == IdleMode::Dim { 0.92 } else { 1. }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.console.controls.idle = None;
                    cx.notify();
                }),
            );
        let layer = match (mode, key) {
            (IdleMode::Attract, Some(key)) => layer.child(self.slide(&key, window)).child(self.idle_clock()),
            _ => layer,
        };
        Some(motion::fade_in("console-idle", layer, 1400))
    }

    /// The picture of `key` (slowly growing), fading in over the last one.
    fn slide(&mut self, key: &SharedString, window: &Window) -> Div {
        let scale = window.scale_factor();
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let shrink = (1920. / (w * scale)).min(1.);
        let (pw, ph) = ((w * scale * shrink) as u32, (h * scale * shrink) as u32);
        // The id of the console background with no blur: the texture is shared.
        let id: SharedString = format!("s:{key}:art:{pw}x{ph}:0:console").into();
        let chains = &self.stage_chains;
        let ready = self.images.get_or_request(
            &id,
            || chains.get(key).cloned().unwrap_or_default(),
            pw,
            ph,
            Variant::Stage { blur_px: 0., saturation: 1. },
            1000,
        );
        let (cur, prev) = match self.console.controls.idle.as_mut() {
            Some(idle) => {
                if let Some(img) = ready.filter(|_| idle.cur.as_ref().is_none_or(|(k, _)| k != key)) {
                    idle.prev = idle.cur.take().map(|(_, i)| i);
                    idle.cur = Some((key.clone(), img));
                }
                (idle.cur.clone(), idle.prev.clone())
            }
            None => (None, None),
        };
        let still = move |i: Arc<RenderImage>| {
            img(ImageSource::Render(i)).absolute().left(px(0.)).top(px(0.)).w(px(w)).h(px(h)).object_fit(ObjectFit::Cover)
        };
        let drift = move |i: Arc<RenderImage>, tag: SharedString| -> AnyElement {
            if !motion::on() {
                return still(i).into_any_element();
            }
            // Grows 6% over the slide, from the centre.
            let pic = img(ImageSource::Render(i)).absolute().object_fit(ObjectFit::Cover);
            pic.with_animation(tag, Animation::new(Duration::from_secs(SLIDE_SECS + 2)).with_max_fps(SLIDE_FPS), move |el, t| {
                el.left(px(-w * 0.03 * t)).top(px(-h * 0.03 * t)).w(px(w * (1. + 0.06 * t))).h(px(h * (1. + 0.06 * t)))
            })
            .into_any_element()
        };
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .children(prev.map(|i| still(i).into_any_element()))
            .children(cur.map(|(k, i)| {
                let tag = SharedString::from(format!("idle-slide-{k}"));
                motion::fade_in(SharedString::from(format!("idle-fade-{k}")), div().absolute().inset_0().child(drift(i, tag)), 1600)
            }))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(theme::black_a(0.8), 0.),
                linear_color_stop(theme::black_a(0.), 0.5),
            )))
    }

    /// The clock (as the theme sets it) and the game shown, bottom left.
    fn idle_clock(&self) -> Div {
        let title = self.console.controls.idle.as_ref().map(|i| i.title.clone()).unwrap_or_default();
        div()
            .absolute()
            .left(u(64.))
            .bottom(u(56.))
            .flex()
            .flex_col()
            .gap(u(4.))
            .child(
                div()
                    .font_family(kit::heading_font())
                    .font_weight(theme::W_BOLD)
                    .text_size(u(72.))
                    .text_color(theme::text_primary())
                    .children(look::clock_text(chrono::Local::now())),
            )
            .child(div().text_size(u(18.)).text_color(theme::text_muted()).child(title))
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use super::{shuffled, MAX_SLIDES};
    use gpui_kit::SharedString;

    fn keys(n: usize) -> Vec<SharedString> {
        (0..n).map(|i| SharedString::from(format!("g{i}"))).collect()
    }

    #[test]
    fn shuffling_keeps_every_game_once_and_depends_on_the_seed() {
        let a = shuffled(keys(20), 1);
        let mut sorted = a.clone();
        sorted.sort();
        let mut expected = keys(20);
        expected.sort();
        assert_eq!(sorted, expected);
        assert_ne!(a, shuffled(keys(20), 2));
        assert_eq!(a, shuffled(keys(20), 1));
    }

    #[test]
    fn a_round_is_capped_and_nothing_is_fine() {
        assert_eq!(shuffled(keys(500), 9).len(), MAX_SLIDES);
        assert!(shuffled(Vec::new(), 9).is_empty());
    }
}
