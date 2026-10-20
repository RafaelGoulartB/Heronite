//! "Getting started": three cards (your games, your look, while you play)
//! with what is left to set up, and tips to discover later. The first two
//! cards open inside the dialog (the sources, the look picker with a live
//! preview); the third switches things on in place. It opens by itself once
//! per setup and from the bulb in the header (`collection::onboarding`),
//! which "Don't show again" removes.

mod art;
mod games;
mod hub;
mod look;
pub mod steps;

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{Appearance, StartView};
use heronite_data::{art as game_art, ArtContext};
use heronite_emulation::ludusavi::{detect_ludusavi, SystemRunner};
use heronite_emulation::settings_ext::{set_ludusavi_settings, LudusaviSettingsPatch};
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_launch::SystemProbe;
use serde_json::Value;

use self::art::Art;
use self::steps::Facts;
pub use self::steps::{Action, Step};
use super::art_images::DialogImages;
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, close_button, kicker, ButtonKind};
use crate::ui::{glass, icon};

const CARD_W: f32 = 1180.;
/// Games whose covers the pictures show.
const COVERS: usize = 9;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Hub,
    Games,
    Look,
}

struct Game {
    key: String,
    title: SharedString,
    cover: Vec<ImageRef>,
    stage: Vec<ImageRef>,
}

pub struct OnboardingDialog {
    ctx: DialogCtx,
    facts: Facts,
    view: View,
    games: Vec<Game>,
    images: Entity<DialogImages>,
    error: Option<String>,
    on_action: Rc<dyn Fn(Step, Action, &mut App)>,
}

impl EventEmitter<DialogEvent> for OnboardingDialog {}

impl OnboardingDialog {
    /// `done`: the steps already done here (`onboarding.json`). `on_action`
    /// runs what needs the Collection (pages, palette, console mode, saving
    /// the dialog's own state).
    pub fn new(
        ctx: DialogCtx,
        done: BTreeSet<String>,
        on_action: impl Fn(Step, Action, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let facts = Facts::read(&ctx.lib, &ctx.data_paths, &ctx.app, done);
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        // The other launchers found on this PC, offered in "Your games".
        let launchers = crate::services::launcher_imports(cx);
        cx.observe(&launchers, |_, _, cx| cx.notify()).detach();
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
        Self { ctx, facts, view: View::Hub, games, images, error: None, on_action: Rc::new(on_action) }
    }

    /// Opens the part of the dialog with `step` (`HERONITE_DEBUG_DIALOG=onboarding:<id>`,
    /// and coming back from where a step led).
    pub fn show(&mut self, step: Step, cx: &mut Context<Self>) {
        self.open(
            match step {
                Step::Steam | Step::Stores | Step::Emulators => View::Games,
                Step::Appearance => View::Look,
                _ => View::Hub,
            },
            cx,
        );
    }

    fn open(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        self.error = None;
        cx.notify();
    }

    fn go(&mut self, step: Step, action: Action, cx: &mut Context<Self>) {
        (self.on_action)(step, action, cx);
        match action {
            Action::Done(step) => {
                self.facts.done.insert(step.id().to_owned());
                cx.notify();
            }
            Action::LookChanged => cx.notify(),
            _ => cx.emit(DialogEvent::Close),
        }
    }

    /// Turns a step's feature on or off right here.
    fn switch(&mut self, step: Step, on: bool, cx: &mut Context<Self>) {
        let paths = &self.ctx.data_paths;
        let res = match step {
            Step::Overlay => {
                self.facts.overlay.enabled = on;
                let prefs = self.facts.overlay.clone();
                let app = self.ctx.app.clone();
                let saved = prefs.clone();
                cx.background_spawn(async move {
                    if let Err(e) = saved.save(&app) {
                        log::warn!("saving the overlay options: {e:#}");
                    }
                })
                .detach();
                if let Some(service) = crate::overlay::get(cx) {
                    service.update(cx, |s, cx| s.set_prefs(prefs, cx));
                }
                Ok(())
            }
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
                if step != Step::Overlay {
                    cx.emit(DialogEvent::Changed);
                }
            }
            Err(e) => self.error = Some(tr!("Could not save the setting: {error}", error = format!("{e:#}"))),
        }
        cx.notify();
    }

    /// Changes the appearance at once (the Collection behind follows) and
    /// saves it off the UI thread, like Settings → Appearance.
    fn set_look(&mut self, edit: impl FnOnce(&mut Appearance), cx: &mut Context<Self>) {
        let mut look = theme::appearance();
        edit(&mut look);
        theme::set_appearance(look);
        let look = theme::appearance();
        // The Collection switches to the chosen view now (`apply_start_view`).
        match look.start_view {
            StartView::Last => {}
            StartView::Focus => self.facts.grid_view = false,
            StartView::Grid => self.facts.grid_view = true,
        }
        let app = self.ctx.app.clone();
        cx.background_spawn(async move {
            if let Err(err) = look.save(&app) {
                log::warn!("saving appearance.json: {err:#}");
            }
        })
        .detach();
        self.facts.done.insert(Step::Appearance.id().to_owned());
        self.go(Step::Appearance, Action::LookChanged, cx);
    }

