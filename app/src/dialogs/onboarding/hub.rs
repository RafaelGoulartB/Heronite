//! The first view: the three cards (your games, your look, while you play)
//! and the tips to discover later.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{presets, Corners, StartView};
use heronite_i18n::{tr, trn};

use super::art::{self, Art};
use super::look::{accent_dot, accent_name, is_preset, preset_name};
use super::steps::{Action, Step, PLAY, TIPS};
use super::{done_label, step_icon, OnboardingDialog, View};
use crate::icons::Icon;
use crate::pages::settings::Section;
use crate::pages::Page;
use crate::theme;
use crate::ui::controls::{button_sm, toggle, ButtonKind};
use crate::ui::icon;

/// Height of the pictures on top of the cards.
const VISUAL_H: f32 = 150.;
/// Width the pictures are drawn at (a card is about this wide plus padding).
const VISUAL_W: f32 = 340.;

/// A card: picture on top, title, then its rows.
fn card(id: &'static str, visual: impl IntoElement, title: SharedString, trailing: Option<Div>, rows: Vec<AnyElement>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .rounded(px(22.))
        .bg(theme::ink_a(0.04))
        .border_1()
        .border_color(theme::ink_a(0.09))
        .child(div().relative().h(px(VISUAL_H)).overflow_hidden().rounded_t(px(22.)).child(visual))
        .child(
            div()
                .flex()
                .flex_col()
                .px(px(18.))
                .pt(px(14.))
                .pb(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pb(px(6.))
                        .child(div().flex_1().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(18.)).child(title))
                        .children(trailing),
                )
                .children(rows),
        )
}

/// A row of a card: icon, label, and what goes on the right.
fn line(ic: Icon, label: impl Into<SharedString>, right: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_h(px(40.))
        .border_t_1()
        .border_color(theme::ink_a(0.05))
        .text_size(px(13.5))
        .child(icon(ic, px(16.), theme::text_muted()))
        .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(label.into()))
        .child(div().flex_none().flex().items_center().child(right))
}

fn dim(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.5)).text_color(theme::text_dim()).child(text.into())
}

/// The picture of a card, cut to its height (wider boards show their top).
fn framed(picture: Div) -> Div {
    div().absolute().top(px(10.)).left_0().right_0().flex().justify_center().child(picture)
}

fn beta() -> Div {
    div()
        .px(px(7.))
        .py(px(1.))
        .rounded_full()
        .bg(theme::accent_alpha(0.16))
        .text_size(px(10.))
        .text_color(theme::accent())
        .child(tr!("Beta").to_uppercase())
}

