//! Command palette commands of the Statistics page (see `pages/commands.rs`).

use chrono::Datelike;
use gpui_kit::*;
use heronite_data::stats::{Period, Span};
use heronite_i18n::tr;

use super::{today, StatsPage};
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

const SPANS: [(Span, &str); 4] = [(Span::Week, "week"), (Span::Month, "month"), (Span::Year, "year"), (Span::All, "all")];

impl PaletteCommands for StatsPage {
    fn palette_commands(page: Option<&Self>, _ctx: &CommandCtx, _cx: &App) -> Vec<PageCommand> {
        let mut v: Vec<PageCommand> = SPANS
            .iter()
            .map(|(span, id)| {
                let shown = page
                    .is_some_and(|p| p.review.is_none() && p.period.span == *span && (*span == Span::All || p.period.contains(today())));
                let title = match span {
                    Span::Week => tr!("Statistics: this week"),
                    Span::Month => tr!("Statistics: this month"),
                    Span::Year => tr!("Statistics: this year"),
                    Span::All => tr!("Statistics: all time"),
                };
                PageCommand::new(format!("span:{id}"), title, tr!("Play time and habits"), Icon::BarChart)
                    .keywords(tr!("stats playtime hours period"))
                    .disabled(shown.then_some(tr!("Shown now")))
            })
            .collect();
        let last = today().year() - 1;
        v.push(
            PageCommand::new("review-last", tr!("Year in review {last}", last), tr!("Last year in games, as stories"), Icon::BarChart)
                .keywords(tr!("wrapped recap retrospective")),
        );
        v
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some((span, ..)) = id.strip_prefix("span:").and_then(|s| SPANS.iter().find(|(_, k)| *k == s)) {
            self.review = None;
            self.go(Period::of(*span, today()), cx);
        } else if id == "review-last" {
            self.open_review(today().year() - 1, cx);
        }
        cx.notify();
    }
}
