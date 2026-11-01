//! Settings → Sync: the library synced between PCs (`heronite_sync`, run by
//! `services::sync`): where (a shared folder, a cloud account or a server
//! through rclone), connecting the account, this PC's name, "Sync now", how
//! the last sync went and the other PCs found.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_sync::prefs::{self, LastSummary, WEBDAV_VENDORS};
use heronite_sync::{Provider, Server, SyncPrefs};

use super::SettingsPage;
use crate::pages::{status_pill, PageCtx};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card_box, row, toggle_row, ButtonKind};
use crate::ui::fields::{choose_path, input_box, labeled, meta_text, new_input, on_commit, path_field, select_field, text_field};

pub struct State {
    /// `sync.json` as last read or written here (`None` until read).
    prefs: Option<SyncPrefs>,
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
    /// A server's form was not filled in, or a write failed.
    error: Option<SharedString>,
    /// Whether the service was busy at its last change.
    was_busy: bool,
}

impl State {
    pub fn new(_ctx: &PageCtx, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let folder = new_input(window, cx, "", "");
        on_commit(&folder, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.sync.prefs.as_ref().is_some_and(|p| p.folder != value) {
                persist(d, move |p| p.folder = value, cx);
                sync_now(cx);
            }
        });
        let remote_folder = new_input(window, cx, "", prefs::DEFAULT_REMOTE_FOLDER);
        on_commit(&remote_folder, window, cx, |d: &mut SettingsPage, value, _, cx| {
            let value = if value.is_empty() { prefs::DEFAULT_REMOTE_FOLDER.to_owned() } else { value };
            if d.sync.prefs.as_ref().is_some_and(|p| p.remote_folder != value) {
                persist(d, move |p| p.remote_folder = value, cx);
            }
        });
        let custom_remote = new_input(window, cx, "", "mydrive:Heronite");
        on_commit(&custom_remote, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.sync.prefs.as_ref().is_some_and(|p| p.custom_remote != value) {
                persist(d, move |p| p.custom_remote = value, cx);
            }
        });
        let device_name = new_input(window, cx, "", &prefs::default_device_name());
        on_commit(&device_name, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.sync.prefs.as_ref().is_some_and(|p| p.device_name != value) {
                persist(d, move |p| p.device_name = value, cx);
            }
        });
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        if let Some(svc) = crate::services::sync::get(cx) {
            cx.observe(&svc, |d, svc, cx| {
                let busy = { svc.read(cx).running() || svc.read(cx).connecting().is_some() };
                if d.sync.was_busy && !busy {
                    reload(d, cx);
                }
                d.sync.was_busy = busy;
                cx.notify();
            })
            .detach();
        }
        Self {
            prefs: None,
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
            error: None,
            was_busy: false,
        }
    }
}

/// The section came on screen: reads `sync.json` (the fields take its
/// values the first time).
pub(super) fn shown(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let app = d.ctx.app.clone();
    cx.spawn_in(window, async move |this, cx| {
        let loaded = cx.background_spawn(async move { prefs::load(&app) }).await;
        this.update_in(cx, |d, window, cx| {
            let first = d.sync.prefs.is_none();
            took(d, loaded, cx);
            if first {
                fill_inputs(d, window, cx);
            }
        })
        .ok();
    })
    .detach();
}

/// Reads `sync.json` again (a sync or a connection ended).
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
        Ok(p) => d.sync.prefs = Some(p),
        Err(e) => d.sync.error = Some(format!("{e:#}").into()),
    }
    cx.notify();
}

/// The fields take the saved values (once, when first read).
fn fill_inputs(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(p) = d.sync.prefs.clone() else { return };
    let s = &d.sync;
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
            d.sync.prefs = Some(p);
            d.sync.error = None;
        }
        Err(e) => d.sync.error = Some(format!("{e:#}").into()),
    }
    cx.notify();
}

fn sync_now(cx: &mut App) {
    if let Some(svc) = crate::services::sync::get(cx) {
        svc.update(cx, |s, cx| s.sync(cx));
    }
}

fn provider_label(p: Provider) -> &'static str {
    match p {
        Provider::Folder => tr!("A shared folder"),
        Provider::Drive => "Google Drive",
        Provider::Dropbox => "Dropbox",
        Provider::OneDrive => "OneDrive",
        Provider::Box => "Box",
        Provider::Ftp => "FTP",
        Provider::Smb => tr!("SMB (Windows or NAS share)"),
        Provider::WebDav => "WebDAV",
        Provider::Custom => tr!("A remote of your rclone settings"),
    }
}

fn os_label(os: &str) -> &str {
    match os {
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    }
}

