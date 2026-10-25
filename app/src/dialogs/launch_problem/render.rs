//! The desktop look of the dialog: the main finding as a card with its
//! evidence and fixes, the other findings as rows under it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::diagnose::{AntiCheatStatus, Finding, Fix, Problem};

use super::{At, LaunchProblemDialog, State};
use crate::diagnosis::{text, was_tried};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, caption, card, help_text, ButtonKind};
use crate::ui::dialog::{dialog_scrolled, DialogProps};
use crate::ui::icon;

/// Problems no setting fixes: drawn in the danger colour.
fn blocked(f: &Finding) -> bool {
    matches!(f.problem, Problem::AntiCheat { status: AntiCheatStatus::Broken | AntiCheatStatus::Denied, .. })
}

/// The evidence comes from the log (else from the game's data or the computer).
fn from_log(f: &Finding) -> bool {
    match f.problem {
        Problem::AntiCheat { .. } | Problem::Dx12WithoutVkd3d => false,
        Problem::MaxMapCount { .. } => f.sure,
        _ => true,
    }
}

fn tint(c: Rgba, a: f32) -> Rgba {
    Rgba { a, ..c }
}

/// Monospace lines in a recessed box.
fn code_block(lines: impl IntoIterator<Item = String>, wrap: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(10.))
        .bg(theme::recess_a(0.35))
        .font_family(theme::FONT_MONO)
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(theme::code_text())
        .children(lines.into_iter().map(move |l| {
            let line = div().min_w_0().child(l);
            if wrap {
                line
            } else {
                line.truncate()
            }
        }))
}

