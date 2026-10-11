//! Command palette commands of the Statistics page (see `pages/commands.rs`).

use chrono::Datelike;
use gpui_kit::*;
use heronite_data::stats::{Period, Span};

use super::{today, StatsPage};
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

const SPANS: [(Span, &str, &str); 4] =
    [(Span::Week, "week", "This week"), (Span::Month, "month", "This month"), (Span::Year, "year", "This year"), (Span::All, "all", "All time")];

impl PaletteCommands for StatsPage {
    fn palette_commands(page: Option<&Self>, _ctx: &CommandCtx, _cx: &App) -> Vec<PageCommand> {
        let mut v: Vec<PageCommand> = SPANS
            .iter()
            .map(|(span, id, label)| {
                let shown = page.is_some_and(|p| p.review.is_none() && p.period.span == *span && (*span == Span::All || p.period.contains(today())));
                PageCommand::new(format!("span:{id}"), format!("Statistics: {}", label.to_lowercase()), "Play time and habits", Icon::BarChart)
                    .keywords("stats playtime hours period")
                    .disabled(shown.then_some("Shown now"))
            })
            .collect();
        let last = today().year() - 1;
        v.push(PageCommand::new("review-last", format!("Year in review {last}"), "Last year in games, as stories", Icon::BarChart).keywords("wrapped recap retrospective"));
        v
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some((span, ..)) = id.strip_prefix("span:").and_then(|s| SPANS.iter().find(|(_, k, _)| *k == s)) {
            self.review = None;
            self.go(Period::of(*span, today()), cx);
        } else if id == "review-last" {
            self.open_review(today().year() - 1, cx);
        }
        cx.notify();
    }
}
