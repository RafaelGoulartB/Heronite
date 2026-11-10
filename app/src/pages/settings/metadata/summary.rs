//! The card at the top of Settings → Metadata: how many of the Collection's
//! games have details, and "Update metadata…" (the dialog on the games on
//! screen).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::CollectionGameMetadata;
use heronite_data::Library;
use heronite_i18n::tr;

use super::super::SettingsPage;
use crate::dialogs::metadata_wizard::{hub, HubGlobal};
use crate::icons::Icon;
use crate::services::widgets::progress_bar;
use crate::theme;
use crate::ui::controls::{card_box, icon_action_button, icon_badge, ButtonKind};

/// How far the Collection's details are. Counted off the UI thread when the
/// section shows and when the library changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Coverage {
    pub with_details: usize,
    pub total: usize,
}

impl Coverage {
    pub fn of(lib: &Library) -> Self {
        let with_details = lib.games.iter().filter(|g| has_details(g.metadata.as_deref())).count();
        Self { with_details, total: lib.games.len() }
    }
}

/// Whether a game's record holds something a source found (a fetch that
/// found nothing leaves only its time and error).
fn has_details(m: Option<&CollectionGameMetadata>) -> bool {
    let Some(m) = m else { return false };
    let text = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.trim().is_empty());
    text(&m.description)
        || text(&m.release_date)
        || !m.developers.is_empty()
        || !m.publishers.is_empty()
        || !m.genres.is_empty()
        || m.critic_score.is_some()
}

/// Counts the games with details in the library the page was handed.
pub(super) fn count(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let Some(lib) = d.storage.library() else { return };
    d.metadata.coverage_task = Some(cx.spawn(async move |this, cx| {
        let coverage = cx.background_spawn(async move { Coverage::of(&lib) }).await;
        this.update(cx, |d, cx| {
            d.metadata.coverage = Some(coverage);
            cx.notify();
        })
        .ok();
    }));
}

pub(super) fn card(d: &SettingsPage) -> Div {
    let text = div().flex().flex_col().gap(px(8.)).flex_1().min_w_0();
    let text = match d.metadata.coverage {
        Some(c) if c.total > 0 => text
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!(
                "{n} of {total} games have details",
                n = c.with_details,
                total = c.total
            )))
            .child(div().max_w(px(260.)).child(progress_bar(Some(c.with_details as f32 * 100. / c.total as f32), 4., theme::accent()))),
        _ => text.child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Details and pictures for your games"))),
    };
    let update = icon_action_button("md-update", ButtonKind::Primary, Icon::CloudDownload, tr!("Update metadata\u{2026}"), |_, _, cx| {
        if cx.has_global::<HubGlobal>() {
            hub(cx).update(cx, |h, cx| h.request_update(cx));
        }
    });
    card_box().flex_row().items_center().gap(px(14.)).py(px(14.)).child(icon_badge(Icon::TravelExplore)).child(text).child(update)
}

#[cfg(test)]
mod tests {
    use heronite_data::models::CollectionGameMetadata;

    use super::has_details;

    #[test]
    fn a_record_without_details_does_not_count() {
        assert!(!has_details(None));
        let mut m = CollectionGameMetadata { fetch_error: Some("not found".into()), fetched_at: Some("2026".into()), ..Default::default() };
        assert!(!has_details(Some(&m)));
        m.description = Some("  ".into());
        assert!(!has_details(Some(&m)));
        m.genres = vec!["RPG".into()];
        assert!(has_details(Some(&m)));
    }
}
