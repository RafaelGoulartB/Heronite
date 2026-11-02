//! Settings → Cloud: the card at the top. Before an account is connected,
//! what the cloud does and the three steps to it, the first one lit; after,
//! where things go, how much is up (one line for everything: all sent, what
//! is not, the upload running with Stop, or why uploads wait), "Account"
//! (the page unfolds the account under it) and "Send everything now".

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_sync::cloud::Upload;
use heronite_sync::sent::Status;
use heronite_sync::usage::Kind;
use heronite_sync::{Provider, SyncPrefs};

use super::{account, features, space};
use crate::icons::Icon;
use crate::pages::settings::SettingsPage;
use crate::services::widgets::progress_bar;
use crate::theme;
use crate::ui::controls::{action_button, button_sm, card_box, icon_button_sm, ButtonKind};
use crate::ui::icon;

/// Whether the kind goes to the cloud.
fn is_on(kind: Kind, p: &SyncPrefs) -> bool {
    match kind {
        Kind::Library => p.library.is_on(),
        Kind::Backups => p.backups.is_on(),
        Kind::Saves => p.saves.is_on(),
        Kind::Captures => p.captures.is_on(),
    }
}

fn dot(color: Hsla) -> Div {
    div().flex_none().size(px(9.)).rounded_full().bg(color)
}

/// One step of the intro: a number and what to do; `now` lights the one to
/// do now (the first: the intro shows until an account is connected).
fn step(n: &str, text: &str, now: bool) -> Div {
    let number = div()
        .flex_none()
        .size(px(24.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .font_weight(theme::W_BOLD)
        .map(|b| {
            if now {
                b.bg(theme::accent()).text_color(theme::on_accent())
            } else {
                b.bg(theme::ink_a(0.08)).text_color(theme::text_muted())
            }
        });
    div()
        .flex()
        .flex_1()
        .min_w(px(180.))
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(12.))
        .bg(theme::recess_a(0.12))
        .border_1()
        .border_color(if now { theme::accent_alpha(0.7).into() } else { theme::card_border() })
        .child(number.child(n.to_owned()))
        .child(div().text_size(px(13.)).text_color(if now { theme::text_primary() } else { theme::text_muted() }).child(text.to_owned()))
}

fn intro() -> Div {
    card_box()
        .py(px(18.))
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(icon(Icon::CloudSync, px(26.), theme::accent()))
                .child(div().text_size(px(17.)).font_weight(theme::W_BOLD).text_color(theme::text_bright()).child(tr!("Your library, in the cloud"))),
        )
        .child(div().text_size(px(13.)).line_height(px(19.)).text_color(theme::text_muted()).child(tr!(
            "Keep the same library on every PC you play on, and a copy of your Heronite backups, game saves and screenshots. Each PC writes only its own files, so nothing is overwritten."
        )))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(10.))
                .child(step("1", tr!("Choose where and connect"), true))
                .child(step("2", tr!("Choose what goes, and when"), false))
                .child(step("3", tr!("Heronite does the rest"), false)),
        )
}