impl LaunchProblemDialog {
    pub(super) fn render_desktop(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let report = self.report.clone();
        let mut body = div().flex().flex_col().gap(px(14.)).child(help_text(text::how_it_ended(&report)));
        match report.findings.split_first() {
            Some((main, rest)) => {
                body = body.child(self.main_card(main, cx));
                if !rest.is_empty() {
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .child(caption(tr!("Also found")).mt(px(2.)))
                            .child(card(rest.iter().enumerate().map(|(i, f)| self.other_row(i + 1, f, cx).into_any_element()))),
                    );
                }
            }
            None => body = body.child(self.nothing_found()),
        }
        if self.scroll.bounds().size.height == px(0.) {
            window.request_animation_frame();
        }
        let entity = cx.entity().downgrade();
        dialog_scrolled(
            DialogProps { id: "launch-problem".into(), title: tr!("Launch problem").into(), width: 620., height: None },
            &self.scroll,
            body.into_any_element(),
            Some(self.footer(cx).into_any_element()),
            move |_, cx| {
                entity.update(cx, |this, cx| this.close(cx)).ok();
            },
        )
    }

    fn main_card(&self, f: &Finding, cx: &mut Context<Self>) -> Div {
        let (badge_icon, colour) = if blocked(f) { (Icon::Lock, theme::danger()) } else { (Icon::Build, theme::warning()) };
        let tried = f.fixes.first().is_some_and(|fix| was_tried(&self.report.app_name, fix));
        let head = div()
            .flex()
            .items_start()
            .gap(px(14.))
            .child(
                div().flex().flex_none().items_center().justify_center().size(px(38.)).rounded(px(12.)).bg(tint(colour, 0.16)).child(icon(
                    badge_icon,
                    px(20.),
                    theme::readable(colour),
                )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(16.))
                            .line_height(px(21.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child(text::headline(f)),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(19.))
                            .text_color(theme::text_muted())
                            .child(text::explanation(f, &self.report)),
                    ),
            );
        let mut c = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .w_full()
            .px(px(18.))
            .py(px(16.))
            .rounded(px(theme::R_CARD))
            .bg(theme::card_fill())
            .border_1()
            .border_color(if blocked(f) { tint(theme::danger(), 0.45).into() } else { theme::card_border() })
            .child(head);
        if !f.evidence.is_empty() {
            let label = if from_log(f) { tr!("What the log says") } else { tr!("What Heronite found") };
            c = c.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(label))
                    .child(code_block(f.evidence.clone(), false)),
            );
        }
        for (j, fix) in f.fixes.iter().enumerate() {
            if let Fix::Terminal(cmd) = fix {
                c = c.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(text::terminal_note(f)))
                        .child(code_block([cmd.clone()], true))
                        .when(self.copied == Some((0, j)), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(12.5))
                                    .text_color(theme::success())
                                    .child(icon(Icon::Check, px(15.), theme::success()))
                                    .child(tr!("Copied. Paste it in a terminal.")),
                            )
                        }),
                );
            }
        }
        if tried {
            c = c.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(theme::caution_text())
                    .child(icon(Icon::History, px(15.), theme::caution_text()))
                    .child(text::tried_note()),
            );
        }
        c = c.children(self.state_block(0, cx));
        if !f.fixes.is_empty() {
            c = c.child(self.fix_buttons(0, f, tried, cx));
        }
        c
    }

    /// Progress or failure of a fix of finding `i`.
    fn state_block(&self, i: usize, cx: &mut Context<Self>) -> Option<Div> {
        match &self.state {
            State::Working { at, lines, .. } if at.0 == i => {
                let cancellable = matches!(self.fix(*at), Some(Fix::Winetricks(_)));
                let label = self.fix(*at).map(text::working).unwrap_or_default();
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(10.))
                                .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(label))
                                .when(cancellable, |d| {
                                    d.child(
                                        crate::ui::controls::button_sm("lp-cancel", ButtonKind::Ghost, tr!("Cancel"))
                                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                                    )
                                }),
                        )
                        .child(crate::services::widgets::busy_bar("lp-busy", 5.))
                        .when(!lines.is_empty(), |d| d.child(code_block(lines.clone(), false))),
                )
            }
            State::Failed { at, message } if at.0 == i => Some(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .line_height(px(17.))
                    .text_color(theme::danger_text())
                    .child(icon(Icon::Warning, px(15.), theme::danger_text()))
                    .child(div().min_w_0().child(message.clone())),
            ),
            _ => None,
        }
    }

    fn fix_buttons(&self, i: usize, f: &Finding, tried: bool, cx: &mut Context<Self>) -> Div {
        let busy = self.busy();
        let mut row = div().flex().flex_wrap().items_center().gap(px(8.));
        for (j, fix) in f.fixes.iter().enumerate() {
            let at: At = (i, j);
            // A first fix that did not help is no longer the answer.
            let kind = if j == 0 && !tried { ButtonKind::Primary } else { ButtonKind::Glass };
            let label = match fix {
                Fix::Terminal(_) if self.copied == Some(at) => tr!("Copy again").to_string(),
                _ => text::fix_label(fix, &self.report),
            };
            row = row.child(action_button(
                SharedString::from(format!("lp-fix-{i}-{j}")),
                kind,
                label,
                !busy,
                cx.listener(move |this, _, _, cx| this.run_fix(at, cx)),
            ));
        }
        row
    }

    fn other_row(&self, i: usize, f: &Finding, cx: &mut Context<Self>) -> Div {
        let tried = f.fixes.first().is_some_and(|fix| was_tried(&self.report.app_name, fix));
        let mut col = div().flex().flex_col().gap(px(8.)).py(px(12.)).child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(text::headline(f)))
                .child(
                    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(text::explanation(f, &self.report)),
                ),
        );
        col = col.children(self.state_block(i, cx));
        if !f.fixes.is_empty() {
            let busy = self.busy();
            let mut row = div().flex().flex_wrap().gap(px(8.));
            for (j, fix) in f.fixes.iter().enumerate() {
                let at: At = (i, j);
                let label = match fix {
                    Fix::Terminal(_) if self.copied == Some(at) => tr!("Copy again").to_string(),
                    _ => text::fix_label(fix, &self.report),
                };
                row = row.child(
                    crate::ui::controls::button_sm(SharedString::from(format!("lp-fix-{i}-{j}")), ButtonKind::Glass, label)
                        .when(busy, |b| b.opacity(0.45))
                        .when(!busy, |b| b.on_click(cx.listener(move |this, _, _, cx| this.run_fix(at, cx)))),
                );
            }
            col = col.child(row);
        }
        if tried {
            col = col.child(div().text_size(px(12.)).text_color(theme::caution_text()).child(text::tried_note()));
        }
        col
    }

    fn nothing_found(&self) -> Div {
        let lead = if self.report.has_log {
            tr!("The last launch log shows no known problem.")
        } else {
            tr!("This game has no launch log yet: start it once, then look again.")
        };
        crate::ui::controls::card_box().px(px(18.)).py(px(16.)).gap(px(6.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(icon(Icon::Check, px(20.), theme::success()))
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(lead)),
        )
    }

    fn footer(&self, cx: &mut Context<Self>) -> Div {
        let busy = self.busy();
        let left = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .when(self.report.has_log, |d| {
                d.child(crate::ui::controls::icon_button_sm("lp-log", ButtonKind::Ghost, Icon::Description, tr!("View log"), true, {
                    let view_log = self.actions.view_log.clone();
                    move |_, _, cx| view_log(cx)
                }))
            })
            .when(self.report.steam_app_id.is_some() && self.report.kind.windows(), |d| {
                d.child(crate::ui::controls::icon_button_sm(
                    "lp-protondb",
                    ButtonKind::Ghost,
                    Icon::OpenInNew,
                    "ProtonDB",
                    true,
                    cx.listener(|this, _, _, cx| this.protondb(cx)),
                ))
            });
        div().flex().flex_1().items_center().justify_between().gap(px(10.)).child(left).child(
            button("lp-close", ButtonKind::Glass, tr!("Close"))
                .when(busy, |b| b.opacity(0.45))
                .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.close(cx)))),
        )
    }
}
