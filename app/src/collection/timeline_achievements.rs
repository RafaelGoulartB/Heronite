//! The achievements of each Timeline day: a "4 achievements" chip on the
//! day's line and their icons under its screenshots. An unlock goes to a day
//! by the screenshots' rule (the session it happened in, else its own day),
//! and only to days the Timeline already lists: history from before the app
//! tracked the game adds no days. Settings → Achievements turns it off.

use std::collections::HashMap;

use chrono::NaiveDate;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::Achievement;
use heronite_data::models::LocalGameSession;
use heronite_data::timeline::days_of_moments;

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::achievements::badge;
use crate::ui::icon;
use heronite_i18n::{tr, trn};

/// Icons a day shows before "+N".
const ICONS: usize = 6;
const ICON: f32 = 26.;

/// The unlocks of one day, latest first.
pub(super) type DayUnlocks = HashMap<NaiveDate, Vec<Achievement>>;

/// Puts unlocks (with a time) on the days of `sessions`.
pub(super) fn by_day(unlocked: &[&Achievement], sessions: &[LocalGameSession]) -> DayUnlocks {
    let timed: Vec<&Achievement> = unlocked.iter().copied().filter(|a| a.unlocked_at.is_some()).collect();
    let moments: Vec<i64> = timed.iter().filter_map(|a| a.unlocked_at.map(|t| t.timestamp_millis())).collect();
    let days = days_of_moments(sessions, &moments, &chrono::Local);
    let mut out: DayUnlocks = HashMap::new();
    for (a, day) in timed.into_iter().zip(days) {
        if let Some(day) = day {
            out.entry(day).or_default().push(a.clone());
        }
    }
    for list in out.values_mut() {
        list.sort_by_key(|a| std::cmp::Reverse(a.unlocked_at));
    }
    out
}

impl CollectionView {
    /// The focused game's unlocks by Timeline day (empty when the Timeline
    /// shows none).
    pub(super) fn timeline_unlocks(&mut self, key: &SharedString, cx: &mut Context<Self>) -> DayUnlocks {
        let Some(game) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else { return DayUnlocks::new() };
        let service = crate::services::achievements::get(cx);
        let service = service.read(cx);
        if !service.prefs().timeline {
            return DayUnlocks::new();
        }
        let Some(set) = service.entry(&crate::services::achievements::key_of(&game.runner, &game.app_name)).and_then(|e| e.primary())
        else {
            return DayUnlocks::new();
        };
        let unlocked: Vec<&Achievement> = set.achievements.iter().filter(|a| a.unlocked).collect();
        let sessions: &[LocalGameSession] = game.sessions.as_deref().map(Vec::as_slice).unwrap_or(&[]);
        by_day(&unlocked, sessions)
    }

    /// "4 achievements" on a day's line; opens the list.
    pub(super) fn achievements_chip(&mut self, day_i: usize, n: usize, key: &SharedString, cx: &mut Context<Self>) -> AnyElement {
        let key = key.clone();
        div()
            .id(SharedString::from(format!("tl-ach-{day_i}")))
            .flex()
            .items_center()
            .gap(px(5.))
            .ml(px(4.))
            .px(px(7.))
            .rounded(px(5.))
            .cursor_pointer()
            .bg(theme::accent_tint())
            .hover(|s| s.bg(theme::accent_alpha(0.26)))
            .text_size(px(11.5))
            .text_color(theme::accent())
            .child(icon(Icon::Trophy, px(12.), theme::accent()))
            .child(trn!(n, "{n} achievement", "{n} achievements"))
            .on_click(cx.listener(move |this, _, window, cx| this.open_achievements(&key, window, cx)))
            .into_any_element()
    }

    /// The icons and names of a day's unlocks, under its screenshots.
    pub(super) fn achievements_line(
        &mut self,
        day_i: usize,
        list: &[Achievement],
        key: &SharedString,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut icons = div().flex().items_center().gap(px(6.));
        for a in list.iter().take(ICONS) {
            let image = self.achievement_icon(a.icon_now(), ICON, scale);
            icons = icons.child(badge(image, true, false, ICON));
        }
        if list.len() > ICONS {
            icons = icons.child(div().text_size(px(11.5)).text_color(theme::ink_a(0.6)).child(format!("+{}", list.len() - ICONS)));
        }
        let names: Vec<&str> = list.iter().take(2).map(|a| a.name.as_str()).collect();
        let more = list.len().saturating_sub(names.len());
        let text = if more > 0 { tr!("{names} +{n}", names = names.join(", "), n = more) } else { names.join(", ") };
        let key = key.clone();
        div()
            .id(SharedString::from(format!("tl-ach-line-{day_i}")))
            .mt(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .min_w_0()
            .cursor_pointer()
            .child(icons)
            .child(div().min_w_0().truncate().text_size(px(12.)).text_color(theme::ink_a(0.72)).child(text))
            .on_click(cx.listener(move |this, _, window, cx| this.open_achievements(&key, window, cx)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::by_day;
    use chrono::{NaiveDate, TimeZone, Utc};
    use heronite_achievements::Achievement;

    fn unlocked(id: &str, at: Option<chrono::DateTime<Utc>>) -> Achievement {
        Achievement { id: id.into(), name: id.into(), unlocked: true, unlocked_at: at, ..Default::default() }
    }

    #[test]
    fn unlocks_go_to_their_day_latest_first_and_untimed_ones_are_left_out() {
        let noon = |d| chrono::Local.with_ymd_and_hms(2026, 9, d, 12, 0, 0).unwrap().with_timezone(&Utc);
        let a = unlocked("a", Some(noon(27)));
        let b = unlocked("b", Some(noon(27) + chrono::Duration::hours(1)));
        let c = unlocked("c", Some(noon(22)));
        let d = unlocked("d", None);
        let days = by_day(&[&a, &b, &c, &d], &[]);
        let day27 = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
        let ids: Vec<_> = days[&day27].iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert_eq!(days.len(), 2);
    }
}
