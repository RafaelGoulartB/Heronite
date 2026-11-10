//! Settings → Cloud: one place for everything that leaves this PC
//! (`heronite_sync`, run by `services::sync`).
//!
//! Before an account is connected the page walks through it: the steps
//! (`overview.rs`), 1, where things go (`account.rs`: a shared folder, a
//! cloud account or a server through rclone; the folder in the account and a
//! Google app of one's own behind "Advanced options"), and 2, what goes
//! there, waiting for the connection. Once connected: how things stand, with
//! the account folded under "Account"; what goes to the cloud (`features.rs`:
//! the library sync, backups, game saves, screenshots, each off, manual or
//! automatic, under the space they take, `space.rs`, measured on this PC);
//! then, folded, restoring from the cloud (`restore.rs`) and the PCs (this
//! one's name, the others found, how the last sync went). Backups, Saves and
//! Screenshots link here (`features::link_row`).

mod account;
mod features;
mod overview;
mod restore;
mod space;

pub(super) use features::link_row;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_sync::prefs::{self, LastSummary};
use heronite_sync::SyncPrefs;

use super::SettingsPage;
use crate::icons::Icon;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{caption, card_box, row};
use crate::ui::fields::{meta_text, new_input, on_commit, text_field};
use crate::ui::find::flash;
use crate::ui::icon;

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
    /// Google Drive: the user's own Google app.
    google_id: Entity<InputState>,
    google_secret: Entity<InputState>,
    /// WebDAV server kind (`WEBDAV_VENDORS`).
    vendor: usize,
    provider_open: bool,
    vendor_open: bool,
    /// What this PC sends, measured here when shown.
    usage: Option<heronite_sync::usage::Usage>,
    measuring: bool,
    /// The treemap of what takes the space is open.
    map_open: bool,
    /// "Advanced options" of the account are shown.
    advanced_open: bool,
    /// Once connected, the account (service, sign-in, advanced options) is
    /// unfolded in the top card; before, it is always shown.
    account_open: bool,
    /// "Restore from the cloud" and "Your PCs" are unfolded.
    restore_open: bool,
    pcs_open: bool,
    /// The PC to bring things back from (`None`: the newest backup's), and
    /// whether its menu is open.
    restore_pc: Option<String>,
    restore_pc_open: bool,
    /// A server's form was not filled in, or a write failed.
    error: Option<SharedString>,
    /// The service's change count last seen (`LibrarySync::changes`).
    seen_changes: u64,
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
        let google_id = new_input(window, cx, "", "1234-abc.apps.googleusercontent.com");
        on_commit(&google_id, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.cloud.prefs.as_ref().is_some_and(|p| p.google_client_id != value) {
                persist(d, move |p| p.google_client_id = value, cx);
            }
        });
        let google_secret = cx.new(|cx| InputState::new(window, cx).masked(true));
        on_commit(&google_secret, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if d.cloud.prefs.as_ref().is_some_and(|p| p.google_client_secret != value) {
                persist(d, move |p| p.google_client_secret = value, cx);
            }
        });
        if let Some(svc) = crate::services::sync::get(cx) {
            cx.observe_in(&svc, window, |d, svc, window, cx| {
                // A sign-in, a sign-out, a sync or an upload ended: what it
                // changed shows at once, whatever else still runs.
                let changes = svc.read(cx).changes();
                if changes != d.cloud.seen_changes {
                    d.cloud.seen_changes = changes;
                    reload(d, cx);
                    // What went up changed the dots.
                    if d.section == super::Section::Cloud {
                        space::measure(d, cx);
                    }
                }
                // A backup brought from the cloud to be restored: Backups asks.
                if let Some(path) = svc.update(cx, |s, _| s.take_backup_to_restore()) {
                    d.select(super::Section::Backups, window, cx);
                    super::backup::open_and_ask(d, path, cx);
                }
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
        // `HERONITE_DEBUG_CLOUD_OPEN=account,restore,pcs`: those parts start unfolded.
        let unfolded = crate::env::value("HERONITE_DEBUG_CLOUD_OPEN").unwrap_or_default();
        let starts_open = |part: &str| unfolded.split(',').any(|p| p.trim() == part);
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
            google_id,
            google_secret,
            vendor: 0,
            provider_open: false,
            vendor_open: false,
            usage: None,
            measuring: false,
            // `HERONITE_DEBUG_CLOUD_MAP=1`: the treemap starts open.
            map_open: crate::env::flag("HERONITE_DEBUG_CLOUD_MAP"),
            // `HERONITE_DEBUG_CLOUD_ADVANCED=1`: "Advanced options" start open.
            advanced_open: crate::env::flag("HERONITE_DEBUG_CLOUD_ADVANCED"),
            account_open: starts_open("account"),
            restore_open: starts_open("restore"),
            pcs_open: starts_open("pcs"),
            restore_pc: None,
            restore_pc_open: false,
            error: None,
            seen_changes: 0,
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
    space::measure(d, cx);
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
        (&s.google_id, p.google_client_id.clone()),
        (&s.google_secret, p.google_client_secret.clone()),
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
        svc.update(cx, |s, cx| s.sync(false, cx));
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
    if s.fields + s.sessions + s.playtime + s.groups == 0 {
        lines.push(tr!("Everything was already up to date.").to_string());
    } else {
        let mut parts = Vec::new();
        if s.sessions > 0 {
            parts.push(trn!(s.sessions, "{n} play session", "{n} play sessions"));
        }
        if s.playtime > 0 {
            parts.push(trn!(s.playtime, "{n} play time", "{n} play times"));
        }
        if s.groups > 0 {
            parts.push(trn!(s.groups, "{n} smart category", "{n} smart categories"));
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
        (None, Some(s)) if p.last_sync_at.is_some() && p.library.is_on() => (summary_lines(s), false),
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

/// The Settings search jumped to `label` (its English key): unfolds the
/// part of the page that draws it, so it can flash.
pub(super) fn reveal(d: &mut SettingsPage, label: &str) {
    let s = &mut d.cloud;
    match label {
        "Service" | "Sync folder" | "Advanced options" => s.account_open = true,
        "Folder in the account" | "Your own Google app" => {
            s.account_open = true;
            s.advanced_open = true;
        }
        "Restore from the cloud" | "What is in the cloud" => s.restore_open = true,
        "Your PCs" | "This PC's name" => s.pcs_open = true,
        _ => {}
    }
}

fn separator() -> Div {
    div().h(px(1.)).w_full().bg(theme::row_separator())
}

/// A square with the icon of a line or a part of the page.
fn tile(ic: Icon, fill: impl Into<Hsla>, color: impl Into<Hsla>) -> Div {
    div().flex_none().size(px(34.)).rounded(px(10.)).flex().items_center().justify_center().bg(fill.into()).child(icon(ic, px(18.), color))
}

/// One step before an account is connected: its number and heading, a line
/// on what it is for, and its body. Only the step to do now is lit.
fn step(n: &str, heading: &str, about: &str, now: bool, body: impl IntoElement) -> Div {
    let badge = div()
        .flex_none()
        .size(px(20.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.))
        .font_weight(theme::W_BOLD)
        .map(|b| {
            if now {
                b.bg(theme::accent()).text_color(theme::on_accent())
            } else {
                b.bg(theme::ink_a(0.08)).text_color(theme::text_muted())
            }
        })
        .child(n.to_owned());
    let title = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mx(px(6.))
        .mb(px(4.))
        .child(badge)
        .child(div().text_size(px(14.)).text_color(if now { theme::text_primary() } else { theme::text_muted() }).child(heading.to_owned()))
        .children((!now).then(|| icon(Icon::Lock, px(14.), theme::text_dim())));
    div().flex().flex_col().child(title).child(div().px(px(6.)).pb(px(8.)).child(meta_text(about.to_owned()))).child(body)
}

/// Where things go, with "Advanced options" folded under it.
fn render_place(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let mut col = div().flex().flex_col().gap(px(12.)).child(account::render_where(d, p, cx));
    if let Some(advanced) = account::render_advanced(d, p) {
        let open = d.cloud.advanced_open;
        let toggle = div()
            .id("cloud-advanced")
            .flex()
            .items_center()
            .gap(px(6.))
            .py(px(4.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(theme::text_muted())
            .hover(|s| s.text_color(theme::text_bright()))
            .child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(16.), theme::text_muted()))
            .child(tr!("Advanced options"))
            .on_click(cx.listener(move |d, _, _, cx| {
                d.cloud.advanced_open = !open;
                cx.notify();
            }));
        col = col.child(separator()).child(toggle);
        if open {
            col = col.child(div().pl(px(22.)).pb(px(4.)).child(advanced));
        }
    }
    col
}

/// This PC's name, the other PCs found, and how the last sync went.
fn render_devices(d: &SettingsPage, p: &SyncPrefs) -> Div {
    let mut col = div().flex().flex_col().gap(px(4.)).child(text_field(tr!("This PC's name"), &d.cloud.device_name));
    if p.devices.is_empty() {
        col = col.child(div().pt(px(8.)).child(meta_text(tr!(
            "No other PC yet. Set up Heronite on your other PCs with the same account or folder, and they show here."
        ))));
    }
    for dev in &p.devices {
        let when = tr!("last synced {when}", when = super::format_time(&dev.written_at));
        let detail = format!("{} \u{b7} {when}", os_label(&dev.os));
        col = col.child(separator()).child(row(dev.name.clone(), Some(&detail), div()));
    }
    col.child(render_sync_result(p))
}

/// A part of the page folded under one line (its icon, name and what it
/// holds); `body` is drawn under it while unfolded.
fn fold(
    id: &'static str,
    ic: Icon,
    label: &'static str,
    about: String,
    body: Option<Div>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let open = body.is_some();
    let head = div()
        .id(id)
        .relative()
        .children(flash(label, 8.))
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(12.))
        .cursor_pointer()
        .child(tile(ic, theme::ink_a(0.06), theme::text_muted()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(label))
                .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(about)),
        )
        .child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(18.), theme::text_muted()))
        .on_click(on_click);
    div().flex().flex_col().child(head).children(body.map(|b| div().pl(px(46.)).pb(px(12.)).child(b)))
}

fn pcs_fold(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let name = if p.device_name.trim().is_empty() { prefs::default_device_name() } else { p.device_name.trim().to_owned() };
    let about = format!("{} \u{b7} {}", tr!("This PC: {name}", name), trn!(p.devices.len(), "{n} other PC", "{n} other PCs"));
    let open = d.cloud.pcs_open;
    fold(
        "cloud-pcs",
        Icon::Tv,
        tr!("Your PCs"),
        about,
        open.then(|| render_devices(d, p)),
        cx.listener(move |d, _, _, cx| {
            d.cloud.pcs_open = !open;
            cx.notify();
        }),
    )
}

/// Connected: how things stand (the account folded in it), what goes to the
/// cloud, then restoring and the PCs, folded.
fn render_connected(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let mut top = overview::render(d, p, cx);
    if d.cloud.account_open {
        top = top.child(separator()).child(render_place(d, p, cx));
    }
    let restore_open = d.cloud.restore_open;
    let restore = fold(
        "cloud-restore",
        Icon::CloudDownload,
        tr!("Restore from the cloud"),
        tr!("New PC, or formatted this one? Connect, then bring back your Collection, game saves and screenshots.").to_owned(),
        restore_open.then(|| restore::render(d, cx)),
        cx.listener(move |d, _, _, cx| {
            d.cloud.restore_open = !restore_open;
            cx.notify();
        }),
    );
    div()
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(top)
        .child(
            div()
                .flex()
                .flex_col()
                .child(caption(tr!("What goes to the cloud")))
                .child(features::render(d, p, cx))
                .child(features::stays_note()),
        )
        .child(card_box().child(restore).child(separator()).child(pcs_fold(d, p, cx)))
}

/// Before an account is connected: the steps, the one to do now, and what
/// comes after it, waiting for the connection.
fn render_first_use(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let place = card_box().py(px(12.)).child(render_place(d, p, cx));
    div()
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(overview::render(d, p, cx))
        .child(step(
            "1",
            tr!("Choose where and connect"),
            tr!("A cloud account, a server, or a folder all your PCs reach. Sign in once: everything below uses it."),
            true,
            place,
        ))
        .child(step(
            "2",
            tr!("Choose what goes, and when"),
            tr!("Connect above first. Then choose, for each, whether it goes by itself (Automatic), only when you press Send now (Manual), or not at all."),
            false,
            div().flex().flex_col().child(features::render(d, p, cx)).child(features::stays_note()),
        ))
        .child(card_box().child(pcs_fold(d, p, cx)))
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let Some(p) = d.cloud.prefs.clone() else {
        // `sync.json` could not be read: say why instead of waiting forever.
        return match d.cloud.error.clone() {
            Some(e) => div().text_size(px(13.)).text_color(theme::danger_text()).child(e).into_any_element(),
            None => meta_text(tr!("Loading…")).into_any_element(),
        };
    };
    if p.ready() {
        render_connected(d, &p, cx).into_any_element()
    } else {
        render_first_use(d, &p, cx).into_any_element()
    }
}
