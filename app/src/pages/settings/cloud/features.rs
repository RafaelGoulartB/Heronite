//! Settings → Cloud: what goes to the account (the library sync, backups,
//! game saves, screenshots), the upload running, "Sync now", bringing the
//! backups back, and the row the Backups, Saves and Screenshots sections
//! show to link here.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_sync::cloud::Upload;
use heronite_sync::SyncPrefs;

use super::{account, persist, sync_now};
use crate::pages::settings::{Section, SettingsPage};
use crate::services::sync::Job;
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card, card_box, row, toggle_row, with_sub_rows, ButtonKind};
use crate::ui::fields::meta_text;

fn with_service(cx: &mut App, f: impl FnOnce(&mut crate::services::sync::LibrarySync, &mut Context<crate::services::sync::LibrarySync>)) {
    if let Some(svc) = crate::services::sync::get(cx) {
        svc.update(cx, f);
    }
}

/// What is being sent, for "Uploading {what}…".
fn upload_what(upload: Upload) -> &'static str {
    match upload {
        Upload::Backups => tr!("the backup"),
        Upload::Saves => tr!("the game saves"),
        Upload::Captures => tr!("the screenshots"),
    }
}

/// The row's description: its upload running, the last one, or what it does.
fn upload_line(p: &SyncPrefs, upload: Upload, about: &str, cx: &App) -> String {
    let svc = crate::services::sync::get(cx);
    if let Some(u) = svc.as_ref().and_then(|s| s.read(cx).uploading().map(|u| (u.job.upload(), u.percent))) {
        if u.0 == upload {
            return match u.1 {
                Some(percent) => tr!("Uploading… {percent}%", percent),
                None => tr!("Uploading…").into(),
            };
        }
    }
    match p.uploads.get(upload.key()).filter(|_| upload.is_on(p)) {
        Some(last) if last.error.is_some() => tr!("The last upload failed: {error}", error = last.error.clone().unwrap_or_default()),
        Some(last) if last.at.is_some() => {
            tr!("Last upload: {when}", when = crate::pages::settings::format_time(last.at.as_deref().unwrap_or_default()))
        }
        _ => about.to_owned(),
    }
}

fn set_upload(d: &mut SettingsPage, upload: Upload, on: bool, cx: &mut Context<SettingsPage>) {
    persist(
        d,
        move |p| match upload {
            Upload::Backups => p.backups = on,
            Upload::Saves => p.saves = on,
            Upload::Captures => p.captures = on,
        },
        cx,
    );
    if !on {
        return;
    }
    // What is there already goes up now.
    let job = match upload {
        Upload::Backups => d.settings.backup.last_backup_path.clone().map(|p| Job::Backup(p.into())),
        Upload::Saves => Some(Job::Saves),
        Upload::Captures => Some(Job::Captures),
    };
    if let Some(job) = job {
        with_service(cx, |s, cx| s.upload(job, cx));
    }
}

