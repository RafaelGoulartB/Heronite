//! Settings → Cloud: one place for everything that leaves this PC
//! (`heronite_sync`, run by `services::sync`). The account (`account.rs`: a
//! shared folder, a cloud account or a server through rclone, signed in
//! once), what goes there (`features.rs`: the library sync, backups, game
//! saves, screenshots), "Sync now", how the last sync went and the other PCs
//! found. Backups, Saves and Screenshots link here (`features::link_row`).

mod account;
mod features;

pub(super) use features::link_row;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_sync::prefs::{self, LastSummary};
use heronite_sync::SyncPrefs;

use super::SettingsPage;
use crate::pages::{status_pill, PageCtx};
use crate::theme;
use crate::ui::controls::{caption, card_box, row};
use crate::ui::fields::{meta_text, new_input, on_commit, text_field};

pub struct State {
    /// `sync.json` as last read or written here (`None` until read).
    prefs: Option<SyncPrefs>,
    /// Whether the fields took the saved values (the first time shown).
    filled: bool,
    folder: Entity<InputState>,
    remote_folder: Entity<InputState>,
    custom_remote: Entity<InputState>,
    device_name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    /// WebDAV server kind (`WEBDAV_VENDORS`).
    vendor: usize,
    provider_open: bool,
    vendor_open: bool,
    /// The account's space: (used, total) bytes, measured when shown.
    quota: Option<(u64, u64)>,
    /// A server's form was not filled in, or a write failed.
    error: Option<SharedString>,
    /// Whether the service was busy at its last change.
    was_busy: bool,
}

impl State {
    pub fn new(ctx: &PageCtx, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let folder = new_input(window, cx, "", "");
        on_commit(&folder, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.cloud.prefs.as_ref().is_some_and(|p| p.folder != value) {
                persist(d, move |p| p.folder = value, cx);
                sync_now(cx);
            }
        });
        let remote_folder = new_input(window, cx, "", prefs::DEFAULT_REMOTE_FOLDER);
        on_commit(&remote_folder, window, cx, |d: &mut SettingsPage, value, _, cx| {
            let value = if value.is_empty() { prefs::DEFAULT_REMOTE_FOLDER.to_owned() } else { value };
            if d.cloud.prefs.as_ref().is_some_and(|p| p.remote_folder != value) {
                persist(d, move |p| p.remote_folder = value, cx);
            }
        });
        let custom_remote = new_input(window, cx, "", "mydrive:Heronite");
        on_commit(&custom_remote, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.cloud.prefs.as_ref().is_some_and(|p| p.custom_remote != value) {
                persist(d, move |p| p.custom_remote = value, cx);
            }
        });
        let device_name = new_input(window, cx, "", &prefs::default_device_name());
        on_commit(&device_name, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.cloud.prefs.as_ref().is_some_and(|p| p.device_name != value) {
                persist(d, move |p| p.device_name = value, cx);
            }
        });
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        if let Some(svc) = crate::services::sync::get(cx) {
            cx.observe(&svc, |d, svc, cx| {
                let s = svc.read(cx);
                let busy = s.running() || s.connecting().is_some() || s.uploading().is_some() || s.fetching();
                if d.cloud.was_busy && !busy {
                    reload(d, cx);
                }
                d.cloud.was_busy = busy;
                cx.notify();
            })
            .detach();
        }
        // Backups, Saves and Screenshots say whether they go to the cloud.
        let app = ctx.app.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { prefs::load(&app) }).await;
            this.update(cx, |d, cx| took(d, loaded, cx)).ok();
        })
        .detach();
        Self {
            prefs: None,
            filled: false,
            folder,
            remote_folder,
            custom_remote,
            device_name,
            host: new_input(window, cx, "", ""),
            port: new_input(window, cx, "", ""),
            user: new_input(window, cx, "", ""),
            password,
            vendor: 0,
            provider_open: false,
            vendor_open: false,
            quota: None,
            error: None,
            was_busy: false,
        }
    }
}

/// The section came on screen: reads `sync.json` (the fields take its
/// values the first time) and the account's space.
pub(super) fn shown(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let app = d.ctx.app.clone();
    cx.spawn_in(window, async move |this, cx| {
        let loaded = cx.background_spawn(async move { prefs::load(&app) }).await;
        this.update_in(cx, |d, window, cx| {
            took(d, loaded, cx);
            if !d.cloud.filled {
                d.cloud.filled = true;
                fill_inputs(d, window, cx);
            }
        })
        .ok();
    })
    .detach();
    measure_quota(d, cx);
}

