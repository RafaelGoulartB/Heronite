//! The install while it runs and once it ended: stages, progress, the
//! script's questions, the output, and what to do next.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_installer::{Answer, Question};

use super::render::{disclosure, icon_line, mono_box, spinner};
use super::ScriptInstallDialog;
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::services::script_install::{Run, Status};
use crate::services::widgets::{busy_bar, progress_bar};
use crate::theme;
use crate::ui::controls::{action_button, card_box, ButtonKind};
use crate::ui::fields::{choose_path, meta_text, paragraph};
use crate::ui::icon;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Done,
    Now,
    Failed,
    Later,
}

enum End {
    Running { stopping: bool },
    Done { title: String, notes: Vec<String> },
    Failed { message: String, created_dir: bool, cancelled: bool, step: Option<usize> },
}

/// What the view needs of the run, copied out of the service so the
/// listeners can borrow the context.
struct Snapshot {
    title: String,
    folder: String,
    stages: Vec<String>,
    stage: usize,
    progress: Option<(Option<f32>, String)>,
    log: Vec<String>,
    log_count: usize,
    question: Option<Question>,
    end: End,
    removing: bool,
}

impl Snapshot {
    fn of(run: &Run) -> Self {
        let script = &run.job.script;
        let end = match &run.status {
            Status::Running => End::Running { stopping: run.stopping },
            Status::Done(o) => End::Done { title: o.title.clone(), notes: o.notes.clone() },
            Status::Failed(f) => {
                End::Failed { message: f.message.clone(), created_dir: f.created_dir, cancelled: f.cancelled, step: f.step }
            }
        };
        Self {
            title: run.job.title.clone(),
            folder: run.job.game_dir.display().to_string(),
            stages: run.stages.iter().map(|s| script.preview(s)).collect(),
            stage: run.stage,
            progress: run.progress.clone(),
            log: run.log.iter().cloned().collect(),
            log_count: run.log_count,
            question: run.question.clone(),
            end,
            removing: run.removing,
        }
    }
}

