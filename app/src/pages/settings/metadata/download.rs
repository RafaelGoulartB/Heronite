//! The database of an open source row, for any source that downloads one
//! (`SourceRow::download`): "Download the database (105 MB)" before it is
//! there, its progress while it comes (with "Cancel"), then when it was
//! refreshed, how much room it takes, "Refresh now" and "Remove the
//! database". The hub runs the downloads (`MetadataHub::download_source`).

use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::util::file_size_local;
use heronite_i18n::{tr, trn};
use heronite_metadata::sources::{DownloadRow, Phase, ProgressSnapshot};

use super::super::SettingsPage;
use crate::dialogs::metadata_update::source_downloads::SourceDownloadUi;
use crate::dialogs::metadata_wizard::{hub, HubGlobal};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, icon_action_button, ButtonKind};
use crate::ui::fields::meta_text;
use crate::ui::icon;

const BAR_W: f32 = 420.;

/// The download's state for `id` as the hub has it.
fn ui_of(id: &str, cx: &App) -> SourceDownloadUi {
    if cx.has_global::<HubGlobal>() {
        hub(cx).read(cx).source_download(id)
    } else {
        SourceDownloadUi::default()
    }
}

fn mb(bytes: u64) -> String {
    file_size_local(bytes as f64)
}

/// "every week", "every 3 days".
fn every(days: u32) -> String {
    if days == 7 {
        tr!("every week").into()
    } else {
        trn!(days as usize, "every day", "every {n} days")
    }
}

/// "today", "yesterday", "3 days ago".
fn when(at: SystemTime) -> String {
    let days = SystemTime::now().duration_since(at).map_or(0, |d| d.as_secs() / 86_400);
    match days {
        0 => tr!("Refreshed today").into(),
        1 => tr!("Refreshed yesterday").into(),
        n => trn!(n as usize, "Refreshed {n} day ago", "Refreshed {n} days ago"),
    }
}

/// The line under the bar.
fn progress_text(p: &ProgressSnapshot) -> String {
    match p.phase {
        Phase::Starting => tr!("Starting the download\u{2026}").into(),
        Phase::Downloading => match p.total {
            Some(total) => tr!("Downloading\u{2026} {done} of {total}", done = mb(p.done), total = mb(total)),
            None => tr!("Downloading\u{2026} {done}", done = mb(p.done)),
        },
        Phase::Preparing => {
            let percent = p.fraction().map_or(0, |f| (f * 100.).round() as u32);
            tr!("Preparing the database\u{2026} {percent}%", percent)
        }
    }
}

fn bar(fraction: Option<f32>) -> Div {
    div()
        .w_full()
        .max_w(px(BAR_W))
        .h(px(6.))
        .rounded_full()
        .overflow_hidden()
        .bg(theme::ink_a(0.08))
        .child(div().h_full().w(relative(fraction.unwrap_or(0.04))).rounded_full().bg(theme::accent()))
}

pub(super) fn block(id: &str, row: &DownloadRow, cx: &mut Context<SettingsPage>) -> Div {
    let ui = ui_of(id, cx);
    let mut col = div().flex().flex_col().gap(px(10.));
    if let Some(p) = &ui.progress {
        let sid = id.to_string();
        let cancel = action_button(
            SharedString::from(format!("md-dl-cancel-{id}")),
            ButtonKind::Ghost,
            tr!("Cancel"),
            true,
            cx.listener(move |_, _, _, cx| hub(cx).update(cx, |h, _| h.cancel_source_download(&sid))),
        );
        return col
            .child(bar(p.fraction()))
            .child(div().flex().items_center().gap(px(12.)).child(meta_text(progress_text(p))).child(cancel));
    }
    if row.state.ready {
        let mut facts = Vec::new();
        if let Some(at) = row.state.refreshed_at {
            facts.push(when(at));
        }
        facts.push(tr!("{size} on this PC", size = mb(row.state.size_on_disk)));
        let (refresh_id, remove_id) = (id.to_string(), id.to_string());
        let buttons = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .child(icon_action_button(
                SharedString::from(format!("md-dl-refresh-{id}")),
                ButtonKind::Glass,
                Icon::Refresh,
                tr!("Refresh now"),
                cx.listener(move |_, _, _, cx| hub(cx).update(cx, |h, cx| h.download_source(&refresh_id, cx))),
            ))
            .child(action_button(
                SharedString::from(format!("md-dl-remove-{id}")),
                ButtonKind::Ghost,
                tr!("Remove the database"),
                true,
                cx.listener(move |_, _, _, cx| hub(cx).update(cx, |h, cx| h.remove_source_download(&remove_id, cx))),
            ));
        col = col
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(theme::text_muted())
                    .child(icon(Icon::Check, px(14.), theme::readable(Hsla::from(theme::success()))))
                    .child(facts.join(" \u{b7} ")),
            )
            .child(buttons);
    } else {
        let sid = id.to_string();
        let size = mb(u64::from(row.download.size_mb) * 1_000_000);
        col = col
            .child(div().flex().child(icon_action_button(
                SharedString::from(format!("md-dl-start-{id}")),
                ButtonKind::Primary,
                Icon::Download,
                tr!("Download the database ({size})", size),
                cx.listener(move |_, _, _, cx| hub(cx).update(cx, |h, cx| h.download_source(&sid, cx))),
            )))
            .child(meta_text(tr!(
                "Nothing is downloaded before you ask. The database stays on this PC and is refreshed {every} while this source is on.",
                every = every(row.download.refresh_days)
            )));
    }
    if let Some(error) = ui.error {
        col = col.child(
            div()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme::danger_text())
                .child(tr!("The last download failed: {error}", error)),
        );
    }
    col
}
