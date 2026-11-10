//! What an extension's open row adds to a source's: why it cannot be asked
//! (with "Try again" or "Remove"), who made it, where it came from, where
//! it connects, its homepage, an update its list offers, and "Update from a
//! file…" and "Remove" (asked inline first). Removing keeps what the user
//! set for it, for a later install (`ExtensionsExt::remove_extension`
//! without `forget_settings`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::sources::{SourceRow, SourceStatus};
use heronite_metadata_ext::ExtensionsExt;

use super::super::SettingsPage;
use super::{service, set_enabled};
use crate::dialogs::extensions::{words, SourcesRequest};
use crate::dialogs::metadata_wizard::{hub, HubGlobal};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, icon_action_button, ButtonKind};
use crate::ui::fields::{choose_path, meta_text};
use crate::ui::icon;

/// The small "Extension" pill and the version, after an extension's name.
pub(super) fn pill(r: &SourceRow) -> Option<Div> {
    let ext = r.descriptor.extension.as_ref()?;
    Some(
        div()
            .flex_none()
            .px(px(7.))
            .py(px(1.))
            .rounded_full()
            .bg(theme::ink_a(0.07))
            .text_size(px(11.))
            .text_color(theme::text_muted())
            .whitespace_nowrap()
            .child(if ext.version.is_empty() {
                tr!("Extension").to_string()
            } else {
                tr!("Extension \u{b7} {version}", version = ext.version.clone())
            }),
    )
}

/// Why the source cannot be asked, in plain words, and the way out; shown
/// first in the open row.
pub(super) fn problem_block(r: &SourceRow, cx: &mut Context<SettingsPage>) -> Option<Div> {
    let why = r.problem.clone()?;
    let id = r.descriptor.id.clone();
    let stuck = heronite_metadata_ext::source::is_stuck(&why);
    let (text, action) = match r.status {
        SourceStatus::LoadFailed if stuck => (
            tr!("Its script ran past its time limit while loading and could not be stopped, so it is not loaded again. Install a version that works, or remove it."),
            None,
        ),
        SourceStatus::LoadFailed => {
            (tr!("Its files could not be loaded, so it is never asked. Install a version that works, or remove it."), None)
        }
        _ => (
            if stuck {
                tr!(
                    "It ran past its time limit and could not be stopped, so it is not asked for now. Restarting Heronite ends what it left running; it is asked again once you try again or install an update."
                )
            } else {
                tr!(
                    "It failed too many times in a row, so it is not asked for now. It is asked again once you try again or install an update."
                )
            },
            Some(button(SharedString::from(format!("md-ext-retry-{id}")), ButtonKind::Glass, tr!("Try again")).on_click(cx.listener(
                move |d, _, _, cx| {
                    set_enabled(d, &id, true, cx);
                },
            ))),
        ),
    };
    Some(
        div()
            .flex()
            .items_start()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(12.))
            .bg(Hsla { a: 0.08, ..theme::danger().into() })
            .child(div().pt(px(1.)).child(icon(Icon::Warning, px(16.), theme::danger_text())))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(text))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .line_height(px(15.))
                            .text_color(theme::text_dim())
                            .child(tr!("The last error: {why}", why)),
                    ),
            )
            .children(action),
    )
}

/// Who made it, where it came from and where it connects.
fn facts(r: &SourceRow, cx: &App) -> Div {
    let ext = r.descriptor.extension.as_ref();
    let author = ext.map(|e| e.author.clone()).unwrap_or_default();
    let (origin, hosts) = if cx.has_global::<HubGlobal>() {
        let h = hub(cx);
        let state = &h.read(cx).extensions;
        let installed = state.snapshot.installed(&r.descriptor.id);
        (
            installed.map(|_| state.snapshot.origin(&state.lists, &r.descriptor.id)),
            installed.map(|i| i.hosts.iter().chain(&i.image_hosts).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        )
    } else {
        (None, Vec::new())
    };
    let text = |t: String, c: Hsla| div().text_size(px(12.5)).line_height(px(17.)).text_color(c).child(t);
    let muted: Hsla = theme::text_muted().into();
    let mut who = div().flex().flex_wrap().gap_x(px(6.));
    if !author.is_empty() {
        who = who.child(text(tr!("Made by {author}", author), muted));
    }
    if let Some(o) = &origin {
        let c = if words::cautious(o) { theme::readable(theme::warning()) } else { muted };
        who = who.when(!author.is_empty(), |w| w.child(text("\u{b7}".into(), muted))).child(text(words::origin(o), c));
    }
    // A manifest that does not read says nothing about where it connects.
    let unread = r.status == SourceStatus::LoadFailed && hosts.is_empty();
    let hosts = if hosts.is_empty() {
        tr!("Connects to nothing on the internet").into()
    } else {
        tr!("Connects to: {hosts}", hosts = hosts.join(", "))
    };
    let homepage = ext.and_then(|e| e.homepage.clone()).map(|link| {
        let shown = link.clone();
        div()
            .id(SharedString::from(format!("md-ext-home-{}", r.descriptor.id)))
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.5))
            .text_color(theme::readable(theme::accent()))
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .on_click(move |_, _, _| crate::ui::open_external(&link))
            .child(shown)
            .child(icon(Icon::OpenInNew, px(13.), theme::readable(theme::accent())))
    });
    div().flex().flex_col().gap(px(3.)).child(who).when(!unread, |d| d.child(text(hosts, muted))).children(homepage)
}