    /// The view the preview of the Collection draws: the one it opens in, or
    /// the one on screen with "Last used".
    fn preview_grid(&self) -> bool {
        match theme::appearance().start_view {
            StartView::Last => self.facts.grid_view,
            StartView::Focus => false,
            StartView::Grid => true,
        }
    }

    fn art(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Art {
        let games = &self.games;
        let (covers, stage) = self.images.update(cx, |im, _| {
            im.frame(window);
            let covers = games.iter().map(|g| im.get(&format!("onb-c-{}", g.key), g.cover.clone(), 128., 192.)).collect();
            let stage = games.first().and_then(|g| im.get(&format!("onb-s-{}", g.key), g.stage.clone(), 640., 400.));
            (covers, stage)
        });
        Art { covers, stage, titles: self.games.iter().map(|g| g.title.clone()).collect() }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> Div {
        let heading = |text: SharedString| {
            div().font_family(theme::font_heading()).text_size(px(28.)).line_height(px(34.)).font_weight(theme::W_BOLD).child(text)
        };
        match self.view {
            View::Hub => {
                let (done, total) = (self.facts.done_count(), self.facts.counted().len());
                let lead = if self.facts.games() > 0 {
                    tr!("Your games are in. A few more things and you are set.")
                } else {
                    tr!("Three things, in any order. Start with your games.")
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(24.))
                    .pr(px(52.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .child(kicker(tr!("Getting started")))
                            .child(heading(tr!("Get Heronite ready for you").into()))
                            .child(div().text_size(px(14.5)).text_color(theme::text_muted()).child(lead)),
                    )
                    .child(art::ring(done as f32 / total.max(1) as f32, 64., format!("{done}/{total}")))
            }
            View::Games | View::Look => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .id("onb-back")
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .w_auto()
                        .text_size(px(13.5))
                        .text_color(theme::text_muted())
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::text_primary()))
                        .on_click(cx.listener(|this, _, _, cx| this.open(View::Hub, cx)))
                        .child(icon(Icon::ChevronLeft, px(16.), theme::text_muted()))
                        .child(tr!("Getting started")),
                )
                .child(heading(if self.view == View::Games { tr!("Where do your games come from?") } else { tr!("Make it yours") }.into())),
        }
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .pt(px(18.))
            .child(
                button("onb-hide", ButtonKind::Ghost, tr!("Don't show again"))
                    .on_click(cx.listener(|this, _, _, cx| this.go(Step::Palette, Action::Hide, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .mr(px(6.))
                    .text_size(px(12.5))
                    .text_color(theme::text_dim())
                    .child(icon(Icon::Lightbulb, px(14.), theme::text_dim()))
                    .child(tr!("It stays under the bulb at the top")),
            )
            .child(button("onb-close", ButtonKind::Glass, tr!("Close")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
    }
}

impl Render for OnboardingDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let art = self.art(window, cx);
        let body = match self.view {
            View::Hub => self.render_hub(&art, cx),
            View::Games => self.render_games(&art, cx),
            View::Look => self.render_look(&art, cx),
        };
        let card = glass::glass_panel(theme::R_SHEET)
            .id("onboarding")
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .w(px(CARD_W))
            .max_w(relative(0.96))
            .max_h(relative(0.95))
            .px(px(38.))
            .pt(px(32.))
            .pb(px(24.))
            .text_color(theme::text_primary())
            .font_family(theme::font_body())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.render_header(cx))
            .child(div().id("onb-body").flex_1().min_h_0().mt(px(22.)).overflow_y_scroll().child(body))
            .children(self.error.clone().map(|e| div().mt(px(10.)).text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
            .child(self.render_footer(cx))
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

/// A step's icon on a tint, with a check once done.
fn step_icon(step: Step, done: bool, size: f32) -> Div {
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.32))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::accent_alpha(0.13))
        .child(icon(step.icon(), px(size * 0.53), theme::accent()))
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

/// Green check + text.
fn done_label(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .text_size(px(12.5))
        .text_color(theme::success())
        .child(icon(Icon::Check, px(14.), theme::success()))
        .child(text.into())
}

/// The most played games with art, for the pictures.
fn pick_games(ctx: &DialogCtx) -> Vec<Game> {
    let settings = &ctx.lib.settings;
    let shots = (!settings.screenshots.folder.is_empty()).then(|| std::path::PathBuf::from(&settings.screenshots.folder));
    let art_ctx = ArtContext::new(&ctx.data_paths, shots.as_deref());
    let mut games: Vec<_> = ctx.lib.games.iter().filter(|g| !g.title.is_empty()).collect();
    games.sort_by(|a, b| b.playtime_minutes.cmp(&a.playtime_minutes).then(b.installed.cmp(&a.installed)));
    games
        .into_iter()
        .filter_map(|g| {
            let cover = crate::images::to_chain(&game_art::card_cover(&art_ctx, g));
            (!cover.is_empty()).then(|| Game {
                key: g.key.clone(),
                title: g.title.clone().into(),
                stage: crate::images::to_chain(&game_art::stage_art(&art_ctx, g)),
                cover,
            })
        })
        .take(COVERS)
        .collect()
}
