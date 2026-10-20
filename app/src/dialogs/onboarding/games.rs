//! "Where do your games come from?": the sources, each with what it brought
//! in or a way to set it up, over a strip of what the Collection holds.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::art::{self, Art};
use super::steps::{Action, Step, GAMES};
use super::{done_label, step_icon, OnboardingDialog};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::icon;

/// Covers in the strip under the sources.
const STRIP: usize = 9;

fn tile(head: Div, title: &'static str, sub: Div, description: &'static str, foot: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .w(relative(0.49))
        .min_h(px(168.))
        .p(px(18.))
        .rounded(px(20.))
        .bg(theme::white_a(0.045))
        .border_1()
        .border_color(theme::white_a(0.09))
        .child(
            div().flex().items_center().gap(px(12.)).child(head).child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(16.5)).child(title))
                    .child(sub),
            ),
        )
        .child(div().flex_1().text_size(px(13.5)).line_height(px(19.)).text_color(theme::text_muted()).child(description))
        .child(foot)
}

impl OnboardingDialog {
    /// The launchers on this PC with games not brought in yet (Heroic,
    /// Lutris, Faugus): a button each turns its import on.
    fn other_launchers(&self, cx: &mut Context<Self>) -> Option<Div> {
        let svc = crate::services::launcher_imports(cx);
        let offers = svc.read(cx).offers();
        if offers.is_empty() {
            return None;
        }
        let buttons = offers.into_iter().map(|o| {
            let svc = svc.clone();
            let label = format!("{} \u{b7} {}", o.launcher.name(), trn!(o.games, "{n} game", "{n} games"));
            button_sm(SharedString::from(format!("onb-launcher-{}", o.launcher.id())), ButtonKind::Glass, label)
                .on_click(move |_, _, cx| svc.update(cx, |s, cx| s.set_enabled(o.launcher, true, cx)))
        });
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(18.))
                .py(px(12.))
                .rounded(px(18.))
                .bg(theme::white_a(0.035))
                .border_1()
                .border_color(theme::white_a(0.08))
                .child(icon(Icon::CallMerge, px(20.), theme::accent()))
                .child(div().flex_1().text_size(px(13.5)).text_color(theme::text_muted()).child(tr!("Games in other launchers on this PC")))
                .children(buttons),
        )
    }

    pub(super) fn render_games(&self, art: &Art, cx: &mut Context<Self>) -> Div {
        let others = self.other_launchers(cx);
        let f = &self.facts;
        let source = |step: Step, label: &'static str, primary: bool| {
            let done = f.is_done(step);
            let sub = match f.count_line(step) {
                Some(n) => done_label(n),
                None => div().text_size(px(12.5)).text_color(theme::text_dim()).child(match step {
                    Step::Steam => tr!("No login, no key"),
                    Step::Stores => tr!("Epic Games, GOG and Amazon Games"),
                    _ => tr!("RetroArch, PCSX2, Dolphin and more"),
                }),
            };
            let kind = if primary && !done { ButtonKind::Primary } else { ButtonKind::Glass };
            let foot = div().flex().justify_end().child(
                button_sm(SharedString::from(format!("onb-src-{}", step.id())), kind, label)
                    .on_click(cx.listener(move |this, _, _, cx| this.go(step, step.action(), cx))),
            );
            tile(step_icon(step, done, 38.), step.title(), sub, step.description(), foot)
        };
        let [steam, stores, emulators] = GAMES;
        let add = tile(
            div().size(px(38.)).rounded(px(12.)).flex().items_center().justify_center().bg(theme::white_a(0.08)).child(icon(
                Icon::Add,
                px(20.),
                theme::text_primary(),
            )),
            tr!("Add a game"),
            div().text_size(px(12.5)).text_color(theme::text_dim()).child(tr!("Linux, Windows or browser")),
            tr!("A Linux program, a Windows .exe (run through Proton) or a browser game."),
            div().flex().justify_end().child(
                button_sm("onb-src-add", ButtonKind::Glass, tr!("Add a game"))
                    .on_click(cx.listener(|this, _, _, cx| this.go(Step::Stores, Action::AddGame, cx))),
            ),
        );
        let total = f.games();
        let strip = if art.has_games() {
            div().flex().gap(px(10.)).children((0..STRIP.min(art.titles.len())).map(|i| art::cover(art, i, 56., 84., 7.)))
        } else {
            div().flex().gap(px(10.)).children((0..STRIP).map(|_| art::ghost(56., 84.)))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div().text_size(px(14.5)).text_color(theme::text_muted()).child(tr!(
                    "Connect what you use. Steam needs no login: it only has to be open. You can add more later from \u{2630}."
                )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_y(px(14.))
                    .child(source(steam, tr!("Open the Steam page"), true))
                    .child(source(stores, tr!("Log in to a store"), false))
                    .child(source(emulators, tr!("Set up emulators"), false))
                    .child(add),
            )
            .children(others)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(22.))
                    .px(px(22.))
                    .py(px(14.))
                    .rounded(px(20.))
                    .bg(theme::black_a(0.25))
                    .border_1()
                    .border_color(theme::white_a(0.06))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .w(px(150.))
                            .child(
                                div()
                                    .font_family(theme::font_heading())
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(30.))
                                    .child(total.to_string()),
                            )
                            .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(if total > 0 {
                                trn!(total, "game in your Collection", "games in your Collection")
                            } else {
                                tr!("games so far").into()
                            })),
                    )
                    .child(div().flex_1().min_w_0().overflow_hidden().child(strip))
                    .child(div().w(px(230.)).flex_none().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_dim()).child(
                        if total > 0 {
                            tr!("Covers, details and play time arrive by themselves in the background.")
                        } else {
                            tr!("No rush: the empty Collection has the same buttons for later.")
                        },
                    )),
            )
    }
}