fn library_row(p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> AnyElement {
    let about = match (&p.last_sync_at, p.enabled) {
        (Some(at), true) => format!(
            "{} \u{b7} {}",
            tr!("Last sync: {last}", last = crate::pages::settings::format_time(at)),
            trn!(p.devices.len(), "{n} other PC", "{n} other PCs")
        ),
        _ => tr!("Statuses, favourites, notes, play time and sessions, the same on every PC.").into(),
    };
    let on = p.enabled;
    let parent = toggle_row("cloud-library", tr!("Library between PCs"), Some(&about), on)
        .on_click(cx.listener(move |d, _, _, cx| {
            persist(d, move |p| p.enabled = !on, cx);
            if !on {
                sync_now(cx);
            }
        }))
        .into_any_element();
    let play_only = p.play_only;
    let sub = on.then(|| {
        toggle_row(
            "cloud-play-only",
            tr!("This PC only sends play time and sessions"),
            Some(tr!(
                "For a PC you do not keep organised: what you change here (statuses, favourites, notes) stays here and gives way to your other PCs, while its play still reaches them."
            )),
            play_only,
        )
        .on_click(cx.listener(move |d, _, _, cx| persist(d, move |p| p.play_only = !play_only, cx)))
        .into_any_element()
    });
    with_sub_rows(parent, sub.into_iter().collect())
}

fn backups_row(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> AnyElement {
    let about = if d.settings.backup.folder.trim().is_empty() {
        tr!("Backups are off: choose a backup folder in Settings → Backups.")
    } else {
        tr!("Each backup goes up as one zip when it is made, your artwork next to it once. Store sign-ins stay on this PC.")
    };
    let on = p.backups;
    let parent = toggle_row("cloud-backups", tr!("Heronite backups"), Some(&upload_line(p, Upload::Backups, about, cx)), on)
        .on_click(cx.listener(move |d, _, _, cx| set_upload(d, Upload::Backups, !on, cx)))
        .into_any_element();
    let svc = crate::services::sync::get(cx);
    let (fetching, fetched) = svc.as_ref().map(|s| (s.read(cx).fetching(), s.read(cx).fetched().cloned())).unwrap_or_default();
    let result = fetched.map(|r| match r {
        Ok(0) => (tr!("No new backups in the cloud.").to_string(), false),
        Ok(n) => (
            trn!(
                n,
                "{n} backup brought in: restore it in Settings → Backups.",
                "{n} backups brought in: restore them in Settings → Backups."
            ),
            false,
        ),
        Err(e) => (e, true),
    });
    let fetch = row(
        tr!("Backups of your PCs in the cloud"),
        Some(tr!("Brings the newest backup of each PC to the backup folder, to restore it on this one.")),
        button_sm("cloud-fetch", ButtonKind::Glass, if fetching { tr!("Getting backups…") } else { tr!("Get backups") })
            .when(!fetching, |b| b.on_click(cx.listener(|_, _, _, cx| with_service(cx, |s, cx| s.fetch_backups(cx))))),
    );
    let fetch = div().flex().flex_col().child(fetch).children(result.map(|(text, error)| {
        div().pb(px(10.)).text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
    }));
    with_sub_rows(parent, if p.ready() { vec![fetch.into_any_element()] } else { Vec::new() })
}

fn saves_row(p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> AnyElement {
    let about = tr!("Ludusavi's backups go up after each game; only what changed is sent.");
    let on = p.saves;
    toggle_row("cloud-saves", tr!("Game saves (Ludusavi)"), Some(&upload_line(p, Upload::Saves, about, cx)), on)
        .on_click(cx.listener(move |d, _, _, cx| set_upload(d, Upload::Saves, !on, cx)))
        .into_any_element()
}

fn captures_row(p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> AnyElement {
    let about = tr!("Once compressed, one file each; only new ones go up. Deleting one here keeps it in the cloud.");
    let on = p.captures;
    let parent = toggle_row("cloud-captures", tr!("Screenshots"), Some(&upload_line(p, Upload::Captures, about, cx)), on)
        .on_click(cx.listener(move |d, _, _, cx| set_upload(d, Upload::Captures, !on, cx)))
        .into_any_element();
    let replays = p.replays;
    let sub = on.then(|| {
        toggle_row("cloud-replays", tr!("Include replays (videos)"), Some(tr!("About 20 MB each: they fill the account quickly.")), replays)
            .on_click(cx.listener(move |d, _, _, cx| {
                persist(d, move |p| p.replays = !replays, cx);
                if !replays {
                    with_service(cx, |s, cx| s.upload(Job::Captures, cx));
                }
            }))
            .into_any_element()
    });
    with_sub_rows(parent, sub.into_iter().collect())
}

/// "Sync now" and the upload running (with Stop), or why uploads wait.
fn render_activity(p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let svc = crate::services::sync::get(cx);
    let (syncing, uploading, queued) = svc
        .as_ref()
        .map(|s| {
            let s = s.read(cx);
            (s.running(), s.uploading().map(|u| (u.job.upload(), u.percent)), s.queued())
        })
        .unwrap_or_default();
    let now = action_button(
        "cloud-now",
        ButtonKind::Primary,
        if syncing { tr!("Syncing…") } else { tr!("Sync now") },
        !syncing && p.ready() && p.any_on(),
        cx.listener(|_, _, _, cx| {
            with_service(cx, |s, cx| {
                s.sync(cx);
                s.upload_all(cx);
            })
        }),
    );
    let mut line = div().flex().items_center().gap(px(12.)).mt(px(12.)).child(now);
    if let Some((upload, percent)) = uploading {
        let text = match percent {
            Some(n) => format!("{} \u{b7} {n}%", tr!("Uploading {what}…", what = upload_what(upload))),
            None => tr!("Uploading {what}…", what = upload_what(upload)),
        };
        line = line.child(meta_text(text)).child(
            button_sm("cloud-stop", ButtonKind::Ghost, tr!("Stop"))
                .on_click(cx.listener(|_, _, _, cx| with_service(cx, |s, cx| s.stop_uploads(cx)))),
        );
    } else if queued > 0 && crate::services::session::any_running(cx) {
        line = line.child(meta_text(tr!("Uploads wait for your game to end, so they do not take its bandwidth.")));
    }
    line
}

pub(super) fn render(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let rows = vec![library_row(p, cx), backups_row(d, p, cx), saves_row(p, cx), captures_row(p, cx)];
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("What goes to the cloud")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Uploads never run during a game. Installed games, Wine prefixes, game settings and store sign-ins stay on each PC."
        ))))
        .child(card(rows))
        .child(render_activity(p, cx))
}

/// The Cloud row of Backups, Saves and Screenshots: whether `upload` goes to
/// the account, and the way to Settings → Cloud.
pub(in crate::pages::settings) fn link_row(d: &SettingsPage, upload: Upload, cx: &mut Context<SettingsPage>) -> Div {
    let status = match &d.cloud.prefs {
        Some(p) if p.ready() && upload.is_on(p) => tr!("On \u{b7} {place}", place = account::provider_label(p.provider)),
        Some(p) if p.ready() => tr!("Off").into(),
        _ => tr!("No account connected").into(),
    };
    let id = SharedString::from(format!("cloud-link-{}", upload.key()));
    card_box().child(row(
        tr!("Cloud"),
        Some(&status),
        button_sm(id, ButtonKind::Glass, tr!("Cloud settings"))
            .on_click(cx.listener(|d, _, window, cx| d.select(Section::Cloud, window, cx))),
    ))
}
