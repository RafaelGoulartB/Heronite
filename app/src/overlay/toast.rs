//! Short notices in a top corner of the screen (screenshot saved, the
//! volume level), shown by the shortcuts that do not open the panel. A new
//! notice replaces the one on screen and restarts its timer.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{window, OverlayService};
use crate::console::kit::{self, u};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// How long a notice stays.
const SHOWN_FOR: Duration = Duration::from_millis(1800);

#[derive(Clone, Debug)]
pub struct Notice {
    pub icon: Icon,
    pub title: String,
    pub sub: Option<String>,
    pub error: bool,
    /// Draws a level bar (0..=100+).
    pub volume: Option<u16>,
}

impl Notice {
    pub fn text(icon: Icon, title: impl Into<String>, error: bool) -> Self {
        Self { icon, title: title.into(), sub: None, error, volume: None }
    }

    pub fn sub(mut self, sub: impl Into<String>) -> Self {
        let sub = sub.into();
        self.sub = (!sub.is_empty()).then_some(sub);
        self
    }

    pub fn volume(level: u16) -> Self {
        let icon = if level == 0 { Icon::VolumeOff } else { Icon::VolumeUp };
        Self { icon, title: heronite_i18n::tr!("Volume").into(), sub: None, error: false, volume: Some(level) }
    }
}

pub struct NoticeWindow {
    handle: WindowHandle<NoticeView>,
    /// Bumped by every notice; the timer of an older one does nothing.
    generation: u64,
}

impl OverlayService {
    pub(super) fn show_notice(&mut self, notice: Notice, layer: bool, cx: &mut Context<Self>) {
        let generation = self.notice.as_ref().map_or(0, |n| n.generation + 1);
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
            }
        } else {
            let scale = window::scale(cx);
            match window::open_notice(cx, self.prefs.side, layer, |_, cx| cx.new(|_| NoticeView { notice, scale })) {
                Ok(handle) => self.notice = Some(NoticeWindow { handle, generation }),
                Err(e) => {
                    log::warn!("overlay: cannot show a notice: {e:#}");
                    return;
                }
            }
        }
        // Debug captures need the time to find the window.
        let shown = if self.debug_window { SHOWN_FOR * 6 } else { SHOWN_FOR };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shown).await;
            this.update(cx, |this, cx| {
                if this.notice.as_ref().is_some_and(|n| n.generation == generation) {
                    if let Some(n) = this.notice.take() {
                        let _ = n.handle.update(cx, |_, window, _| window.remove_window());
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
        kit::set_scale_value(self.scale);
        let n = &self.notice;
        let accent: Hsla = if n.error { theme::danger_text().into() } else { theme::text_primary().into() };
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(u(6.))
            .child(div().text_size(u(16.)).font_weight(theme::W_BOLD).text_color(accent).truncate().child(n.title.clone()))
            .children(n.sub.clone().map(|s| div().text_size(u(13.)).text_color(theme::text_muted()).truncate().child(s)))
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
        div()
            .size_full()
            .font_family(theme::FONT_BODY)
            .flex()
            .items_center()
            .gap(u(14.))
            .px(u(18.))
            .rounded(u(18.))
            .bg(theme::overlay_fill())
            .border_1()
            .border_color(theme::white_a(0.1))
            .child(icon(n.icon, u(26.), accent))
            .child(text)
    }
}