fn measure_quota(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let (data, app) = (d.ctx.data_paths.clone(), d.ctx.app.clone());
    cx.spawn(async move |this, cx| {
        let quota = cx.background_spawn(async move { heronite_sync::cloud::quota(&data, &app) }).await;
        this.update(cx, |d, cx| {
            d.cloud.quota = quota;
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Reads `sync.json` again (a sync, an upload or a connection ended).
fn reload(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let loaded = cx.background_spawn(async move { prefs::load(&app) }).await;
        this.update(cx, |d, cx| took(d, loaded, cx)).ok();
    })
    .detach();
}

fn took(d: &mut SettingsPage, loaded: anyhow::Result<SyncPrefs>, cx: &mut Context<SettingsPage>) {
    match loaded {
        Ok(p) => d.cloud.prefs = Some(p),
        Err(e) => d.cloud.error = Some(format!("{e:#}").into()),
    }
    cx.notify();
}

/// The fields take the saved values.
fn fill_inputs(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(p) = d.cloud.prefs.clone() else { return };
    let s = &d.cloud;
    for (input, value) in [
        (&s.folder, p.folder.clone()),
        (&s.remote_folder, p.remote_folder.clone()),
        (&s.custom_remote, p.custom_remote.clone()),
        (&s.device_name, p.device_name.clone()),
    ] {
        input.update(cx, |i, cx| i.set_value(value, window, cx));
    }
}

/// Changes `sync.json`.
fn persist(d: &mut SettingsPage, f: impl FnOnce(&mut SyncPrefs), cx: &mut Context<SettingsPage>) {
    match prefs::update(&d.ctx.app, f) {
        Ok(p) => {
            d.cloud.prefs = Some(p);
            d.cloud.error = None;
        }
        Err(e) => d.cloud.error = Some(format!("{e:#}").into()),
    }
    cx.notify();
}

fn sync_now(cx: &mut App) {
    if let Some(svc) = crate::services::sync::get(cx) {
        svc.update(cx, |s, cx| s.sync(cx));
    }
}

fn os_label(os: &str) -> &str {
    match os {
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    }
}

fn summary_lines(s: &LastSummary) -> Vec<String> {
    let mut lines = Vec::new();
    if s.fields + s.sessions + s.playtime == 0 {
        lines.push(tr!("Everything was already up to date.").to_string());
    } else {
        let mut parts = Vec::new();
        if s.sessions > 0 {
            parts.push(trn!(s.sessions, "{n} play session", "{n} play sessions"));
        }
        if s.playtime > 0 {
            parts.push(trn!(s.playtime, "{n} play time", "{n} play times"));
        }
        if s.fields > 0 {
            parts.push(trn!(s.fields, "{n} other change", "{n} other changes"));
        }
        lines.push(tr!("Brought in from your other PCs: {list}.", list = parts.join(", ")));
    }
    if s.waiting > 0 {
        lines.push(trn!(
            s.waiting,
            "{n} game of your other PCs is not in this Collection; its data comes in once it is.",
            "{n} games of your other PCs are not in this Collection; their data comes in once they are."
        ));
    }
    lines
}

/// How the last library sync went.
fn render_sync_result(p: &SyncPrefs) -> Div {
    let (lines, error): (Vec<String>, bool) = match (&p.last_error, &p.last_summary) {
        (Some(e), _) => (vec![e.clone()], true),
        (None, Some(s)) if p.last_sync_at.is_some() && p.enabled => (summary_lines(s), false),
        _ => (Vec::new(), false),
    };
    div().flex().flex_col().children(lines.into_iter().map(move |text| {
        div()
            .mt(px(8.))
            .mx(px(6.))
            .text_size(px(13.))
            .text_color(if error { theme::danger_text() } else { theme::text_muted() })
            .child(text)
    }))
}

fn render_devices(p: &SyncPrefs) -> Div {
    let col = div().flex().flex_col().child(caption(tr!("Your other PCs")));
    if p.devices.is_empty() {
        return col.child(
            div()
                .px(px(6.))
                .child(meta_text(tr!("None yet. Turn the sync on in Heronite on your other PCs, with the same folder or account."))),
        );
    }
    let mut card = card_box();
    for (i, dev) in p.devices.iter().enumerate() {
        if i > 0 {
            card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
        }
        let when = tr!("last synced {when}", when = super::format_time(&dev.written_at));
        let detail = format!("{} \u{b7} {when}", os_label(&dev.os));
        card = card.child(row(dev.name.clone(), Some(&detail), div()));
    }
    col.child(card)
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let Some(p) = d.cloud.prefs.clone() else {
        return meta_text(tr!("Loading…")).into_any_element();
    };
    let separator = || div().h(px(1.)).w_full().bg(theme::row_separator());
    let mut place = card_box().py(px(12.)).gap(px(12.)).child(account::render_where(d, &p, cx));
    if let Some((used, total)) = d.cloud.quota.filter(|_| p.ready()) {
        let human = heronite_data::maintenance::human_size;
        place = place.child(meta_text(tr!("{used} used of {total}", used = human(used), total = human(total))));
    }
    let place = place.child(separator()).child(text_field(tr!("This PC's name"), &d.cloud.device_name));
    let ready = (p.any_on() && !p.ready()).then(|| status_pill(tr!("Choose where to sync"), theme::status_color("on-hold")));
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Account")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Sign in once: the library sync, the backups, the game saves and the screenshots all use this account. Each PC writes only its own files, so nothing is overwritten."
        ))))
        .child(place)
        .children(ready.map(|pill| div().mt(px(10.)).mx(px(6.)).flex().child(pill)))
        .child(div().mt(px(18.)).child(features::render(d, &p, cx)))
        .child(render_sync_result(&p))
        .child(div().mt(px(18.)).child(render_devices(&p)))
        .into_any_element()
}
