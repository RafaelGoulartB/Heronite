//! Short notices in a top corner of the screen (screenshot saved, the
//! volume level), shown by the shortcuts that do not open the panel. A new
//! notice replaces the one on screen and restarts its timer.
//!
//! Achievement notices wait their turn instead: they queue (a few, then one
//! "N more achievements"), each stays longer, and one that another notice
//! interrupts shows again after it.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{window, OverlayService};
use crate::console::kit::{self, u};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// How long a notice stays.
const SHOWN_FOR: Duration = Duration::from_millis(1800);
/// How long an achievement's notice stays.
const ACHIEVEMENT_FOR: Duration = Duration::from_secs(5);
/// Achievement notices waiting at most; more are summed up in one.
const QUEUE_MAX: usize = 4;

#[derive(Clone)]
pub struct Notice {
    pub icon: Icon,
    pub title: String,
    pub sub: Option<String>,
    pub error: bool,
    /// Draws a level bar (0..=100+).
    pub volume: Option<u16>,
    /// A picture in place of the icon (an achievement's).
    pub image: Option<Arc<RenderImage>>,
    /// Colour of the sub line and the border (a rare achievement's tier).
    pub accent: Option<Hsla>,
    pub duration: Duration,
    /// An achievement: it queues instead of replacing.
    pub achievement: bool,
}

impl std::fmt::Debug for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notice").field("title", &self.title).field("sub", &self.sub).field("achievement", &self.achievement).finish()
    }
}

impl Notice {
    pub fn text(icon: Icon, title: impl Into<String>, error: bool) -> Self {
        Self {
            icon,
            title: title.into(),
            sub: None,
            error,
            volume: None,
            image: None,
            accent: None,
            duration: SHOWN_FOR,
            achievement: false,
        }
    }

    pub fn sub(mut self, sub: impl Into<String>) -> Self {
        let sub = sub.into();
        self.sub = (!sub.is_empty()).then_some(sub);
        self
    }

    pub fn volume(level: u16) -> Self {
        let icon = if level == 0 { Icon::VolumeOff } else { Icon::VolumeUp };
        Self { volume: Some(level), ..Self::text(icon, tr!("Volume"), false) }
    }

    /// "Achievement unlocked · 2.1%" under the achievement's name, the
    /// rarity's colour when it is rare.
    pub fn achievement(name: impl Into<String>, rarity: Option<f32>, image: Option<Arc<RenderImage>>) -> Self {
        let sub = match rarity {
            Some(r) => tr!("Achievement unlocked · {percent}", percent = crate::ui::achievements::percent_label(r)),
            None => tr!("Achievement unlocked").to_string(),
        };
        let accent = rarity
            .filter(|r| *r < heronite_achievements::AchievementSet::RARE_BELOW)
            .map(|r| Hsla::from(theme::rarity_colour(theme::rarity_tier(r))));
        Self { image, accent, duration: ACHIEVEMENT_FOR, achievement: true, ..Self::text(Icon::Trophy, name, false) }.sub(sub)
    }

    /// The last of a long run of unlocks.
    fn more_achievements(n: usize) -> Self {
        Self {
            duration: ACHIEVEMENT_FOR,
            achievement: true,
            ..Self::text(Icon::Trophy, trn!(n, "{n} more achievement", "{n} more achievements"), false)
        }
    }
}

pub struct NoticeWindow {
    handle: WindowHandle<NoticeView>,
    /// Bumped by every notice; the timer of an older one does nothing.
    generation: u64,
    /// Windows: the monitor it was placed on.
    #[cfg(windows)]
    screen: Option<heronite_platform::overlay_window::Screen>,
    /// The notice on screen.
    shown: Notice,
}

/// Achievement notices waiting for the screen.
#[derive(Default)]
pub struct Queue {
    waiting: VecDeque<Notice>,
    /// Unlocks past [`QUEUE_MAX`], shown as one notice at the end.
    more: usize,
}

