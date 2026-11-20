//! "When details arrive": what belongs to no single source (filling in the
//! background, copying covers, adult covers, age ratings and their board).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::age_rating::{self, RatingSystem};
use heronite_i18n::tr;

use super::super::SettingsPage;
use super::persist;
use crate::ui::controls::{caption, card as rows, toggle_row};
use crate::ui::fields::{meta_text, select_field};

/// [`super::State::open_select`] of the age rating board.
const AGE_SELECT: &str = "age-board";

/// The age rating boards the setting offers: Automatic (the interface
/// language's), then each board with where it rates.
pub(super) fn board_options() -> Vec<(&'static str, String)> {
    let auto = age_rating::board_order(age_rating::AUTO, heronite_i18n::current())[0];
    std::iter::once((age_rating::AUTO, tr!("Automatic ({board})", board = auto.name())))
        .chain(RatingSystem::BOARDS.into_iter().map(|b| (b.id(), format!("{} \u{2014} {}", b.name(), b.region()))))
        .collect()
}

pub(super) fn board_index(d: &SettingsPage) -> usize {
    board_options().iter().position(|(id, _)| *id == d.settings.metadata.age_rating_system).unwrap_or(0)
}

pub(super) fn set_board(d: &mut SettingsPage, index: usize, cx: &mut Context<SettingsPage>) {
    let Some((id, _)) = board_options().into_iter().nth(index) else { return };
    persist(d, cx, |m| m.age_rating_system = id.to_owned());
}

fn board_select(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let options = board_options();
    let current = board_index(d);
    select_field(
        "md-age-board",
        Some(tr!("Rating board shown first")),
        options[current].1.clone(),
        false,
        d.metadata.open_select.as_deref() == Some(AGE_SELECT),
        options.iter().enumerate().map(|(i, (_, label))| (SharedString::from(label.clone()), i == current)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            let open = d.metadata.open_select.as_deref() == Some(AGE_SELECT);
            d.metadata.open_select = (!open).then(|| AGE_SELECT.into());
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.metadata.open_select = None;
            set_board(d, i, cx);
        },
    )
    .into_any_element()
}

/// What "Allow adult covers" does, under it.
pub(super) fn adult_help() -> &'static str {
    tr!(
        "Covers and backgrounds a source marks as suggestive or explicit. Off, they are never shown or downloaded and the next source gives the picture; pictures already saved stay."
    )
}

pub(super) fn card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let m = &d.settings.metadata;
    let (auto_fill, download, show_age, adult) = (m.auto_fill_missing, m.download_images, m.show_age_ratings, m.allow_adult_covers);
    let card = rows([
        toggle_row("md-autofill", tr!("Fill missing metadata in the background"), None, auto_fill)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.auto_fill_missing = !auto_fill)))
            .into_any_element(),
        toggle_row("md-download", tr!("Copy covers to disk"), Some(tr!("Off keeps the CDN URLs instead of storing the images.")), download)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.download_images = !download)))
            .into_any_element(),
        toggle_row("md-adult", tr!("Allow adult covers"), Some(adult_help()), adult)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.allow_adult_covers = !adult)))
            .into_any_element(),
        toggle_row("md-age-show", tr!("Show age ratings"), Some(tr!("In a game's facts and Details.")), show_age)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.show_age_ratings = !show_age)))
            .into_any_element(),
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .py(px(12.))
            .child(board_select(d, cx))
            .child(meta_text(tr!(
                "Shown in a game's facts and used by the View menu's \u{201c}Ages up to\u{201d}. When this board did not rate a game, the next one that did is shown."
            )))
            .into_any_element(),
    ]);
    div().flex().flex_col().child(caption(tr!("When details arrive"))).child(card)
}
