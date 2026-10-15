//! "Getting started": a checklist of what Heronite does, with the steps on
//! the left and, on the right, a looping preview of the chosen one, what it
//! is for and a way to do it now (a button that leads there, or a switch).
//! It opens by itself once per setup and from the bulb in the header
//! (`collection::onboarding`), which "Don't show again" removes.

mod scenes;
pub mod steps;

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{art, ArtContext};
use heronite_emulation::ludusavi::{detect_ludusavi, SystemRunner};
use heronite_emulation::settings_ext::{set_ludusavi_settings, LudusaviSettingsPatch};
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_launch::SystemProbe;
use serde_json::Value;

use self::scenes::Assets;
use self::steps::{group_title, Facts, Kind, STEPS};
pub use self::steps::{Action, Step};
use super::art_images::DialogImages;
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, close_button, kicker, toggle, ButtonKind};
use crate::ui::{glass, icon};

const CARD_W: f32 = 1100.;
const CARD_H: f32 = 720.;
const LIST_W: f32 = 390.;
/// The preview is 1.15 times the scenes' 520x320 board.
const SCENE_K: f32 = 1.15;
/// Games whose covers the previews show.
const COVERS: usize = 12;

struct Game {
    key: String,
    title: SharedString,
    cover: Vec<ImageRef>,
    stage: Vec<ImageRef>,
}

pub struct OnboardingDialog {
    ctx: DialogCtx,
    facts: Facts,
    selected: usize,
    games: Vec<Game>,
    images: Entity<DialogImages>,
    error: Option<String>,
    on_action: Rc<dyn Fn(Step, Action, &mut App)>,
}

impl EventEmitter<DialogEvent> for OnboardingDialog {}