impl Queue {
    fn push(&mut self, notice: Notice) {
        if self.waiting.len() < QUEUE_MAX {
            self.waiting.push_back(notice);
        } else {
            self.more += 1;
        }
    }

    fn next(&mut self) -> Option<Notice> {
        if let Some(n) = self.waiting.pop_front() {
            return Some(n);
        }
        let more = std::mem::take(&mut self.more);
        (more > 0).then(|| Notice::more_achievements(more))
    }

    pub fn clear(&mut self) {
        self.waiting.clear();
        self.more = 0;
    }
}

impl OverlayService {
    /// Queues an achievement's notice: it shows when the screen is free.
    pub(super) fn queue_achievement(&mut self, notice: Notice, layer: bool, cx: &mut Context<Self>) {
        self.achievement_queue.push(notice);
        if self.notice.is_none() {
            if let Some(next) = self.achievement_queue.next() {
                self.show_notice(next, layer, cx);
            }
        }
    }

    pub(super) fn show_notice(&mut self, notice: Notice, layer: bool, cx: &mut Context<Self>) {
        // Windows: a topmost window would minimise a game in exclusive
        // fullscreen. Nothing shows, and the achievements waiting are dropped
        // (the one on screen goes too, or its timer would leave it there).
        if layer && window::exclusive_fullscreen() {
            self.achievement_queue.clear();
            if let Some(old) = self.notice.take() {
                let _ = old.handle.update(cx, |_, window, _| window.remove_window());
            }
            return;
        }
        // An achievement cut short by another notice shows again after it.
        if !notice.achievement {
            if let Some(shown) = self.notice.as_ref().map(|n| n.shown.clone()).filter(|n| n.achievement) {
                self.achievement_queue.waiting.push_front(shown);
            }
        }
        let generation = self.notice.as_ref().map_or(0, |n| n.generation + 1);
        let display = window::Display::current(cx);
        // A Windows notice stays where it was placed: one for another
        // monitor (the foreground window's moved) replaces it.
        #[cfg(windows)]
        if layer && self.notice.as_ref().is_some_and(|n| display.screen().is_some_and(|now| n.screen != Some(now))) {
            if let Some(old) = self.notice.take() {
                let _ = old.handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        let duration = notice.duration;
        let reused = self.notice.as_ref().is_some_and(|n| {
            let notice = notice.clone();
            n.handle
                .update(cx, |v, _, cx| {
                    v.notice = notice;
                    cx.notify();
                })
                .is_ok()
        });
        if reused {
            if let Some(n) = &mut self.notice {
                n.generation = generation;
                n.shown = notice;
            }
        } else {
            let scale = display.scale();
            let view = notice.clone();
            #[cfg(windows)]
            let screen = display.screen();
            match window::open_notice(cx, display, self.prefs.side, layer, |_, cx| cx.new(|_| NoticeView { notice: view, scale })) {
                Ok(handle) => {
                    self.notice = Some(NoticeWindow {
                        handle,
                        generation,
                        shown: notice,
                        #[cfg(windows)]
                        screen,
                    })
                }
                Err(e) => {
                    log::warn!("overlay: cannot show a notice: {e:#}");
                    // Its window is gone: the next notice opens a new one
                    // (else the achievements would wait for it forever).
                    self.notice = None;
                    return;
                }
            }
        }
        // Debug captures need the time to find the window (on Windows, the
        // native one of `HERONITE_DEBUG_OVERLAY=layer` too).
        let debug = self.debug_window || (cfg!(windows) && crate::env::value("HERONITE_DEBUG_OVERLAY").is_some());
        let shown = if debug { duration * 6 } else { duration };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shown).await;
            this.update(cx, |this, cx| {
                if this.notice.as_ref().is_some_and(|n| n.generation == generation) {
                    match this.achievement_queue.next() {
                        Some(next) => this.show_notice(next, layer, cx),
                        None => {
                            if let Some(n) = this.notice.take() {
                                let _ = n.handle.update(cx, |_, window, _| window.remove_window());
                            }
                        }
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}

pub struct NoticeView {
    notice: Notice,
    /// Console scale of the display (the window itself is small).
    scale: f32,
}

impl Render for NoticeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Over the game, whatever the theme.
        theme::set_dark_only(true);
        kit::set_scale_value(self.scale);
        let n = &self.notice;
        let accent: Hsla = if n.error { theme::danger_text().into() } else { theme::text_primary().into() };
        let sub_colour: Hsla = n.accent.unwrap_or_else(|| theme::text_muted().into());
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(u(6.))
            .child(div().text_size(u(16.)).font_weight(theme::W_BOLD).text_color(accent).truncate().child(n.title.clone()))
            .children(n.sub.clone().map(|s| div().text_size(u(13.)).text_color(sub_colour).truncate().child(s)))
            .children(n.volume.map(|v| {
                let fill = (f32::from(v.min(100)) / 100.).clamp(0., 1.);
                div()
                    .flex()
                    .items_center()
                    .gap(u(10.))
                    .child(
                        div()
                            .flex_1()
                            .h(u(6.))
                            .rounded_full()
                            .bg(theme::white_a(0.14))
                            .child(div().h_full().w(relative(fill)).rounded_full().bg(theme::console_btn_x())),
                    )
                    .child(div().w(u(40.)).text_size(u(13.)).text_color(theme::text_muted()).child(format!("{v}%")))
            }));
        let gold: Hsla = theme::achievement_gold().into();
        let lead = match (&n.image, n.achievement) {
            (Some(image), _) => div()
                .flex_none()
                .size(u(44.))
                .rounded(u(10.))
                .overflow_hidden()
                .child(img(ImageSource::Render(image.clone())).size_full().object_fit(ObjectFit::Cover)),
            (None, true) => {
                div().flex_none().size(u(44.)).rounded(u(10.)).flex().items_center().justify_center().bg(gold.opacity(0.16)).child(icon(
                    n.icon,
                    u(26.),
                    gold,
                ))
            }
            (None, false) => div().flex_none().child(icon(n.icon, u(26.), accent)),
        };
        let border = match (n.accent, n.achievement) {
            (Some(c), _) => c.opacity(0.7),
            (None, true) => gold.opacity(0.45),
            (None, false) => theme::white_a(0.1),
        };
        div()
            .size_full()
            .font_family(theme::font_body())
            .flex()
            .items_center()
            .gap(u(14.))
            .px(u(18.))
            .rounded(u(18.))
            .bg(theme::overlay_fill())
            .border_1()
            .border_color(border)
            .child(lead)
            .child(text)
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use super::{Notice, Queue, QUEUE_MAX};

    #[test]
    fn the_queue_keeps_a_few_and_sums_up_the_rest() {
        let mut q = Queue::default();
        for i in 0..7 {
            q.push(Notice::achievement(format!("a{i}"), None, None));
        }
        let titles: Vec<String> = std::iter::from_fn(|| q.next()).map(|n| n.title).collect();
        assert_eq!(titles.len(), QUEUE_MAX + 1);
        assert_eq!(titles[..QUEUE_MAX], ["a0", "a1", "a2", "a3"]);
        assert_eq!(titles[QUEUE_MAX], "3 more achievements");
        assert!(q.next().is_none());
    }

    #[test]
    fn a_rare_achievement_takes_its_tier_colour() {
        let rare = Notice::achievement("x", Some(2.), None);
        assert!(rare.accent.is_some());
        assert_eq!(rare.sub.as_deref(), Some("Achievement unlocked · 2.0%"));
        let common = Notice::achievement("y", Some(60.), None);
        assert!(common.accent.is_none());
        assert_eq!(Notice::achievement("z", None, None).sub.as_deref(), Some("Achievement unlocked"));
    }
}