impl OnboardingDialog {
    pub(super) fn render_hub(&self, art: &Art, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(self.games_card(art, cx))
                    .child(self.look_card(art, cx))
                    .when(PLAY.iter().any(|s| s.available()), |d| d.child(self.play_card(art, cx))),
            )
            .child(self.render_tips(cx))
    }

    fn games_card(&self, art: &Art, cx: &mut Context<Self>) -> Stateful<Div> {
        let f = &self.facts;
        let source = |step: Step, hint: &'static str| {
            let right = match f.count_line(step) {
                Some(n) => done_label(n).into_any_element(),
                None => dim(hint).into_any_element(),
            };
            line(step.icon(), step.title(), right).into_any_element()
        };
        let total = f.games();
        let launchers = heronite_platform::supported(heronite_platform::Feature::LauncherImports).then(|| self.launchers_line(cx));
        let add_hint = if cfg!(windows) { tr!("Windows, web") } else { tr!("Linux, Windows, web") };
        let visual = div()
            .absolute()
            .inset_0()
            .bg(linear_gradient(180., linear_color_stop(theme::black_a(0.), 0.), linear_color_stop(theme::accent_alpha(0.12), 1.)))
            .child(art::fan(art, VISUAL_H));
        card(
            "onb-card-games",
            visual,
            tr!("Your games").into(),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(total > 0, |d| d.child(done_label(trn!(total, "{n} game", "{n} games"))))
                    .child(icon(Icon::ChevronRight, px(16.), theme::text_dim())),
            ),
            [
                Step::Steam.available().then(|| source(Step::Steam, tr!("open Steam"))),
                Step::Stores.available().then(|| source(Step::Stores, tr!("log in"))),
                Step::Emulators.available().then(|| source(Step::Emulators, tr!("set up"))),
                launchers,
                Some(line(Icon::Add, tr!("Add a game"), dim(add_hint)).into_any_element()),
            ]
            .into_iter()
            .flatten()
            .collect(),
        )
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.06)).border_color(theme::ink_a(0.16)))
        .on_click(cx.listener(|this, _, _, cx| this.open(View::Games, cx)))
    }

    /// Heroic, Lutris, Bottles and Faugus: the games brought in from them,
    /// and a click to Settings → Other launchers.
    fn launchers_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let n: usize = crate::services::launcher_imports(cx).read(cx).in_collection.values().sum();
        let right = if n > 0 {
            done_label(trn!(n, "{n} game", "{n} games")).into_any_element()
        } else {
            dim(tr!("Heroic, Lutris and more")).into_any_element()
        };
        line(Icon::CallMerge, tr!("Other launchers"), right)
            .id("onb-line-launchers")
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                // Not the card's click (which opens "Your games").
                cx.stop_propagation();
                this.go(Step::Stores, Action::Settings(Section::Launchers), cx);
            }))
            .into_any_element()
    }

    fn look_card(&self, art: &Art, cx: &mut Context<Self>) -> Stateful<Div> {
        let a = theme::appearance();
        let profile: SharedString =
            presets().iter().find(|p| is_preset(a, p.look)).map(|p| preset_name(p.id)).unwrap_or(tr!("Custom")).into();
        let corners = match a.cover_corners {
            Corners::Square => tr!("Square"),
            Corners::Soft => tr!("Soft"),
            Corners::Round => tr!("Round"),
        };
        let start = match a.start_view {
            StartView::Last => tr!("Last used"),
            StartView::Focus => tr!("Focus"),
            StartView::Grid => tr!("Grid"),
        };
        card(
            "onb-card-look",
            framed(art::collection_preview(art, VISUAL_W, self.preview_grid())),
            tr!("Your look").into(),
            Some(div().child(icon(Icon::ChevronRight, px(16.), theme::text_dim()))),
            vec![
                line(Icon::ViewSidebar, tr!("Open the Collection in"), dim(start)).into_any_element(),
                line(Icon::Palette, tr!("Profile"), dim(profile)).into_any_element(),
                line(
                    Icon::Label,
                    tr!("Accent colour"),
                    div().flex().items_center().gap(px(6.)).child(accent_dot(a.accent, 12.)).child(dim(accent_name(a.accent))),
                )
                .into_any_element(),
                line(Icon::GridView, tr!("Corners of the covers"), dim(corners)).into_any_element(),
            ],
        )
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.06)).border_color(theme::ink_a(0.16)))
        .on_click(cx.listener(|this, _, _, cx| this.open(View::Look, cx)))
    }

    /// A row of the third card that a click switches.
    fn switch_line(&self, step: Step, label: impl Into<SharedString>, extra: Option<Div>, cx: &mut Context<Self>) -> AnyElement {
        let on = self.facts.is_done(step);
        line(step.icon(), label, toggle(SharedString::from(format!("onb-sw-{}", step.id())), on, false))
            .when_some(extra, |d, e| d.child(e))
            .id(SharedString::from(format!("onb-line-{}", step.id())))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.switch(step, !on, cx)))
            .into_any_element()
    }

    fn play_card(&self, art: &Art, cx: &mut Context<Self>) -> Stateful<Div> {
        let f = &self.facts;
        let left = f.overlay.side == heronite_data::overlay_prefs::Side::Left;
        let overlay = if f.overlay_ok {
            self.switch_line(Step::Overlay, Step::Overlay.title(), None, cx)
        } else {
            line(Step::Overlay.icon(), Step::Overlay.title(), dim(tr!("Not on this desktop"))).into_any_element()
        };
        let saves = match f.ludusavi {
            Some(false) if !f.saves => line(
                Step::Saves.icon(),
                Step::Saves.title(),
                button_sm("onb-saves-setup", ButtonKind::Ghost, tr!("Set up"))
                    .on_click(cx.listener(|this, _, _, cx| this.go(Step::Saves, Action::Settings(Section::Saves), cx))),
            )
            .into_any_element(),
            _ => self.switch_line(Step::Saves, Step::Saves.title(), None, cx),
        };
        let proton = match &f.proton {
            Some(name) => line(Step::Proton.icon(), tr!("Proton for Windows games"), done_label(short_proton(name))).into_any_element(),
            None => line(
                Step::Proton.icon(),
                tr!("Proton for Windows games"),
                button_sm("onb-proton", ButtonKind::Glass, tr!("Download"))
                    .on_click(cx.listener(|this, _, _, cx| this.go(Step::Proton, Action::Page(Page::WineManager), cx))),
            )
            .into_any_element(),
        };
        card(
            "onb-card-play",
            framed(art::overlay_preview(art, VISUAL_W, left)),
            tr!("While you play").into(),
            None,
            [
                (Step::Overlay, overlay),
                (Step::Screenshots, self.switch_line(Step::Screenshots, tr!("Screenshots with {key}", key = f.shot_key), None, cx)),
                (Step::Saves, saves),
                (Step::Proton, proton),
                (Step::Tray, self.switch_line(Step::Tray, tr!("Keep in the tray when closed"), None, cx)),
            ]
            .into_iter()
            .filter(|(step, _)| step.available())
            .map(|(_, row)| row)
            .collect(),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(18.))
                .pb(px(14.))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme::text_dim())
                .when(f.overlay_ok, |d| d.child(beta()).child(tr!("The overlay: Guide opens it, Guide + Y marks a moment")))
                .when(!f.overlay_ok, |d| d.child(f.overlay_why.clone())),
        )
    }

    fn render_tips(&self, cx: &mut Context<Self>) -> Div {
        let tip = |step: Step| {
            let done = self.facts.is_done(step);
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(8.))
                .p(px(14.))
                .rounded(px(16.))
                .bg(theme::ink_a(0.035))
                .border_1()
                .border_color(theme::ink_a(0.08))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(step_icon(step, done, 30.))
                        .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).child(step.title())),
                )
                .child(div().flex_1().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_muted()).child(step.description()))
                .when(!step.how().is_empty(), |d| d.child(dim(step.how())))
                .child(div().flex().child(if done {
                    done_label(tr!("Done")).into_any_element()
                } else {
                    button_sm(SharedString::from(format!("onb-tip-{}", step.id())), ButtonKind::Glass, step.tip_label())
                        .on_click(cx.listener(move |this, _, _, cx| this.go(step, step.action(), cx)))
                        .into_any_element()
                }))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .text_color(theme::text_dim())
                    .child(icon(Icon::Lightbulb, px(14.), theme::text_dim()))
                    .child(tr!("Discover").to_uppercase()),
            )
            .child(div().flex().gap(px(10.)).children(TIPS.map(tip)))
    }
}

/// "Proton-CachyOS-latest" → "Proton-CachyOS", to fit the row.
fn short_proton(name: &str) -> String {
    name.strip_suffix("-latest").unwrap_or(name).to_owned()
}