fn connect(d: &mut SettingsPage, provider: Provider, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let value = |i: &Entity<InputState>, cx: &App| i.read(cx).value().trim().to_string();
    let server = Server {
        host: value(&d.sync.host, cx),
        port: value(&d.sync.port, cx),
        user: value(&d.sync.user, cx),
        password: d.sync.password.read(cx).value().to_string(),
        vendor: WEBDAV_VENDORS[d.sync.vendor].0.to_owned(),
    };
    if provider.is_server() && server.host.is_empty() {
        d.sync.error =
            Some(if provider == Provider::WebDav { tr!("Type the server's address.") } else { tr!("Type the server's name.") }.into());
        cx.notify();
        return;
    }
    d.sync.error = None;
    // The password goes to rclone's settings, obscured; the field forgets it.
    d.sync.password.update(cx, |i, cx| i.set_value("", window, cx));
    if let Some(svc) = crate::services::sync::get(cx) {
        svc.update(cx, |s, cx| s.connect(provider, server, cx));
    }
}

fn render_where(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let options: Vec<(SharedString, bool)> = Provider::ALL.iter().map(|o| (provider_label(*o).into(), *o == p.provider)).collect();
    let select = select_field(
        "sync-provider",
        Some(tr!("Sync through")),
        provider_label(p.provider),
        false,
        d.sync.provider_open,
        options,
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.sync.provider_open = !d.sync.provider_open;
            d.sync.vendor_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.sync.provider_open = false;
            let provider = Provider::ALL[i];
            persist(d, move |p| p.provider = provider, cx);
        },
    );
    let body = match p.provider {
        Provider::Folder => render_folder(d, cx),
        Provider::Custom => div().flex().flex_col().gap(px(6.)).child(text_field(tr!("rclone remote"), &d.sync.custom_remote)).child(
            meta_text(tr!("A remote of your own rclone settings and a folder in it, like mydrive:Heronite. Set it up with rclone config.")),
        ),
        provider => render_account(d, p, provider, cx),
    };
    div().flex().flex_col().gap(px(12.)).child(select).child(body)
}

fn render_folder(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let has = !d.sync.folder.read(cx).value().trim().is_empty();
    let field = path_field(
        "sync-folder-pick",
        Some(tr!("Sync folder")),
        &d.sync.folder,
        has,
        false,
        cx.listener(move |d, _, window, cx| {
            if has {
                d.sync.folder.update(cx, |s, cx| s.set_value("", window, cx));
                persist(d, |p| p.folder.clear(), cx);
            } else {
                choose_path(window, cx, true, tr!("Sync folder"), |d: &mut SettingsPage, path, window, cx| {
                    d.sync.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                    persist(d, move |p| p.folder = path, cx);
                    sync_now(cx);
                });
            }
        }),
    );
    div().flex().flex_col().gap(px(6.)).child(field).child(meta_text(tr!(
        "A folder every PC reaches: a drive both systems of a dual boot mount, or a folder Syncthing, Dropbox or OneDrive keep in sync. On a dual boot, use an NTFS or exFAT drive and turn off Fast Startup in Windows."
    )))
}

