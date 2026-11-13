//! The notice in the bottom right corner (see `console::summon`): the same
//! card as the overlay's notices, with a ring around the controller that
//! fills while the button is held and a line at the bottom that runs out
//! with the question.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use crate::console::kit::{self, u};
use crate::icons::Icon;
use crate::services::widgets::progress_ring;
use crate::theme;
use crate::ui::icon;

/// What the notice says.
#[derive(Clone, Debug)]
pub enum Shown {
    /// A controller connected ("Ask"): hold `button` before `until`.
    Asking { name: String, button: String, until: Instant, total: Duration },
    /// The button is held since `since`; `name`: the controller asked about.
    Holding { since: Instant, total: Duration, name: Option<String> },
    /// Held long enough: the window is on its way.
    Opening,
}

pub struct NoticeView {
    shown: Shown,
    /// Console scale of the display (the window itself is small).
    scale: f32,
    /// Debug captures: the ring and the line stop part of the way.
    frozen: Option<f32>,
}

impl NoticeView {
    pub fn new(shown: Shown, scale: f32) -> Self {
        Self { shown, scale, frozen: None }
    }

    pub fn set(&mut self, shown: Shown, cx: &mut Context<Self>) {
        self.shown = shown;
        cx.notify();
    }

    pub fn freeze(&mut self, at: f32) {
        self.frozen = Some(at);
    }
}

/// The part of `total` gone since `since` (0..=1).
fn gone(since: Instant, total: Duration, now: Instant) -> f32 {
    (now.saturating_duration_since(since).as_secs_f32() / total.as_secs_f32().max(0.001)).clamp(0., 1.)
}

impl Render for NoticeView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Over everything, whatever the theme.
        theme::set_dark_only(true);
        kit::set_scale_value(self.scale);
        let now = Instant::now();
        let blue: Hsla = theme::console_btn_x().into();
        let green: Hsla = theme::success().into();
        let (kicker, title, sub, ring, left) = match &self.shown {
            Shown::Asking { name, button, until, total } => {
                let left = self.frozen.unwrap_or_else(|| 1. - gone(until.checked_sub(*total).unwrap_or(now), *total, now));
                (
                    tr!("Controller connected"),
                    name.clone(),
                    tr!("Hold {button} to enter console mode", button = button.clone()),
                    Some(0.),
                    Some(left),
                )
            }
            Shown::Holding { since, total, name } => {
                let p = self.frozen.unwrap_or_else(|| gone(*since, *total, now));
                match name {
                    Some(name) => (tr!("Controller connected"), name.clone(), tr!("Keep holding…").to_string(), Some(p), None),
                    None => (
                        tr!("Console mode"),
                        tr!("Hold to enter console mode").to_string(),
                        tr!("Release to cancel").to_string(),
                        Some(p),
                        None,
                    ),
                }
            }
            Shown::Opening => {
                (tr!("Console mode"), tr!("Opening console mode…").to_string(), tr!("Heronite comes to the front").to_string(), None, None)
            }
        };
        // The ring and the line move on their own.
        if self.frozen.is_none() && !matches!(self.shown, Shown::Opening) {
            window.request_animation_frame();
        }
        let opening = matches!(self.shown, Shown::Opening);
        let accent = if opening { green } else { blue };
        let lead = match ring {
            Some(p) => div()
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(u(52.))
                .child(progress_ring(p * 100., f32::from(u(3.5)), blue).absolute().inset_0())
                .child(icon(Icon::SportsEsports, u(24.), theme::text_primary())),
            None => {
                div().flex().flex_none().items_center().justify_center().size(u(52.)).rounded(u(14.)).bg(green.opacity(0.16)).child(icon(
                    Icon::Check,
                    u(28.),
                    green,
                ))
            }
        };
        let card = div();
        // Debug captures draw it in a window of any size: the card keeps its own.
        let card = match self.frozen {
            Some(_) => card.w(u(super::NOTICE_SIZE.0)).h(u(super::NOTICE_SIZE.1)),
            None => card.size_full(),
        };
        card.relative()
            .overflow_hidden()
            .font_family(theme::font_body())
            .flex()
            .items_center()
            .gap(u(14.))
            .px(u(16.))
            .rounded(u(18.))
            .bg(theme::overlay_fill())
            .border_1()
            .border_color(theme::white_a(0.12))
            .child(lead)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(u(3.))
                    .child(div().text_size(u(11.5)).font_weight(theme::W_BOLD).text_color(accent).truncate().child(kicker.to_uppercase()))
                    .child(div().text_size(u(16.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).truncate().child(title))
                    .child(div().text_size(u(13.)).text_color(theme::text_muted()).truncate().child(sub)),
            )
            .children(left.map(|left| div().absolute().left_0().bottom_0().h(u(3.)).w(relative(left.clamp(0., 1.))).bg(blue)))
    }
}
