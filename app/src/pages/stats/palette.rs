//! Command palette commands of the Statistics page (see `pages/commands.rs`).

use chrono::Datelike;
use gpui_kit::*;
use heronite_data::stats::{Period, Span};
use heronite_i18n::tr;

use super::tabs::Tab;
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
        for tab in [Tab::Journeys, Tab::Backlog, Tab::Discoveries] {
            let (title, sub, keywords, ic) = match tab {
                Tab::Journeys => (
                    tr!("Statistics: Journeys"),
                    tr!("Each game as a lane in time"),
                    tr!("stats journeys timeline games parallel"),
                    Icon::History,
                ),
                Tab::Backlog => (
                    tr!("Statistics: Backlog"),
                    tr!("Goal, beaten games and the queue"),
                    tr!("stats backlog goal beaten queue hltb"),
                    Icon::Flag,
                ),
                _ => (
                    tr!("Statistics: Discoveries"),
                    tr!("Your play in sentences, milestones and memories"),
                    tr!("stats discoveries insights milestones memories"),
                    Icon::Lightbulb,
                ),
            };
            let shown = page.is_some_and(|p| p.review.is_none() && p.tab == tab);
            v.push(
                PageCommand::new(format!("tab:{}", tab.id()), title, sub, ic)
                    .keywords(keywords)
                    .disabled(shown.then_some(tr!("Shown now"))),
            );
        }
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
            if !self.tab.spans().contains(span) {
                self.tab = Tab::Overview;
            }
            self.go(Period::of(*span, today()), cx);
        } else if let Some(tab) = id.strip_prefix("tab:").and_then(Tab::from_id) {
            self.review = None;
            self.set_tab(tab, cx);
        } else if id == "review-last" {
            self.open_review(today().year() - 1, cx);
        }
        cx.notify();
    }
}
