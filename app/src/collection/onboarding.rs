//! The getting-started checklist on the Collection: it opens by itself once
//! per setup (a new one, or one updated from a version without it) and from
//! the bulb next to the View button, which "Don't show again" removes (the
//! command palette still opens it). Using a feature anywhere ticks its step.
//! A step that leads elsewhere (a page, the palette, console mode) brings
//! the checklist back on that step once the user is back on the Collection.
//! `HERONITE_NO_ONBOARDING=1` keeps it from opening by itself (screenshots).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::onboarding::Onboarding;

use super::CollectionView;
use crate::dialogs::onboarding::{Action, OnboardingDialog, Step};
use crate::icons::Icon;
use crate::ui::tool_button;

pub(super) struct OnboardingGlue {
    state: Onboarding,
    /// What the checklist asked for; run on the next render (it needs the window).
    pending: Option<(Step, Action)>,
    /// The step to show again once back from where the checklist led.
    resume: Option<Step>,
}

impl OnboardingGlue {
    pub(super) fn load(app: &heronite_data::AppPaths) -> Self {
        Self { state: Onboarding::load(app), pending: None, resume: None }
    }
}

impl CollectionView {
    fn save_onboarding(&self) {
        if let Err(e) = self.onboarding.state.save(&self.app_paths) {
            log::warn!("saving onboarding.json: {e:#}");
        }
    }

    pub(super) fn open_onboarding(&mut self, cx: &mut Context<Self>) {
        self.open_onboarding_at(None, cx);
    }

    /// Opens the checklist on `step` (else the first one not done).
    pub(super) fn open_onboarding_at(&mut self, step: Option<Step>, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        let done = self.onboarding.state.done.clone();
        let weak = cx.entity().downgrade();
        let view = cx.new(|cx| {
            OnboardingDialog::new(
                ctx,
                done,
                move |step, action, cx| {
                    weak.update(cx, |this, cx| {
                        this.onboarding.pending = Some((step, action));
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            )
        });
        if let Some(step) = step {
            view.update(cx, |d, cx| d.show(step, cx));
        }
        self.open_dialog(view, cx);
    }

    /// After the first load: the one time it opens by itself.
    pub(super) fn maybe_open_onboarding(&mut self, cx: &mut Context<Self>) {
        let state = &self.onboarding.state;
        if state.shown || state.hidden || self.dialog.is_some() || crate::console::active() || crate::env::flag("HERONITE_NO_ONBOARDING") {
            return;
        }
        self.onboarding.state.shown = true;
        self.save_onboarding();
        self.open_onboarding(cx);
    }

    /// Runs what the checklist asked for. Called on every render.
    pub(super) fn sync_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(step) = self.onboarding.resume {
            let back = self.page == crate::pages::Page::Collection
                && !self.palette_open()
                && !crate::console::active()
                && self.dialog.is_none()
                && !self.shots.is_open();
            if back {
                self.onboarding.resume = None;
                self.open_onboarding_at(Some(step), cx);
            }
        }
        let Some((step, action)) = self.onboarding.pending.take() else { return };
        if matches!(action, Action::Page(_) | Action::Palette | Action::Console | Action::Appearance) {
            self.onboarding.resume = Some(step);
        }
        match action {
            Action::Page(page) => self.navigate(page, window, cx),
            Action::Palette => self.open_palette(window, cx),
            Action::Console => self.set_console(true, window, cx),
            Action::Appearance => {
                self.mark_onboarding(step);
                self.open_settings_section(crate::pages::settings::Section::Appearance, window, cx);
            }
            Action::Done(step) => self.mark_onboarding(step),
            Action::Hide => {
                self.onboarding.state.hidden = true;
                self.save_onboarding();
            }
        }
        cx.notify();
    }

    /// The user used a feature of the checklist (once is enough).
    pub(super) fn mark_onboarding(&mut self, step: Step) {
        if self.onboarding.state.done.insert(step.id().to_owned()) {
            self.save_onboarding();
        }
    }

    /// The bulb next to the View button, until "Don't show again".
    pub(super) fn onboarding_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        (!self.onboarding.state.hidden)
            .then(|| tool_button("btn-onboarding", Icon::Lightbulb, false).on_click(cx.listener(|this, _, _, cx| this.open_onboarding(cx))))
    }
}
