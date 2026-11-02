//! Settings → Cloud: where things go (the service, connecting it) and its
//! advanced options (the folder in the account, a Google app of one's own).

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_sync::prefs::WEBDAV_VENDORS;
use heronite_sync::{Provider, Server, SyncPrefs};

use super::{persist, sync_now};
use crate::pages::settings::SettingsPage;
use crate::theme;
use crate::ui::controls::{button_sm, row, ButtonKind};
use crate::ui::fields::{choose_path, input_box, labeled, meta_text, path_field, select_field, text_field};

pub(super) fn provider_label(p: Provider) -> &'static str {
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

fn connect(d: &mut SettingsPage, provider: Provider, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let value = |i: &Entity<InputState>, cx: &App| i.read(cx).value().trim().to_string();
    let server = Server {
        host: value(&d.cloud.host, cx),
        port: value(&d.cloud.port, cx),
        user: value(&d.cloud.user, cx),
        password: d.cloud.password.read(cx).value().to_string(),
        vendor: WEBDAV_VENDORS[d.cloud.vendor].0.to_owned(),
        ..Default::default()
    };
    if provider.is_server() && server.host.is_empty() {
        d.cloud.error =
            Some(if provider == Provider::WebDav { tr!("Type the server's address.") } else { tr!("Type the server's name.") }.into());
        cx.notify();
        return;
    }
    d.cloud.error = None;
    // The password goes to rclone's settings, obscured; the field forgets it.
    d.cloud.password.update(cx, |i, cx| i.set_value("", window, cx));
    if let Some(svc) = crate::services::sync::get(cx) {
        svc.update(cx, |s, cx| s.connect(provider, server, cx));
    }
}

pub(super) fn render_where(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let options: Vec<(SharedString, bool)> = Provider::ALL.iter().map(|o| (provider_label(*o).into(), *o == p.provider)).collect();
    let select = select_field(
        "sync-provider",
        Some(tr!("Service")),
        provider_label(p.provider),
        false,
        d.cloud.provider_open,
        options,
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.cloud.provider_open = !d.cloud.provider_open;
            d.cloud.vendor_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.cloud.provider_open = false;
            let provider = Provider::ALL[i];
            persist(d, move |p| p.provider = provider, cx);
        },
    );
    let body = match p.provider {
        Provider::Folder => render_folder(d, cx),
        Provider::Custom => div().flex().flex_col().gap(px(6.)).child(text_field(tr!("rclone remote"), &d.cloud.custom_remote)).child(
            meta_text(tr!("A remote of your own rclone settings and a folder in it, like mydrive:Heronite. Set it up with rclone config.")),
        ),
        provider => render_account(d, p, provider, cx),
    };
    div().flex().flex_col().gap(px(12.)).child(select).child(body)
}

fn render_folder(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let has = !d.cloud.folder.read(cx).value().trim().is_empty();
    let field = path_field(
        "sync-folder-pick",
        Some(tr!("Sync folder")),
        &d.cloud.folder,
        has,
        false,
        cx.listener(move |d, _, window, cx| {
            if has {
                d.cloud.folder.update(cx, |s, cx| s.set_value("", window, cx));
                persist(d, |p| p.folder.clear(), cx);
            } else {
                choose_path(window, cx, true, tr!("Sync folder"), |d: &mut SettingsPage, path, window, cx| {
                    d.cloud.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
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
    let error = connect_error.or(d.cloud.error.as_ref().map(|e| e.to_string()));
    let drive_note = (provider == Provider::Drive).then(|| meta_text(tr!("Heronite only sees the files it makes in your Drive.")));
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(account)
        .children(error.map(|e| div().text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
        .children(drive_note)
}

/// What few need to change: the folder in the account, and for Google
/// Drive the Google app it signs in with. `None` for a shared folder or a
/// remote of one's own (nothing to set there).
pub(super) fn render_advanced(d: &SettingsPage, p: &SyncPrefs) -> Option<Div> {
    if matches!(p.provider, Provider::Folder | Provider::Custom) {
        return None;
    }
    let folder = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(text_field(tr!("Folder in the account"), &d.cloud.remote_folder))
        .child(meta_text(tr!("All your PCs must use the same folder.")));
    let google = (p.provider == Provider::Drive).then(|| render_google_app(d, p));
    Some(div().flex().flex_col().gap(px(16.)).child(folder).children(google))
}

/// Google Drive: the Google app it signs in with. Google limits the
/// requests an app makes a minute; the one this build carries is shared by
/// everyone using Heronite (rclone's, by everyone using rclone, when it
/// carries none), and one of the user's own has the limit to itself.
fn render_google_app(d: &SettingsPage, p: &SyncPrefs) -> Div {
    let about = if !p.google_client_id.trim().is_empty() {
        tr!("Google Drive signs in with your own Google app.")
    } else if p.google_app().is_some() {
        tr!("Google Drive signs in with Heronite's Google app, shared by everyone using Heronite. Google limits how many requests an app makes a minute: if uploads often have to wait, a Google app of your own has the limit to itself.")
    } else {
        tr!("Google Drive signs in with rclone's Google app, shared by everyone using rclone, so uploads may have to wait. A Google app of your own has the limit to itself.")
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Your own Google app")))
        .child(meta_text(about))
        .child(
            div()
                .flex()
                .gap(px(12.))
                .child(div().flex_1().child(labeled(tr!("Your Google app's client ID"), input_box(&d.cloud.google_id, false))))
                .child(div().flex_1().child(labeled(tr!("Client secret"), input_box(&d.cloud.google_secret, false)))),
        )
        .child(meta_text(tr!(
            "Make one in Google Cloud Console: turn on the Google Drive API, then APIs & Services → Credentials → OAuth client ID, of type Desktop app. After saving it here, disconnect and connect again, and use the same one on all your PCs: each Google app sees only the files it made."
        )))
        .child(div().flex().child(button_sm("cloud-google-how", ButtonKind::Ghost, tr!("How to make one")).on_click(|_, _, _| {
            crate::ui::open_external("https://rclone.org/drive/#making-your-own-client-id")
        })))
}

fn render_server_form(d: &mut SettingsPage, provider: Provider, cx: &mut Context<SettingsPage>) -> Div {
    let s = &d.cloud;
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
                d.cloud.vendor_open = !d.cloud.vendor_open;
                d.cloud.provider_open = false;
                cx.notify();
            },
            |d: &mut SettingsPage, i, _, cx| {
                d.cloud.vendor_open = false;
                d.cloud.vendor = i;
                cx.notify();
            },
        );
        first = first.child(div().w(px(200.)).child(vendor));
    } else {
        first = first.child(div().w(px(110.)).child(labeled(tr!("Port"), input_box(&s.port, false))));
    }
    let s = &d.cloud;
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