/// One line for everything: the upload running, a wait, or how much is up.
fn status_line(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let svc = crate::services::sync::get(cx);
    let svc_state = svc.as_ref().map(|s| {
        let s = s.read(cx);
        let retry = Upload::ALL.iter().filter_map(|u| s.retry_at(*u)).filter(|at| *at > std::time::SystemTime::now()).min();
        (s.running(), s.uploading().map(|u| (u.job.upload(), u.percent)), s.queued(), retry)
    });
    let (syncing, uploading, queued, retry) = svc_state.unwrap_or_default();
    let text_line = |color: Hsla, text: String| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(dot(color))
            .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(text))
    };
    let accent: Hsla = theme::accent().into();
    if let Some((upload, percent)) = uploading {
        let what = tr!("Uploading {what}…", what = features::upload_what(upload));
        let label = match percent {
            Some(n) => format!("{what} {n}%"),
            None => what,
        };
        return div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div().flex().items_center().justify_between().gap(px(12.)).child(text_line(accent, label)).child(
                    button_sm("cloud-stop", ButtonKind::Ghost, tr!("Stop"))
                        .on_click(cx.listener(|_, _, _, cx| features::with_service(cx, |s, cx| s.stop_uploads(cx)))),
                ),
            )
            .child(progress_bar(percent.map(f32::from), 6., theme::accent()));
    }
    if syncing {
        return text_line(accent, tr!("Syncing the library…").into());
    }
    let amber: Hsla = theme::status_color("on-hold").into();
    if queued > 0 && crate::services::session::any_running(cx) {
        return text_line(amber, tr!("Uploads wait for your game to end, so they do not take its bandwidth.").into());
    }
    if let Some(at) = retry {
        let at: chrono::DateTime<chrono::Local> = at.into();
        return text_line(amber, tr!("The account is refusing requests for now: trying again at {time}.", time = at.format("%H:%M")));
    }
    let on: Vec<Kind> = Kind::ALL.into_iter().filter(|k| is_on(*k, p)).collect();
    if on.is_empty() {
        return text_line(theme::text_dim().into(), tr!("Nothing goes to the cloud yet: choose what goes below.").into());
    }
    let Some(usage) = d.cloud.usage.as_ref() else {
        return text_line(theme::text_dim().into(), tr!("Checking what is in the cloud…").into());
    };
    let missing: Vec<(Kind, Status)> = on
        .into_iter()
        .filter_map(|k| usage.status.get(&k).map(|s| (k, *s)))
        .filter(|(_, s)| !matches!(s, Status::Done | Status::Empty))
        .collect();
    if missing.is_empty() {
        return text_line(theme::status_color("playing").into(), tr!("Everything is in the cloud.").into());
    }
    let none_sent = missing.iter().all(|(_, s)| *s == Status::Nothing);
    let list = missing.iter().map(|(k, _)| space::kind_label(*k)).collect::<Vec<_>>().join(", ");
    let color: Hsla = if none_sent { theme::danger_text().into() } else { amber };
    text_line(color, tr!("Not in the cloud yet: {list}.", list))
}

fn connected(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let place = account::provider_label(p.provider);
    let title =
        if p.provider == Provider::Folder { tr!("Syncing through a shared folder").to_owned() } else { tr!("Connected to {place}", place) };
    let detail = match p.provider {
        Provider::Folder => p.folder.trim().to_owned(),
        Provider::Custom => p.custom_remote.trim().to_owned(),
        _ if p.account.is_empty() => tr!("In the folder {folder}", folder = p.remote_folder.trim()),
        _ => format!("{} \u{b7} {}", p.account, tr!("In the folder {folder}", folder = p.remote_folder.trim())),
    };
    let syncing = crate::services::sync::get(cx).is_some_and(|s| s.read(cx).running());
    let backup = features::job_of(d, Upload::Backups);
    let send_all = action_button(
        "cloud-now",
        ButtonKind::Primary,
        tr!("Send everything now"),
        !syncing && p.any_on(),
        cx.listener(move |_, _, _, cx| {
            let backup = backup.clone();
            features::with_service(cx, |s, cx| {
                s.sync(true, cx);
                s.upload_all(true, cx);
                if let Some(job) = backup {
                    s.upload(job, true, cx);
                }
            })
        }),
    );
    let open = d.cloud.account_open;
    let account = icon_button_sm(
        "cloud-account",
        if open { ButtonKind::Glass } else { ButtonKind::Ghost },
        if open { Icon::FaChevronUp } else { Icon::FaChevronDown },
        tr!("Account"),
        true,
        cx.listener(move |d, _, _, cx| {
            d.cloud.account_open = !open;
            cx.notify();
        }),
    );
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .child(
            div().flex().items_center().gap(px(12.)).min_w_0().child(icon(Icon::CloudSync, px(24.), theme::accent())).child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_bright()).child(title))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(detail)),
            ),
        )
        .child(div().flex().flex_none().items_center().gap(px(8.)).child(account).child(send_all));
    card_box().py(px(16.)).gap(px(14.)).child(head).child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(status_line(d, p, cx))
}

pub(super) fn render(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    if p.ready() {
        connected(d, p, cx)
    } else {
        intro()
    }
}
