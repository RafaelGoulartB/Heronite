//! The tabs of the Statistics page. Each tab answers one question about the
//! same period: the period switch next to the tabs is shared, and a tab that
//! cannot show a span (Journeys of a week) shows the year around it instead.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{Period, Span};
use heronite_i18n::{tr, trc};

use super::{today, StatsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tab {
    /// How much and when: the hero, month by month, top games, habits,
    /// records, every day and the genres.
    Overview,
    /// How each game went: a lane per game in time.
    Journeys,
    /// What gets finished: the goal, beaten games, the queue.
    Backlog,
    /// What the numbers tell: sentences, milestones, memories.
    Discoveries,
    /// The screenshots: moments, the most photographed games, a photo a month.
    Album,
}

impl Tab {
    pub(super) const ALL: [Tab; 5] = [Tab::Overview, Tab::Journeys, Tab::Backlog, Tab::Discoveries, Tab::Album];

    pub(super) fn label(self) -> &'static str {
        match self {
            Tab::Overview => tr!("Overview"),
            Tab::Journeys => tr!("Journeys"),
            // The Collection's "Backlog" view is another thing ("Pendentes").
            Tab::Backlog => trc!("stats", "Backlog"),
            Tab::Discoveries => tr!("Discoveries"),
            Tab::Album => tr!("Album"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::Overview => Icon::BarChart,
            Tab::Journeys => Icon::History,
            Tab::Backlog => Icon::Flag,
            Tab::Discoveries => Icon::Lightbulb,
            Tab::Album => Icon::PhotoCamera,
        }
    }

    /// The spans the tab can show.
    pub(super) fn spans(self) -> &'static [Span] {
        match self {
            Tab::Overview => &[Span::Week, Span::Month, Span::Year, Span::All],
            Tab::Journeys => &[Span::Month, Span::Year, Span::All],
            Tab::Backlog => &[Span::Year, Span::All],
            Tab::Discoveries | Tab::Album => &[Span::Week, Span::Month, Span::Year, Span::All],
        }
    }

    /// `HERONITE_DEBUG_STATS_TAB` and the palette commands.
    pub(super) fn id(self) -> &'static str {
        match self {
            Tab::Overview => "overview",
            Tab::Journeys => "journeys",
            Tab::Backlog => "backlog",
            Tab::Discoveries => "discoveries",
            Tab::Album => "album",
        }
    }

    pub(super) fn from_id(id: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// The period `tab` shows for `period`: the same one, or the year around it.
pub(super) fn period_for(tab: Tab, period: Period) -> Period {
    if tab.spans().contains(&period.span) {
        return period;
    }
    let anchor = if period.contains(today()) { today() } else { period.start };
    Period::of(Span::Year, anchor)
}

impl StatsPage {
    pub(super) fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        self.prefs.tab = Some(tab.id().to_string());
        self.save_prefs(cx);
        let period = period_for(tab, self.period);
        self.go(period, cx);
    }

    /// The tabs as a capsule of pills (the period switch's look, with the
    /// app's tab colours so the two read apart).
    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> Div {
        let mut seg = div()
            .flex()
            .items_center()
            .p(px(3.))
            .gap(px(2.))
            .rounded_full()
            .bg(rgba(0x161b24d6))
            .border_1()
            .border_color(theme::white_a(0.08));
        for tab in Tab::ALL {
            let on = self.tab == tab;
            let color = if on { theme::text_primary() } else { theme::text_muted() };
            seg = seg.child(
                div()
                    .id(SharedString::from(format!("stats-tab-{}", tab.id())))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .px(px(15.))
                    .rounded_full()
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(13.5))
                    .text_color(color)
                    .when(on, |d| d.bg(theme::white_a(0.16)))
                    .when(!on, |d| d.cursor_pointer().hover(|s| s.bg(theme::white_a(0.06)).text_color(theme::text_primary())))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
                    .child(icon(tab.icon(), px(16.), color))
                    .child(tab.label()),
            );
        }
        seg
    }
}

#[cfg(test)]
mod tests {
    use super::{period_for, Tab};
    use chrono::NaiveDate;
    use heronite_data::stats::{Period, Span};

    #[test]
    fn a_tab_without_the_span_shows_the_year_around_it() {
        let d = NaiveDate::from_ymd_opt(2025, 3, 12).unwrap();
        let week = Period::of(Span::Week, d);
        assert_eq!(period_for(Tab::Overview, week), week);
        assert_eq!(period_for(Tab::Journeys, week), Period::of(Span::Year, d));
        let month = Period::of(Span::Month, d);
        assert_eq!(period_for(Tab::Journeys, month), month);
    }
}