impl OnboardingDialog {
    /// `done`: the steps already done here (`onboarding.json`). `on_action`
    /// runs what needs the Collection (pages, palette, console mode, saving
    /// the checklist's own state), for the step on show.
    pub fn new(
        ctx: DialogCtx,
        done: BTreeSet<String>,
        on_action: impl Fn(Step, Action, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let facts = Facts::read(&ctx.lib, &ctx.data_paths, done);
        let selected = STEPS.iter().position(|s| !facts.is_done(*s)).unwrap_or(0);
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let games = pick_games(&ctx);
        // Whether Ludusavi is installed decides if backups can be turned on.
        let preferred = ctx.lib.settings.ludusavi.binary_path.clone();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    detect_ludusavi(&SystemRunner::default(), &SystemProbe, Some(preferred.as_str()).filter(|p| !p.is_empty())).is_some()
                })
                .await;
            this.update(cx, |this, cx| {
                this.facts.ludusavi = Some(found);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self { ctx, facts, selected, games, images, error: None, on_action: Rc::new(on_action) }
    }

    /// Shows `step` (`HERONITE_DEBUG_DIALOG=onboarding:<id>`).
    pub fn show(&mut self, step: Step, cx: &mut Context<Self>) {
        if let Some(ix) = STEPS.iter().position(|s| *s == step) {
            self.select(ix, cx);
        }
    }

    fn step(&self) -> Step {
        STEPS[self.selected]
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = ix;
        self.error = None;
        cx.notify();
    }

    fn go(&mut self, action: Action, cx: &mut Context<Self>) {
        (self.on_action)(self.step(), action, cx);
        match action {
            Action::Done(step) => {
                self.facts.done.insert(step.id().to_owned());
                self.select((self.selected + 1) % STEPS.len(), cx);
            }
            _ => cx.emit(DialogEvent::Close),
        }
    }

    /// Turns a step's feature on or off right here.
    fn switch(&mut self, step: Step, on: bool, cx: &mut Context<Self>) {
        let paths = &self.ctx.data_paths;
        let res = match step {
            Step::Screenshots => {
                let s = &self.ctx.lib.settings.screenshots;
                heronite_data::settings::save_screenshots(paths, &s.folder, s.cache_limit_mb.get(), on, &s.capture_accelerator)
                    .map(|_| self.facts.screenshots = on)
            }
            Step::Saves => set_ludusavi_settings(paths, &LudusaviSettingsPatch { enabled: Some(on), ..Default::default() })
                .map(|_| self.facts.saves = on),
            Step::Tray => {
                let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
                heronite_data::game_config::set_global_setting(paths, &home, "exitToTray", Value::Bool(on)).map(|_| self.facts.tray = on)
            }
            _ => Ok(()),
        };
        match res {
            Ok(()) => {
                self.error = None;
                cx.emit(DialogEvent::Changed);
            }
            Err(e) => self.error = Some(tr!("Could not save the setting: {error}", error = format!("{e:#}"))),
        }
        cx.notify();
    }

    fn assets(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Assets {
        let games = &self.games;
        let (covers, stage) = self.images.update(cx, |im, _| {
            im.frame(window);
            let covers = games.iter().map(|g| im.get(&format!("onb-c-{}", g.key), g.cover.clone(), 96., 144.)).collect();
            let stage = games.first().and_then(|g| im.get(&format!("onb-s-{}", g.key), g.stage.clone(), 600., 370.));
            (covers, stage)
        });
        Assets {
            covers,
            stage,
            titles: self.games.iter().map(|g| g.title.clone()).collect(),
            shot_key: self.facts.shot_key.clone().into(),
            steam_games: self.facts.steam_games,
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap(px(2.));
        let mut group = usize::MAX;
        for (ix, step) in STEPS.iter().copied().enumerate() {
            if step.group() != group {
                group = step.group();
                col = col.child(
                    div()
                        .mt(px(if ix == 0 { 6. } else { 14. }))
                        .mb(px(4.))
                        .ml(px(4.))
                        .text_size(px(11.))
                        .text_color(theme::text_dim())
                        .child(SharedString::from(group_title(group).to_uppercase())),
                );
            }
            let done = self.facts.is_done(step);
            let selected = ix == self.selected;
            let state = if done {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(theme::success())
                    .child(icon(Icon::Check, px(13.), theme::success()))
                    .child(if matches!(step.kind(&self.facts), Kind::Switch { .. }) { tr!("On") } else { tr!("Done") })
            } else {
                div().text_color(theme::text_dim()).child(if matches!(step.kind(&self.facts), Kind::Switch { .. }) {
                    tr!("Off")
                } else {
                    tr!("To do")
                })
            };
            col = col.child(
                div()
                    .id(SharedString::from(format!("onb-step-{}", step.id())))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(14.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme::rgb_a(165, 220, 235, 0.1)).border_1().border_color(theme::rgb_a(165, 220, 235, 0.28)))
                    .when(!selected, |d| d.hover(|s| s.bg(theme::white_a(0.05))))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                    .child(step_icon(step, done, 36.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(theme::W_BOLD)
                                    .text_color(if done { theme::text_muted() } else { theme::text_primary() })
                                    .child(step.title()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::text_dim())
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(self.facts.status_line(step)),
                            ),
                    )
                    .child(div().flex_none().text_size(px(12.5)).child(state)),
            );
        }
        col
    }

    /// The button or switch of the chosen step.
    fn render_action(&self, step: Step, cx: &mut Context<Self>) -> Div {
        match step.kind(&self.facts) {
            Kind::Go { label, action } => {
                let done = self.facts.is_done(step) && !matches!(action, Action::Done(_));
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        button("onb-go", if done { ButtonKind::Glass } else { ButtonKind::Primary }, label)
                            .on_click(cx.listener(move |this, _, _, cx| this.go(action, cx))),
                    )
                    .when(self.facts.is_done(step), |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .text_size(px(13.5))
                                .text_color(theme::success())
                                .child(icon(Icon::Check, px(15.), theme::success()))
                                .child(self.facts.status_line(step)),
                        )
                    })
            }
            Kind::Switch { label } => {
                let on = self.facts.is_done(step);
                let missing = step == Step::Saves && self.facts.ludusavi == Some(false) && !on;
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .max_w(px(460.))
                    .child(
                        div()
                            .id("onb-switch")
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .px(px(14.))
                            .py(px(10.))
                            .rounded(px(14.))
                            .bg(theme::white_a(0.04))
                            .border_1()
                            .border_color(theme::white_a(0.09))
                            .when(!missing, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.switch(step, !on, cx))))
                            .child(div().flex_1().text_size(px(14.)).child(label))
                            .child(toggle("onb-switch-toggle", on, missing)),
                    )
                    .when(missing, |d| {
                        d.child(
                            div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::danger_text()).child(tr!(
                                "Ludusavi was not found. Settings \u{2192} Saves shows where to download it; then turn this on."
                            )),
                        )
                    })
            }
        }
    }
}

