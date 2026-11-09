//! The facts card's "Age rating" row, and hiding the age ratings from it:
//! an eye on the row's right while pointed at turns Settings → Metadata →
//! "Show age ratings" off, with a toast that says where it comes back.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings;
use heronite_i18n::tr;

use super::focus_left::fact_row;
use super::vm::AgeVm;
use super::{CollectionView, ToastAction};
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tooltip};

impl CollectionView {
    /// The board and its code on a chip coloured by age (what it means and
    /// why in the tooltip), or "Unrated".
    pub(super) fn age_row(&self, age: Option<&AgeVm>, cx: &mut Context<Self>) -> AnyElement {
        // Only while the row is pointed at, sliding in from the right so the
        // chip makes room for it smoothly.
        let hide = self.facts.age_hovered.then(|| {
            let button = div()
                .id("focus-age-hide")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.1)))
                .child(icon(Icon::VisibilityOff, px(14.), theme::ink_a(0.6)))
                .tooltip(tooltip(tr!("Hide age ratings")))
                .on_click(cx.listener(|this, _, _, cx| this.set_age_ratings_shown(false, cx)));
            let slot = div().flex().flex_none().justify_end().overflow_hidden().child(button);
            if theme::animations() {
                slot.with_animation(
                    "focus-age-hide-in",
                    Animation::new(Duration::from_millis(160)).with_easing(ease_out_quint()),
                    |el, t| el.w(px(26. * t)).opacity(t),
                )
                .into_any_element()
            } else {
                slot.w(px(26.)).into_any_element()
            }
        });
        let (value, tip): (Div, SharedString) = match age {
            None => (div().text_color(theme::ink_a(0.66)).child(tr!("Unrated")), tr!("No age rating was found for this game.").into()),
            Some(age) => {
                let tone = theme::age_tone(age.min_age);
                let value = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(12.)).text_color(theme::ink_a(0.6)).child(age.board.clone()))
                    .child(
                        div()
                            .px(px(7.))
                            .rounded(px(5.))
                            .bg(Rgba { a: 0.1, ..tone })
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::readable(tone))
                            .child(age.badge.clone()),
                    );
                let tip = if age.reasons.is_empty() { age.title.clone() } else { format!("{}\n{}", age.title, age.reasons).into() };
                (value, tip)
            }
        };
        let value = div().flex().items_center().justify_end().child(value).children(hide);
        fact_row(tr!("Age rating"), value)
            .id("focus-age")
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.facts.age_hovered != *hovered {
                    this.facts.age_hovered = *hovered;
                    cx.notify();
                }
            }))
            .tooltip(tooltip(tip))
            .into_any_element()
    }

    /// Settings → Metadata → "Show age ratings", saved at once. Hiding says
    /// where to turn them back on, with Undo.
    pub(super) fn set_age_ratings_shown(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(lib) = &self.lib else { return };
        if lib.settings.metadata.show_age_ratings == on {
            return;
        }
        let mut patch = lib.settings.metadata.clone();
        patch.show_age_ratings = on;
        if let Err(err) = settings::save_metadata(&self.data_paths, &patch) {
            self.show_toast(tr!("Could not save the setting: {err}", err = format!("{err:#}")), true, cx);
            return;
        }
        // The row goes away under the pointer: no leave event comes.
        self.facts.age_hovered = false;
        self.reload(cx);
        if on {
            self.toast = None;
        } else {
            self.show_toast(tr!("Age ratings hidden. Turn them back on in Settings \u{2192} Metadata."), false, cx);
            self.toast_action = Some(ToastAction::ShowAgeRatings);
        }
        cx.notify();
    }
}
