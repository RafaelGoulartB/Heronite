//! Console mode: the findings as text, their fixes as rows.

use gpui_kit::*;
use heronite_i18n::tr;

use super::{LaunchProblemDialog, State};
use crate::console::list::{Outcome, Row};
use crate::diagnosis::text;

impl LaunchProblemDialog {
    fn console_rows(&self) -> Vec<Row> {
        let busy = self.busy();
        let mut rows = Vec::new();
        for (i, f) in self.report.findings.iter().enumerate() {
            rows.push(Row::header(text::headline(f)));
            for (j, fix) in f.fixes.iter().enumerate() {
                let label = match fix {
                    heronite_launch::diagnose::Fix::Terminal(_) if self.copied == Some((i, j)) => tr!("Copy again").to_string(),
                    _ => text::fix_label(fix, &self.report),
                };
                rows.push(Row::action(format!("fix-{i}-{j}"), label).enabled(!busy));
            }
        }
        if self.report.has_log {
            rows.push(Row::action("log", tr!("View log")).enabled(!busy));
        }
        rows.push(Row::action("close", tr!("Close")).enabled(!busy));
        rows
    }

    fn console_message(&self) -> String {
        let mut lines = vec![text::how_it_ended(&self.report)];
        match self.report.main() {
            Some(f) => lines.push(text::explanation(f, &self.report)),
            None => lines.push(tr!("The last launch log shows no known problem.").into()),
        }
        if let State::Working { lines: out, .. } = &self.state {
            lines.push(tr!("Working\u{2026}").into());
            lines.extend(out.last().cloned());
        }
        lines.join("\n")
    }

    pub(super) fn render_console(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows();
        let message = self.console_message();
        let error = match &self.state {
            State::Failed { message, .. } => Some(SharedString::from(message.clone())),
            _ => None,
        };
        crate::console::list::simple_dialog(&mut self.console_list, "console-launch-problem", tr!("Launch problem"), &message, error, &rows)
    }
}

impl crate::console::PadTarget for LaunchProblemDialog {
    fn on_pad(&mut self, pad: crate::console::Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => match id.as_ref() {
                "close" => self.close(cx),
                "log" => (self.actions.view_log.clone())(cx),
                other => {
                    let mut parts = other.strip_prefix("fix-").unwrap_or_default().split('-').map(|n| n.parse::<usize>().ok());
                    if let (Some(Some(i)), Some(Some(j))) = (parts.next(), parts.next()) {
                        self.run_fix((i, j), cx);
                    }
                }
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<crate::console::Hint> {
        use crate::console::{hint, Btn};
        vec![hint(Btn::A, tr!("Select")), hint(Btn::B, tr!("Close"))]
    }
}
