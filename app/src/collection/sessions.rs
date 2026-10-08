//! Play session history in the focus panel's side column (`SessionHistory`):
//! "Play sessions (N)" and the newest 12 sessions with their date, length
//! (`HH:MM`) and origin (Heroic or Playnite). Reads the sessions already
//! loaded with the library.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::LocalGameSession;
use heronite_data::sessions::{
    format_session_date, format_session_duration, session_source_label, sorted_newest_first, SHOWN_SESSIONS,
};
use heronite_data::text::DateLocale;

use super::focus::{h3, panel};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// `(date, "HH:MM · Source")` of the rows to show.
pub fn rows(sessions: &[LocalGameSession], locale: DateLocale) -> Vec<(SharedString, SharedString)> {
    sorted_newest_first(sessions)
        .into_iter()
        .take(SHOWN_SESSIONS)
        .map(|s| {
            (
                format_session_date(&s.started_at, locale).into(),
                format!("{} · {}", format_session_duration(s.elapsed_seconds.get()), session_source_label(&s.source)).into(),
            )
        })
        .collect()
}

pub fn view(total: usize, rows: Vec<(SharedString, SharedString)>) -> AnyElement {
    let mut list = div()
        .id("sessions-list")
        .flex()
        .flex_col()
        .gap(px(2.))
        .max_h(px(220.))
        .overflow_y_scroll();
    for (date, meta) in rows {
        list = list.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .py(px(3.))
                .text_size(px(13.33))
                .child(div().min_w_0().truncate().text_color(theme::white_a(0.94)).child(date))
                .child(div().flex_none().text_color(theme::white_a(0.62)).child(meta)),
        );
    }
    panel(false)
        .child(
            h3(format!("Play sessions ({total})"))
                .flex()
                .items_center()
                .gap(px(6.4))
                .child(icon(Icon::History, px(18.), theme::white_a(0.8))),
        )
        .child(list)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{rows, DateLocale, LocalGameSession};
    use heronite_data::JsNum;

    fn s(started: &str, secs: f64, source: &str) -> LocalGameSession {
        LocalGameSession {
            started_at: started.into(),
            ended_at: started.into(),
            elapsed_seconds: JsNum(secs),
            source: source.into(),
            game_action_name: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn rows_are_newest_first_capped_and_labelled() {
        let mut list: Vec<_> = (1..=20).map(|d| s(&format!("2026-01-{d:02}T12:00:00.000Z"), 3660., "heroic")).collect();
        list.push(s("2026-02-01T12:00:00.000Z", 60., "playnite"));
        let rows = rows(&list, DateLocale::En);
        assert_eq!(rows.len(), 12);
        assert_eq!(rows[0].1.as_ref(), "00:01 · Playnite");
        assert_eq!(rows[1].1.as_ref(), "01:01 · Heroic");
    }
}
