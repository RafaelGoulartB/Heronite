//! Building blocks of the achievements: an icon with its lock, the progress
//! bar, the rarity pill, the source chip and the dates (the focus panel, the
//! Timeline and the "All achievements" dialog share them).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::Source;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;
use heronite_i18n::tr;

/// An achievement's icon; a locked one is dimmed and gets a small lock.
pub fn badge(image: Option<ImageSource>, unlocked: bool, grey: bool, size: f32) -> Div {
    let radius = px((size * 0.22).round());
    let mut d = div().relative().flex_none().size(px(size)).rounded(radius).bg(theme::ink_a(0.08));
    d = d.child(
        div()
            .size_full()
            .rounded(radius)
            .overflow_hidden()
            .when(!unlocked, |d| d.opacity(0.55))
            .children(image.map(|src| img(src).size_full().rounded(radius).object_fit(ObjectFit::Cover).grayscale(grey))),
    );
    if !unlocked && size >= 26. {
        let lock = (size * 0.44).min(18.);
        d = d.child(
            div()
                .absolute()
                .right(px(-3.))
                .bottom(px(-3.))
                .size(px(lock))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::rgb_a(26, 30, 37, 1.))
                .border_1()
                .border_color(theme::ink_a(0.12))
                .child(icon(Icon::Lock, px(lock * 0.6), theme::ink_a(0.7))),
        );
    }
    d
}

/// A progress bar, `fraction` in `0.0..=1.0`.
pub fn bar(fraction: f32, gold: bool, height: f32) -> Div {
    let fill: Hsla = if gold { theme::achievement_gold().into() } else { theme::accent().into() };
    div()
        .w_full()
        .h(px(height))
        .rounded_full()
        .bg(theme::ink_a(0.09))
        .overflow_hidden()
        .child(div().h_full().rounded_full().bg(fill).w(relative(fraction.clamp(0., 1.))))
}

/// "2.1%" in the colour of its rarity tier.
pub fn rarity_pill(percent: f32) -> Div {
    let colour = theme::rarity_colour(theme::rarity_tier(percent));
    div().px(px(6.)).rounded_full().bg(Hsla::from(colour).opacity(0.14)).text_size(px(11.)).text_color(colour).child(percent_label(percent))
}

pub fn percent_label(percent: f32) -> String {
    format!("{}%", heronite_i18n::decimal(percent as f64, 1))
}

pub fn source_chip(source: Source) -> Div {
    let dot = match source {
        Source::Steam => theme::rgb_a(102, 192, 244, 1.),
        Source::Gog => theme::rgb_a(164, 107, 220, 1.),
        Source::Epic => theme::rgb_a(217, 217, 217, 1.),
        Source::RetroAchievements => theme::rgb_a(240, 180, 41, 1.),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .px(px(7.))
        .py(px(2.))
        .rounded_full()
        .bg(theme::ink_a(0.08))
        .text_size(px(11.))
        .text_color(theme::ink_a(0.78))
        .child(div().size(px(6.)).rounded_full().bg(dot))
        .child(source.label())
}

pub fn gold_a(a: f32) -> Hsla {
    Hsla::from(theme::achievement_gold()).opacity(a)
}

/// "Oct 7", or "Oct 7, 2025" outside the current year.
pub fn short_day(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let (month, d) = (heronite_i18n::month_short(date.month()), date.day());
    if date.year() == chrono::Local::now().year() {
        tr!("{month} {d}", month, d)
    } else {
        tr!("{month} {d}, {year}", month, d, year = date.year())
    }
}

/// "Oct 7, 2026".
pub fn long_day(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    tr!("{month} {d}, {year}", month = heronite_i18n::month_short(date.month()), d = date.day(), year = date.year())
}

/// "Updated just now", "Updated 5 min ago", "Updated 3 h ago", "Updated Oct 7".
pub fn synced_label(at: chrono::DateTime<chrono::Utc>) -> String {
    let mins = (chrono::Utc::now() - at).num_minutes();
    match mins {
        ..=0 => tr!("Updated just now").to_string(),
        1..=59 => tr!("Updated {n} min ago", n = mins),
        60..=1439 => tr!("Updated {n} h ago", n = mins / 60),
        _ if at.timestamp() <= 0 => tr!("Not updated yet").to_string(),
        _ => tr!("Updated {date}", date = short_day(at.with_timezone(&chrono::Local).date_naive())),
    }
}
