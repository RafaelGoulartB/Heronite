//! Hover tooltips of the Statistics page: the element under the cursor sets
//! [`Hover`]; the tip is built here from the statistics and follows the
//! mouse.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_secs, Stats};
use heronite_i18n::{tr, trn};

use super::{bucket_label, day_label, day_part, series_color, short_date, weekday_plural, Hover, StatsPage};
use crate::theme;

struct Row {
    dot: Option<Hsla>,
    label: String,
    value: String,
}

struct Body {
    title: String,
    rows: Vec<Row>,
    note: Option<String>,
}

fn row(dot: Option<Hsla>, label: impl Into<String>, value: impl Into<String>) -> Row {
    Row { dot, label: label.into(), value: value.into() }
}

/// Listener for `on_hover`: sets `h` while the cursor is over the element.
pub(super) fn hover(h: Hover, cx: &mut Context<StatsPage>) -> impl Fn(&bool, &mut Window, &mut App) + 'static + use<> {
    let page = cx.entity().downgrade();
    move |on: &bool, _: &mut Window, cx: &mut App| {
        let on = *on;
        page.update(cx, |this, cx| {
            if on {
                this.hover = Some(h);
            } else if this.hover == Some(h) {
                this.hover = None;
            }
            cx.notify();
        })
        .ok();
    }
}

impl StatsPage {
    /// Games of bucket `b`, the top five with their colours plus Other.
    fn bucket_rows(&self, st: &Stats, b: usize) -> Vec<Row> {
        let Some(games) = st.bucket_games.get(b) else { return Vec::new() };
        let mut rows: Vec<Row> =
            games.iter().take(5).map(|&(g, secs)| row(Some(series_color(st.series_of(g))), self.game_title(g), fmt_secs(secs))).collect();
        let rest: f64 = games.iter().skip(5).map(|(_, s)| s).sum();
        if rest >= 60. {
            rows.push(row(Some(theme::chart_other()), trn!(games.len() - 5, "{n} more game", "{n} more games"), fmt_secs(rest)));
        }
        rows
    }

    fn tip_body(&self, st: &Stats, h: Hover) -> Option<Body> {
        let heat_total: f64 = st.heat.iter().flatten().sum();
        Some(match h {
            Hover::Bar(i) => {
                let b = st.buckets.get(i)?;
                let mut rows = vec![row(None, tr!("Played"), fmt_secs(b.secs))];
                rows.extend(self.bucket_rows(st, i));
                Body { title: bucket_label(st.period.span, b.start, b.end), rows, note: None }
            }
            Hover::Heat(d, hr) => Body {
                title: format!("{}, {hr:02}:00\u{2013}{:02}:00", weekday_plural(d), (hr + 1) % 24),
                rows: vec![row(None, tr!("Played"), fmt_secs(st.heat[d][hr]))],
                note: None,
            },
            Hover::Day(d) => {
                let secs = st.day_secs(d);
                Body {
                    title: day_label(d),
                    rows: vec![row(
                        None,
                        if secs > 0. { tr!("Played") } else { tr!("No play") },
                        if secs > 0. { fmt_secs(secs) } else { String::new() },
                    )],
                    note: None,
                }
            }
            Hover::Game(g) => {
                let s = st.game(g)?;
                let lib = self.lib.as_ref()?;
                let game = lib.games.get(g)?;
                let status =
                    lib.statuses.iter().find(|x| x.id == game.status_id).map(|x| heronite_data::status::status_label(&x.name).to_string());
                let mut rows = vec![row(None, tr!("Time"), fmt_secs(s.secs)), row(None, tr!("Sessions"), s.sessions.to_string())];
                if let Some(f) = s.first {
                    rows.push(row(None, tr!("First played"), short_date(f.date())));
                }
                if let Some(l) = s.last {
                    rows.push(row(None, tr!("Last played"), short_date(l.date())));
                }
                if let Some(status) = status {
                    rows.push(row(
                        Some(
                            theme::status_color(
                                &lib.statuses.iter().find(|x| x.id == game.status_id).map_or(String::new(), |x| x.slug.clone()),
                            )
                            .into(),
                        ),
                        tr!("Status"),
                        status,
                    ));
                }
                Body { title: game.title.clone(), rows, note: None }
            }
            Hover::Genre(i) => {
                let (name, secs) = self.derived.genres.get(i)?;
                Body { title: name.clone(), rows: vec![row(None, tr!("Time in these games"), fmt_secs(*secs))], note: None }
            }
            Hover::Source(i) => {
                let (name, secs) = self.derived.sources.get(i)?;
                let share = secs / st.total_secs.max(1.) * 100.;
                Body {
                    title: (*name).to_string(),
                    rows: vec![row(None, tr!("Time"), fmt_secs(*secs)), row(None, tr!("Share"), format!("{share:.0}%"))],
                    note: None,
                }
            }
            Hover::Part(i) => {
                let (name, range) = day_part(i);
                let share = st.day_parts[i] / heat_total.max(1.) * 100.;
                Body {
                    title: format!("{name} \u{b7} {range}"),
                    rows: vec![row(None, tr!("Time"), fmt_secs(st.day_parts[i])), row(None, tr!("Share"), format!("{share:.0}%"))],
                    note: None,
                }
            }
        })
    }

    /// The tooltip of the hovered element, at the mouse.
    pub(super) fn render_tip(&self, st: &Stats, window: &Window) -> Option<AnyElement> {
        let body = self.tip_body(st, self.hover?)?;
        let mouse = window.mouse_position();
        let rows = body.rows.into_iter().map(|r| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(18.))
                .text_size(px(12.5))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .min_w_0()
                        .text_color(theme::text_muted())
                        .children(r.dot.map(|c| div().flex_none().size(px(8.)).rounded_full().bg(c)))
                        .child(div().truncate().child(r.label)),
                )
                .child(div().flex_none().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(r.value))
        });
        let card = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .min_w(px(170.))
            .max_w(px(320.))
            .px(px(13.))
            .py(px(11.))
            .rounded(px(12.))
            .bg(rgba(0x0e1117f5))
            .border_1()
            .border_color(theme::white_a(0.14))
            .shadow(vec![BoxShadow {
                color: theme::black_a(0.6),
                offset: point(px(0.), px(14.)),
                blur_radius: px(40.),
                spread_radius: px(0.),
                inset: false,
            }])
            .font_family(theme::font_body())
            .child(
                div()
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(13.))
                    .text_color(theme::text_primary())
                    .mb(px(2.))
                    .child(body.title),
            )
            .children(rows)
            .children(body.note.map(|n| div().mt(px(2.)).text_size(px(12.)).text_color(theme::text_dim()).child(n)));
        Some(
            deferred(anchored().position(mouse + point(px(14.), px(16.))).snap_to_window_with_margin(px(8.)).child(card))
                .with_priority(60)
                .into_any_element(),
        )
    }
}
