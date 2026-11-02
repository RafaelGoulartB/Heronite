//! Settings → Cloud: one place for everything that leaves this PC
//! (`heronite_sync`, run by `services::sync`), top to bottom: how things
//! stand (`overview.rs`: connected or not, how much is up, "Send everything
//! now"); 1, where they go (`account.rs`: a shared folder, a cloud account or
//! a server through rclone, signed in once; the folder in the account and a
//! Google app of one's own behind "Advanced options"); 2, what goes there and when (`features.rs`: the library
//! sync, backups, game saves, screenshots, each off, manual or automatic);
//! 3, the space it needs (`space.rs`, measured on this PC); and the PCs
//! (this one's name, the others found, how the last sync went). Backups, Saves and
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
    /// The PC to bring things back from (`None`: the newest backup's), and
    /// whether its menu is open.
    restore_pc: Option<String>,
    restore_pc_open: bool,
    /// The part of the space bar under the pointer.
    hover: Option<heronite_sync::usage::Kind>,
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
            restore_pc: None,
            restore_pc_open: false,
            hover: None,
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

/// One step of the page: its heading, a line on what it is for, and its body.
fn step(heading: &str, about: &str, body: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .child(caption(heading.to_owned()))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(about.to_owned())))
        .child(body)
}

/// Where things go, with "Advanced options" folded under it.
fn render_place(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let mut card = card_box().py(px(12.)).gap(px(12.)).child(account::render_where(d, p, cx));
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
        card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(toggle);
        if open {
            card = card.child(div().pl(px(22.)).pb(px(4.)).child(advanced));
        }
    }
    card
}

/// This PC's name, the other PCs found, and how the last sync went.
fn render_devices(d: &SettingsPage, p: &SyncPrefs) -> Div {
    let mut card = card_box().py(px(12.)).gap(px(4.)).child(text_field(tr!("This PC's name"), &d.cloud.device_name));
    if p.devices.is_empty() {
        card = card.child(div().pt(px(8.)).child(meta_text(tr!(
            "No other PC yet. Set up Heronite on your other PCs with the same account or folder, and they show here."
        ))));
    }
    for dev in &p.devices {
        let when = tr!("last synced {when}", when = super::format_time(&dev.written_at));
        let detail = format!("{} \u{b7} {when}", os_label(&dev.os));
        card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(row(dev.name.clone(), Some(&detail), div()));
    }
    div().flex().flex_col().child(card).child(render_sync_result(p))
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let Some(p) = d.cloud.prefs.clone() else {
        // `sync.json` could not be read: say why instead of waiting forever.
        return match d.cloud.error.clone() {
            Some(e) => div().text_size(px(13.)).text_color(theme::danger_text()).child(e).into_any_element(),
            None => meta_text(tr!("Loading…")).into_any_element(),
        };
    };
    let place = render_place(d, &p, cx);
    let what = features::render(d, &p, cx);
    let space = space::render(d, &p, cx);
    let what_about = if p.ready() {
        tr!("Automatic: Heronite sends by itself, never during a game. Manual: only when you press Send now. Installed games, Wine prefixes, game settings and store sign-ins always stay on each PC.")
    } else {
        tr!("Connect above first. Then choose, for each, whether it goes by itself (Automatic), only when you press Send now (Manual), or not at all.")
    };
    div()
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(overview::render(d, &p, cx))
        .child(step(
            tr!("1. Where your things go"),
            tr!("A cloud account, a server, or a folder all your PCs reach. Sign in once: everything below uses it."),
            place,
        ))
        .child(step(tr!("2. What goes there"), what_about, what))
        .child(step(
            tr!("3. Space it needs"),
            tr!("Measured on this PC: how much room the account needs for what you send. The account itself is not asked."),
            space,
        ))
        .children(p.ready().then(|| {
            step(
                tr!("4. Restore from the cloud"),
                tr!("New PC, or formatted this one? Connect, then bring back your Collection, game saves and screenshots."),
                restore::render(d, cx),
            )
        }))
        .child(step(
            tr!("Your PCs"),
            tr!("Every PC that syncs with this account or folder, and the name the others see for this one."),
            render_devices(d, &p),
        ))
        .into_any_element()
}
