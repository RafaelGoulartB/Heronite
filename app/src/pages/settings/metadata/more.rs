//! The dashed row under the sources: "Get more sources" opens the gallery,
//! and says how many more the lists offer once a copy of them was read
//! ("Get more sources · 2 more from the Heronite list"); "Install from a
//! file…" beside it goes straight to the file chooser.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::super::SettingsPage;
use super::extension::pick_file;
use heronite_metadata_ext::lists::{host_of, is_local, HERONITE_LIST};

use crate::dialogs::extensions::SourcesRequest;
use crate::dialogs::metadata_wizard::{hub, HubGlobal};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{icon_action_button, ButtonKind};
use crate::ui::icon;

/// "2 more from the Heronite list", "5 more from your lists", "2 more from
/// this copy of the repository" (a debug build's own list), or nothing
/// without a copy of a list.
fn count_line(cx: &App) -> Option<String> {
    if !cx.has_global::<HubGlobal>() {
        return None;
    }
    let ext = &hub(cx).read(cx).extensions;
    let (n, single) = ext.snapshot.more()?;
    Some(match single {
        Some(copy) if ext.is_checkout(&copy.url) => {
            trn!(n, "{n} more from this copy of the repository", "{n} more from this copy of the repository")
        }
        Some(copy) if is_local(&copy.url) => trn!(n, "{n} more from a list on this computer", "{n} more from a list on this computer"),
        Some(copy) if copy.builtin && copy.url == HERONITE_LIST => {
            trn!(n, "{n} more from the Heronite list", "{n} more from the Heronite list")
        }
        Some(copy) => trn!(n, "{n} more from {host}", "{n} more from {host}", host = host_of(&copy.url)),
        None if n == 0 => tr!("Nothing new on your lists").into(),
        None => trn!(n, "{n} more from your lists", "{n} more from your lists"),
    })
}

pub(super) fn row(cx: &mut Context<SettingsPage>) -> Div {
    let title = tr!("Get more sources");
    let line = count_line(cx);
    let label = div()
        .relative()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .children(crate::ui::find::flash(title, 8.))
        .child(div().flex_none().text_size(px(14.)).text_color(theme::text_primary()).child(title))
        .children(line.map(|l| div().min_w_0().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(format!("\u{b7} {l}"))));
    let open = div()
        .id("md-more")
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(12.))
        .cursor_pointer()
        .on_click(|_, _, cx| {
            if cx.has_global::<HubGlobal>() {
                hub(cx).update(cx, |h, cx| h.request_sources(SourcesRequest::Gallery { pick: None }, cx));
            }
        })
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded(px(9.))
                .bg(theme::accent_alpha(0.12))
                .child(icon(Icon::Add, px(18.), theme::readable(theme::accent()))),
        )
        .child(label);
    div()
        .mt(px(14.))
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(14.))
        .py(px(10.))
        .rounded(px(theme::R_CARD))
        .border_1()
        .border_dashed()
        .border_color(theme::ink_a(0.18))
        .hover(|s| s.bg(theme::ink_a(0.03)))
        .child(open)
        .child(icon_action_button(
            "md-more-file",
            ButtonKind::Ghost,
            Icon::FolderOpen,
            tr!("Install from a file\u{2026}"),
            cx.listener(|_, _, window, cx| pick_file(window, cx)),
        ))
}