impl Render for OnboardingDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let assets = self.assets(window, cx);
        let step = self.step();
        let done = self.facts.done_count();
        let total = STEPS.len();
        let last = self.selected + 1 == total;

        let left = div()
            .w(px(LIST_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(30.))
            .pb(px(18.))
            .pl(px(26.))
            .pr(px(14.))
            .border_r_1()
            .border_color(theme::white_a(0.07))
            .child(div().pl(px(4.)).child(kicker(tr!("Getting started"))))
            .child(
                div()
                    .pl(px(4.))
                    .mt(px(8.))
                    .font_family(theme::FONT_HEADING)
                    .text_size(px(24.))
                    .line_height(px(30.))
                    .font_weight(theme::W_BOLD)
                    .child(tr!("Get Heronite ready for you")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .mt(px(16.))
                    .mb(px(4.))
                    .pl(px(4.))
                    .pr(px(10.))
                    .child(div().flex_1().h(px(6.)).rounded_full().bg(theme::white_a(0.08)).child(
                        div().h_full().w(relative(done as f32 / total as f32)).rounded_full().bg(linear_gradient(
                            90.,
                            linear_color_stop(theme::accent(), 0.),
                            linear_color_stop(rgb(0xb7a8ff), 1.),
                        )),
                    ))
                    .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(tr!("{done} of {total}", done, total))),
            )
            .child(div().id("onb-list").flex_1().min_h_0().overflow_y_scroll().pr(px(6.)).child(self.render_list(cx)));

        let right = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(40.))
            .pb(px(26.))
            .px(px(40.))
            .child(scenes::scene(step, SCENE_K, assets))
            .child(div().mt(px(22.)).font_family(theme::FONT_HEADING).text_size(px(24.)).font_weight(theme::W_BOLD).child(step.title()))
            .child(
                div()
                    .mt(px(8.))
                    .mb(px(18.))
                    .max_w(px(600.))
                    .text_size(px(15.))
                    .line_height(px(22.))
                    .text_color(theme::text_muted())
                    .child(step.description()),
            )
            .child(self.render_action(step, cx))
            .children(self.error.clone().map(|e| div().mt(px(8.)).text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.5))
                            .text_color(theme::text_dim())
                            .child(icon(Icon::ChevronRight, px(13.), theme::text_dim()))
                            .child(step.how()),
                    )
                    .child(
                        button("onb-hide", ButtonKind::Ghost, tr!("Don't show again"))
                            .on_click(cx.listener(|this, _, _, cx| this.go(Action::Hide, cx))),
                    )
                    .child(button("onb-next", ButtonKind::Glass, if last { tr!("Finish") } else { tr!("Next") }).on_click(cx.listener(
                        move |this, _, _, cx| {
                            if last {
                                cx.emit(DialogEvent::Close);
                            } else {
                                this.select(this.selected + 1, cx);
                            }
                        },
                    ))),
            );

        let card = glass::glass_panel(theme::R_SHEET)
            .id("onboarding")
            .occlude()
            .relative()
            .flex()
            .w(px(CARD_W))
            .h(px(CARD_H))
            .max_w(relative(0.96))
            .max_h(relative(0.95))
            .overflow_hidden()
            .text_color(theme::text_primary())
            .font_family(theme::FONT_BODY)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(left)
            .child(right)
            .child(
                div()
                    .absolute()
                    .top(px(18.))
                    .right(px(18.))
                    .child(close_button("onboarding-close").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))),
            );
        deferred(
            div()
                .id("onboarding-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::glass_backdrop_veil())
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
                .child(card),
        )
        .with_priority(30)
    }
}

/// The step's icon on its group's tint, with a check once done.
fn step_icon(step: Step, done: bool, size: f32) -> Div {
    let (fg, bg) = match step.group() {
        0 => (theme::accent(), theme::rgb_a(165, 220, 235, 0.13)),
        1 => (theme::success(), theme::rgb_a(95, 211, 155, 0.13)),
        _ => (rgb(0xb7a8ff), theme::rgb_a(183, 168, 255, 0.14)),
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .bg(bg)
        .child(icon(step.icon(), px(size * 0.53), fg))
        .when(done, |d| {
            d.child(
                div()
                    .absolute()
                    .right(px(-4.))
                    .bottom(px(-4.))
                    .size(px(17.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::success())
                    .border_2()
                    .border_color(rgb(0x121821))
                    .child(icon(Icon::Check, px(11.), rgb(0x06150d))),
            )
        })
}

/// The most played games with art, for the previews.
fn pick_games(ctx: &DialogCtx) -> Vec<Game> {
    let settings = &ctx.lib.settings;
    let shots = (!settings.screenshots.folder.is_empty()).then(|| std::path::PathBuf::from(&settings.screenshots.folder));
    let art_ctx = ArtContext::new(&ctx.data_paths, shots.as_deref());
    let mut games: Vec<_> = ctx.lib.games.iter().filter(|g| !g.title.is_empty()).collect();
    games.sort_by(|a, b| b.playtime_minutes.cmp(&a.playtime_minutes).then(b.installed.cmp(&a.installed)));
    games
        .into_iter()
        .filter_map(|g| {
            let cover = crate::images::to_chain(&art::card_cover(&art_ctx, g));
            (!cover.is_empty()).then(|| Game {
                key: g.key.clone(),
                title: g.title.clone().into(),
                stage: crate::images::to_chain(&art::stage_art(&art_ctx, g)),
                cover,
            })
        })
        .take(COVERS)
        .collect()
}