impl ScriptInstallDialog {
    fn stage_list(&self, s: &Snapshot) -> Div {
        let rows = s.stages.iter().enumerate().map(|(i, text)| {
            let mark = match &s.end {
                End::Done { .. } => Mark::Done,
                _ if i < s.stage => Mark::Done,
                End::Failed { .. } if i == s.stage => Mark::Failed,
                End::Running { .. } if i == s.stage => Mark::Now,
                _ => Mark::Later,
            };
            let (ic, color) = match mark {
                Mark::Done => (Icon::Check, theme::success()),
                Mark::Now => (Icon::FaSpinner, theme::accent()),
                Mark::Failed => (Icon::Close, theme::danger_text()),
                Mark::Later => (Icon::ChevronRight, theme::text_dim()),
            };
            let mut row = div().flex().flex_col().gap(px(6.)).py(px(6.)).child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(div().pt(px(2.)).flex_none().child(if mark == Mark::Now {
                        spinner("si-stage-spin", 14., color)
                    } else {
                        icon(ic, px(14.), color).into_any_element()
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .text_color(if mark == Mark::Later { theme::text_dim() } else { theme::text_primary() })
                            .when(mark == Mark::Now, |d| d.font_weight(theme::W_BOLD))
                            .child(text.clone()),
                    ),
            );
            if mark == Mark::Now && s.question.is_none() {
                let bar = match s.progress.as_ref().and_then(|(f, _)| *f) {
                    Some(f) => progress_bar(Some(f * 100.), 4., theme::accent()).into_any_element(),
                    None => busy_bar("si-busy", 4.),
                };
                row = row.child(div().pl(px(24.)).child(bar));
                if let Some((_, text)) = &s.progress {
                    row = row.child(div().pl(px(24.)).child(meta_text(text.clone())));
                }
            }
            row
        });
        card_box().py(px(8.)).children(rows)
    }

    fn question(&self, s: &Snapshot, cx: &mut Context<Self>) -> Option<Div> {
        let q = s.question.clone()?;
        let mut panel = card_box().gap(px(10.)).py(px(14.)).border_color(theme::accent_tint_border());
        match q {
            Question::Menu { description, options, preselect } => {
                let chosen = self.menu_choice.clone().or(preselect).or_else(|| options.first().map(|o| o.0.clone()));
                panel = panel.child(paragraph(if description.is_empty() { tr!("Choose an option").to_string() } else { description }));
                for (i, (value, label)) in options.into_iter().enumerate() {
                    let on = chosen.as_deref() == Some(value.as_str());
                    panel = panel.child(
                        div()
                            .id(SharedString::from(format!("si-opt-{i}")))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(10.))
                            .py(px(7.))
                            .rounded(px(10.))
                            .cursor_pointer()
                            .when(on, |d| d.bg(theme::accent_tint()))
                            .hover(|st| st.bg(theme::menu_item_hover()))
                            .child(icon(
                                if on { Icon::Check } else { Icon::ChevronRight },
                                px(14.),
                                if on { theme::accent() } else { theme::text_dim() },
                            ))
                            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(label))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.menu_choice = Some(value.clone());
                                cx.notify();
                            })),
                    );
                }
                panel = panel.child(div().flex().justify_end().child(action_button(
                    "si-answer",
                    ButtonKind::Primary,
                    tr!("Continue"),
                    chosen.is_some(),
                    cx.listener(move |this, _, _, cx| {
                        let a = chosen.clone().map(Answer::Choice);
                        this.menu_choice = None;
                        this.service.update(cx, |s, cx| s.answer(a, cx));
                    }),
                )));
            }
            Question::Disc { message, requires, wrong } => {
                panel = panel.child(paragraph(message)).child(meta_text(tr!("The script looks for {file} on it.", file = requires)));
                if let Some(w) = wrong {
                    panel = panel.child(icon_line(Icon::Warning, theme::danger_text(), tr!("It is not in {path}.", path = w.display())));
                }
                panel = panel.child(div().flex().justify_end().child(action_button(
                    "si-disc",
                    ButtonKind::Primary,
                    tr!("Select the disc folder\u{2026}"),
                    true,
                    cx.listener(|_, _, window, cx| {
                        choose_path(window, cx, true, tr!("Select the disc folder"), |this: &mut Self, p, _, cx| {
                            this.service.update(cx, |s, cx| s.answer(Some(Answer::Folder(p.into())), cx));
                        });
                    }),
                )));
            }
        }
        Some(panel)
    }

    fn output(&mut self, s: &Snapshot, always: bool, cx: &mut Context<Self>) -> Div {
        let open = self.show_log || always;
        let was_open = std::mem::replace(&mut self.log_was_open, open);
        let mut col = div().flex().flex_col();
        if !always {
            let label = if open { tr!("Hide the output") } else { tr!("Show the output") };
            col = col.child(disclosure("si-log", open, label).on_click(cx.listener(|this, _, _, cx| {
                this.show_log = !this.show_log;
                cx.notify();
            })));
        }
        if open {
            if s.log.is_empty() {
                col = col.child(meta_text(tr!("No output yet.")));
            } else {
                col = col.child(mono_box("si-log-box", s.log.clone(), Some(&self.log_scroll)));
                // Follow the end when lines arrive (scrolling up stays put otherwise).
                if self.log_seen != s.log_count || !was_open {
                    self.log_seen = s.log_count;
                    self.log_scroll.scroll_to_bottom();
                }
            }
        }
        col
    }

    fn run_footer(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let mut footer = div().flex().items_center().w_full().gap(px(10.)).child(div().flex_1());
        match &s.end {
            End::Running { stopping } => footer
                .child(action_button(
                    "si-hide",
                    ButtonKind::Ghost,
                    tr!("Hide"),
                    true,
                    cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)),
                ))
                .child(action_button(
                    "si-stop",
                    ButtonKind::Danger,
                    if *stopping { tr!("Stopping\u{2026}") } else { tr!("Stop") },
                    !*stopping,
                    cx.listener(|this, _, _, cx| this.service.update(cx, |s, cx| s.stop(cx))),
                )),
            End::Failed { created_dir, step, .. } => {
                let retry = step.is_some();
                if *created_dir {
                    footer = footer.child(action_button(
                        "si-remove",
                        ButtonKind::Danger,
                        if s.removing { tr!("Removing\u{2026}") } else { tr!("Remove installed files") },
                        !s.removing,
                        cx.listener(|this, _, _, cx| this.service.update(cx, |s, cx| s.remove_files(cx))),
                    ));
                }
                footer
                    .child(action_button(
                        "si-back",
                        ButtonKind::Glass,
                        tr!("Back to the script"),
                        !s.removing,
                        cx.listener(|this, _, _, cx| this.service.update(cx, |s, cx| s.clear(cx))),
                    ))
                    .child(action_button(
                        "si-close",
                        if retry { ButtonKind::Ghost } else { ButtonKind::Primary },
                        tr!("Close"),
                        true,
                        cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)),
                    ))
                    .when(retry, |f| {
                        f.child(action_button(
                            "si-retry",
                            ButtonKind::Primary,
                            tr!("Try again from this step"),
                            !s.removing,
                            cx.listener(|this, _, _, cx| {
                                let res = this.service.update(cx, |s, cx| s.retry(cx));
                                if let Err(e) = res {
                                    this.error = e;
                                }
                                this.menu_choice = None;
                                cx.notify();
                            }),
                        ))
                    })
            }
            End::Done { .. } => footer.child(action_button(
                "si-done",
                ButtonKind::Primary,
                tr!("Done"),
                true,
                cx.listener(|this, _, _, cx| {
                    this.service.update(cx, |s, cx| s.clear(cx));
                    cx.emit(DialogEvent::ChangedAndClose);
                }),
            )),
        }
    }

    /// `(title, body, footer)` of the install.
    pub(super) fn run_view(&mut self, cx: &mut Context<Self>) -> (String, Div, Div) {
        let Some(s) = self.service.read(cx).run().map(Snapshot::of) else { return (String::new(), div(), div()) };
        let heading = match &s.end {
            End::Running { stopping: true } => tr!("Stopping the install of {title}\u{2026}", title = s.title.clone()),
            End::Running { .. } => tr!("Installing {title}", title = s.title.clone()),
            End::Done { title, .. } => tr!("{title} is in your Collection", title = title.clone()),
            End::Failed { cancelled: true, .. } => tr!("The install of {title} was stopped", title = s.title.clone()),
            End::Failed { .. } => tr!("The install of {title} failed", title = s.title.clone()),
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().text_size(px(18.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(heading));
        match &s.end {
            End::Failed { message, cancelled: false, .. } => {
                body = body.child(icon_line(Icon::Cancel, theme::danger_text(), message.clone()))
            }
            End::Done { notes, .. } if !notes.is_empty() => {
                body = body.child(
                    card_box().gap(px(8.)).py(px(12.)).children(notes.iter().map(|n| icon_line(Icon::Warning, rgb(0xebcb8b), n.clone()))),
                )
            }
            _ => {}
        }
        body = body.children(self.question(&s, cx));
        body = body.child(self.stage_list(&s));
        let failed = matches!(s.end, End::Failed { cancelled: false, .. });
        body = body.child(self.output(&s, failed, cx));
        if matches!(s.end, End::Failed { .. }) {
            body = body.child(meta_text(tr!("Files installed so far are in {folder}.", folder = s.folder.clone())));
        }
        if matches!(s.end, End::Failed { step: Some(_), .. }) {
            body = body.child(meta_text(tr!(
                "Fix what went wrong (the output above says what), then try again from this step: the steps before it are not run again."
            )));
        }
        let footer = self.run_footer(&s, cx);
        (tr!("Install from script").to_string(), body, footer)
    }
}