/// A cloud account or a server: connected, being connected, or its form.
fn render_account(d: &mut SettingsPage, p: &SyncPrefs, provider: Provider, cx: &mut Context<SettingsPage>) -> Div {
    let svc = crate::services::sync::get(cx);
    let (connecting, link, connect_error) = svc
        .as_ref()
        .map(|s| {
            let s = s.read(cx);
            let c = s.connecting().filter(|c| c.provider == provider);
            (c.is_some(), c.and_then(|c| c.link.clone()), s.connect_error().map(str::to_owned))
        })
        .unwrap_or_default();
    let name = provider_label(provider);
    let account: Div = if p.connected == Some(provider) {
        let who = if p.account.is_empty() { name.to_owned() } else { p.account.clone() };
        row(
            who,
            Some(tr!("Connected")),
            button_sm("sync-disconnect", ButtonKind::Ghost, tr!("Disconnect")).on_click(cx.listener(|_, _, _, cx| {
                if let Some(svc) = crate::services::sync::get(cx) {
                    svc.update(cx, |s, cx| s.disconnect(cx));
                }
            })),
        )
    } else if connecting {
        let waiting = if provider.is_oauth() { tr!("Waiting for you to sign in in the browser…") } else { tr!("Connecting…") };
        let buttons = div()
            .flex()
            .gap(px(8.))
            .children(link.map(|url| {
                button_sm("sync-open-link", ButtonKind::Glass, tr!("Open the page")).on_click(move |_, _, _| crate::ui::open_external(&url))
            }))
            .child(button_sm("sync-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|_, _, _, cx| {
                if let Some(svc) = crate::services::sync::get(cx) {
                    svc.update(cx, |s, cx| s.cancel_connect(cx));
                }
            })));
        row(name, Some(waiting), buttons)
    } else if provider.is_oauth() {
        row(
            name,
            Some(tr!("Not connected. Your browser opens to sign in; rclone is downloaded first if it is not installed.")),
            button_sm("sync-connect", ButtonKind::Primary, tr!("Connect"))
                .on_click(cx.listener(move |d, _, window, cx| connect(d, provider, window, cx))),
        )
    } else {
        render_server_form(d, provider, cx)
    };
    let error = connect_error.or(d.sync.error.as_ref().map(|e| e.to_string()));
    let drive_note = (provider == Provider::Drive).then(|| meta_text(tr!("Heronite only sees the files it makes in your Drive.")));
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(account)
        .children(error.map(|e| div().text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
        .child(text_field(tr!("Folder in the account"), &d.sync.remote_folder))
        .children(drive_note)
}

fn render_server_form(d: &mut SettingsPage, provider: Provider, cx: &mut Context<SettingsPage>) -> Div {
    let s = &d.sync;
    let host_label = if provider == Provider::WebDav { tr!("Address") } else { tr!("Server") };
    let mut first = div().flex().gap(px(12.)).child(div().flex_1().child(labeled(host_label, input_box(&s.host, false))));
    if provider == Provider::WebDav {
        let vendor = select_field(
            "sync-webdav-vendor",
            Some(tr!("Server type")),
            WEBDAV_VENDORS[s.vendor].1,
            false,
            s.vendor_open,
            WEBDAV_VENDORS.iter().enumerate().map(|(i, (_, l))| (SharedString::from(*l), i == s.vendor)).collect(),
            cx,
            |d: &mut SettingsPage, _, cx| {
                d.sync.vendor_open = !d.sync.vendor_open;
                d.sync.provider_open = false;
                cx.notify();
            },
            |d: &mut SettingsPage, i, _, cx| {
                d.sync.vendor_open = false;
                d.sync.vendor = i;
                cx.notify();
            },
        );
        first = first.child(div().w(px(200.)).child(vendor));
    } else {
        first = first.child(div().w(px(110.)).child(labeled(tr!("Port"), input_box(&s.port, false))));
    }
    let s = &d.sync;
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(first)
        .child(
            div()
                .flex()
                .gap(px(12.))
                .child(div().flex_1().child(labeled(tr!("Username"), input_box(&s.user, false))))
                .child(div().flex_1().child(labeled(tr!("Password"), input_box(&s.password, false)))),
        )
        .child(meta_text(tr!("The password is kept obscured in Heronite's own rclone settings, readable only by your user.")))
        .child(
            div().flex().child(
                button_sm("sync-connect", ButtonKind::Primary, tr!("Connect"))
                    .on_click(cx.listener(move |d, _, window, cx| connect(d, provider, window, cx))),
            ),
        )
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

fn render_status(p: &SyncPrefs, busy: bool, cx: &mut Context<SettingsPage>) -> Div {
    let last = match &p.last_sync_at {
        Some(at) => super::format_time(at),
        None => tr!("Never").into(),
    };
    let action = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .mt(px(12.))
        .child(action_button(
            "sync-now",
            ButtonKind::Primary,
            if busy { tr!("Syncing…") } else { tr!("Sync now") },
            !busy && p.enabled && p.ready(),
            cx.listener(|_, _, _, cx| sync_now(cx)),
        ))
        .child(meta_text(tr!("Last sync: {last}", last)));
    let (lines, error): (Vec<String>, bool) = match (&p.last_error, &p.last_summary) {
        (Some(e), _) => (vec![e.clone()], true),
        (None, Some(s)) if p.last_sync_at.is_some() => (summary_lines(s), false),
        _ => (Vec::new(), false),
    };
    div().flex().flex_col().child(action).children(lines.into_iter().map(move |text| {
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
    let Some(p) = d.sync.prefs.clone() else {
        return meta_text(tr!("Loading…")).into_any_element();
    };
    let busy = crate::services::sync::get(cx).is_some_and(|s| s.read(cx).running());
    let on = toggle_row(
        "sync-on",
        tr!("Sync this PC"),
        Some(tr!("When Heronite starts, after each game and every 10 minutes, never during a game.")),
        p.enabled,
    )
    .on_click(cx.listener(move |d, _, _, cx| {
        let enabled = !p.enabled;
        persist(d, move |p| p.enabled = enabled, cx);
        if enabled {
            sync_now(cx);
        }
    }));
    let p = d.sync.prefs.clone().unwrap_or_default();
    let separator = || div().h(px(1.)).w_full().bg(theme::row_separator());
    let setup = card_box()
        .py(px(12.))
        .gap(px(12.))
        .child(on)
        .child(separator())
        .child(render_where(d, &p, cx))
        .child(separator())
        .child(text_field(tr!("This PC's name"), &d.sync.device_name));
    let ready = (p.enabled && !p.ready()).then(|| status_pill(tr!("Choose where to sync"), theme::status_color("on-hold")));
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Library sync")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Keep the same Collection on every PC you play on: Linux and Windows on a dual boot, or other computers. Each PC writes only its own file in the folder or account, so nothing is overwritten, and a game from a store signed in on one PC only keeps its data on the others."
        ))))
        .child(setup)
        .children(ready.map(|pill| div().mt(px(10.)).mx(px(6.)).flex().child(pill)))
        .child(render_status(&p, busy, cx))
        .child(div().mt(px(18.)).child(render_devices(&p)))
        .child(div().mt(px(18.)).child(caption(tr!("What syncs"))))
        .child(div().px(px(6.)).child(meta_text(tr!(
            "Statuses, favourites, hidden games, ratings, notes, play time and play sessions. Installed games, Wine prefixes, game settings and store sign-ins stay on each PC. Before a sync changes this Collection, the files it changes are copied to local_library/sync-before in the data folder."
        ))))
        .into_any_element()
}
