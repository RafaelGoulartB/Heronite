//! A launch before the game runs: the step it is at (creating the prefix,
//! installing redistributables, downloading cloud saves, Steam starting the
//! game) in place of the Play button, and a way to cancel it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::CollectionView;
use crate::icons::Icon;
use crate::services::widgets::busy_bar;
use crate::theme;
use crate::ui::icon;

/// Where a launch is (from `LaunchEvent::Preparing`).
#[derive(Clone, Debug)]
pub(super) struct LaunchStep {
    pub message: SharedString,
    /// Stopping now is safe; otherwise the step finishes first.
    pub cancellable: bool,
    /// The user cancelled: waiting for the step to stop.
    pub cancelling: bool,
}

impl LaunchStep {
    pub fn starting() -> Self {
        Self { message: tr!("Starting...").into(), cancellable: true, cancelling: false }
    }

    /// What the step says, or that it is being cancelled.
    pub fn label(&self) -> SharedString {
        if self.cancelling {
            tr!("Cancelling...").into()
        } else {
            self.message.clone()
        }
    }
}

impl CollectionView {
    /// The launch step of the game behind `key`, while it prepares.
    pub(super) fn launch_step(&self, key: &str) -> Option<&LaunchStep> {
        let app = &self.lib.as_ref()?.get(key)?.app_name;
        self.preparing.get(app)
    }

    /// A new step of a launch; a cancel asked for earlier stays shown.
    pub(super) fn set_launch_step(&mut self, app_name: String, message: String, cancellable: bool) {
        let step = self.preparing.entry(app_name).or_insert_with(LaunchStep::starting);
        step.message = message.into();
        step.cancellable = cancellable;
    }

    /// Stops a launch that has not started the game yet.
    pub(super) fn cancel_launch(&mut self, app_name: &str, cx: &mut Context<Self>) {
        if let Some(step) = self.preparing.get_mut(app_name) {
            step.cancelling = true;
        }
        if !self.runtime.stop(app_name) {
            self.runtime.force_clear(app_name);
        }
        cx.notify();
    }

    /// The step, a busy bar and Cancel, the size of the Play button.
    pub(super) fn launch_progress(&self, key: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let step = self.launch_step(key)?.clone();
        let app = self.lib.as_ref()?.get(key)?.app_name.clone();
        let text = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_1()
            .min_w_0()
            .child(div().line_clamp(2).text_ellipsis().text_size(px(13.33)).text_color(theme::text_bright()).child(step.label()))
            .child(busy_bar(SharedString::from(format!("launch-sweep-{app}")), 5.))
            .when(!step.cancellable && !step.cancelling, |d| {
                d.child(div().text_size(px(11.)).text_color(theme::ink_a(0.6)).child(tr!("This step finishes before the launch can stop")))
            });
        // Round, as the download's cancel next to it in the same panel.
        let cancel = (step.cancellable && !step.cancelling).then(|| {
            div()
                .id("launch-cancel")
                .size(px(30.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .bg(theme::round_fill(0.72))
                .hover(|s| s.bg(theme::round_fill_hover()))
                .child(icon(Icon::Close, px(16.), theme::round_icon()))
                .on_click(cx.listener(move |this, _, _, cx| this.cancel_launch(&app, cx)))
        });
        Some(
            div()
                .id("launch-progress")
                .flex()
                .items_center()
                .gap(px(10.))
                .w_full()
                .min_h(px(46.))
                .px(px(14.))
                .py(px(8.))
                .rounded(px(14.))
                .bg(theme::ink_a(0.08))
                .border_1()
                .border_color(theme::ink_a(0.12))
                .child(text)
                .children(cancel)
                .into_any_element(),
        )
    }
}