/// The update the extension's list offers, if any.
fn update(r: &SourceRow, cx: &mut Context<SettingsPage>) -> Option<Div> {
    if !cx.has_global::<HubGlobal>() {
        return None;
    }
    let offered = hub(cx).read(cx).extensions.snapshot.update_for(&r.descriptor.id)?.offered.to_string();
    let id = r.descriptor.id.clone();
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(icon(Icon::Update, px(16.), theme::readable(theme::accent())))
            .child(meta_text(tr!("Version {version} is on its list.", version = offered.clone())))
            .child(button(SharedString::from(format!("md-ext-update-{id}")), ButtonKind::Glass, tr!("See the update\u{2026}")).on_click(
                move |_, _, cx| {
                    if cx.has_global::<HubGlobal>() {
                        hub(cx).update(cx, |h, cx| h.request_sources(SourcesRequest::Gallery { pick: Some(id.clone()) }, cx));
                    }
                },
            )),
    )
}

/// "Update from a file…" and "Remove", or the question before removing.
fn actions(d: &SettingsPage, r: &SourceRow, cx: &mut Context<SettingsPage>) -> Div {
    let id = r.descriptor.id.clone();
    let name = r.descriptor.name.clone();
    if d.metadata.removing.as_deref() == Some(id.as_str()) {
        let busy = d.metadata.ext_busy;
        let (rid, cancel_id) = (id.clone(), id.clone());
        return div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(12.))
            .bg(theme::ink_a(0.05))
            .child(div().flex_1().min_w(px(240.)).text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(tr!(
                "Remove {name}? Its files are deleted and it is no longer asked. What you set for it stays, in case you install it again.",
                name
            )))
            .child(action_button(
                SharedString::from(format!("md-ext-remove-no-{id}")),
                ButtonKind::Ghost,
                tr!("Cancel"),
                !busy,
                cx.listener(move |d, _, _, cx| {
                    if d.metadata.removing.as_deref() == Some(cancel_id.as_str()) {
                        d.metadata.removing = None;
                    }
                    cx.notify();
                }),
            ))
            .child(action_button(
                SharedString::from(format!("md-ext-remove-yes-{id}")),
                ButtonKind::Danger,
                if busy { tr!("Removing\u{2026}") } else { tr!("Remove") },
                !busy,
                cx.listener(move |d, _, _, cx| remove(d, &rid, cx)),
            ));
    }
    let rid = id.clone();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .child(icon_action_button(
            SharedString::from(format!("md-ext-file-{id}")),
            ButtonKind::Glass,
            Icon::FolderOpen,
            tr!("Update from a file\u{2026}"),
            cx.listener(|_, _, window, cx| pick_file(window, cx)),
        ))
        .child(button(SharedString::from(format!("md-ext-remove-{id}")), ButtonKind::Ghost, tr!("Remove")).on_click(cx.listener(
            move |d, _, _, cx| {
                d.metadata.removing = Some(rid.clone());
                d.metadata.ext_error = None;
                cx.notify();
            },
        )))
        .children(d.metadata.ext_error.clone().map(|e| div().text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
}

/// The extension's part of its open row.
pub(super) fn block(d: &SettingsPage, r: &SourceRow, cx: &mut Context<SettingsPage>) -> Option<Div> {
    r.descriptor.extension.as_ref()?;
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .pt(px(4.))
            .border_t_1()
            .border_color(theme::row_separator())
            .child(div().h(px(4.)))
            .child(facts(r, cx))
            .children(update(r, cx))
            .child(actions(d, r, cx)),
    )
}

/// The file chooser, then the installer (it says when it updates one).
pub(super) fn pick_file(window: &mut Window, cx: &mut Context<SettingsPage>) {
    choose_path(
        window,
        cx,
        false,
        tr!("Choose an extension: its .zip, or the source.toml in its folder"),
        |_: &mut SettingsPage, p, _, cx| {
            if cx.has_global::<HubGlobal>() {
                hub(cx).update(cx, |h, cx| h.request_sources(SourcesRequest::File(p.into()), cx));
            }
        },
    );
}

/// Removes extension `id` off the UI thread, keeping what the user set.
fn remove(d: &mut SettingsPage, id: &str, cx: &mut Context<SettingsPage>) {
    let Some(svc) = service(cx) else { return };
    d.metadata.ext_busy = true;
    let id = id.to_string();
    cx.spawn(async move |this, cx| {
        let target = id.clone();
        let done = cx.background_spawn(async move { svc.remove_extension(&target, false) }).await;
        this.update(cx, |d, cx| {
            d.metadata.ext_busy = false;
            d.metadata.removing = None;
            match done {
                Ok(()) => {
                    d.metadata.open.remove(&id);
                    d.metadata.ext_error = None;
                }
                Err(e) => d.metadata.ext_error = Some(words::error(&e)),
            }
            d.metadata.stale = true;
            if cx.has_global::<HubGlobal>() {
                hub(cx).update(cx, |h, cx| h.read_extensions(false, cx));
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}
