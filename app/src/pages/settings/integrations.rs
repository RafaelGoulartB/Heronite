//! Settings → Integrations: Sunshine (and Apollo). The switch, the web UI's
//! address, user name and password (typed once, kept encrypted), which
//! games go there, and the two extras: quitting the stream closes the game,
//! and an entry that opens console mode. Everything goes through
//! `services::sunshine`, which saves it and syncs.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_sunshine::prefs::Games;

use super::SettingsPage;
use crate::icons::Icon;
use crate::pages::status_pill;
use crate::services::sunshine::{self as service, Status};
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, help_text, icon_badge, row, row_lead, segmented, toggle, toggle_row, ButtonKind};
use crate::ui::fields::{input_box, labeled, new_input};
use crate::ui::icon;

pub struct State {
    address: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let svc = service::get(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        let prefs = svc.read(cx).prefs().clone();
        let address = new_input(window, cx, &prefs.address, heronite_sunshine::api::DEFAULT_ADDRESS);
        let username = new_input(window, cx, &prefs.username, tr!("Sunshine's user name"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        cx.subscribe_in(&password, window, |d: &mut SettingsPage, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                connect(d, window, cx);
            }
        })
        .detach();
        Self { address, username, password }
    }
}

/// Keeps what was typed and syncs; the password leaves the field at once.
fn connect(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let s = &d.integrations;
    let address = s.address.read(cx).value().trim().to_string();
    let username = s.username.read(cx).value().trim().to_string();
    let password = s.password.read(cx).value().to_string();
    s.password.update(cx, |p, cx| p.set_value("", window, cx));
    service::get(cx).update(cx, |svc, cx| svc.connect(address, username, password, cx));
}

fn set(cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut heronite_sunshine::prefs::Prefs)) {
    service::get(cx).update(cx, |s, cx| s.set_prefs(change, cx));
}

/// The pill next to the name, and the line under the form.
fn status_parts(status: &Status, enabled: bool) -> (SharedString, Rgba, Option<(String, bool)>) {
    let off = (SharedString::from(tr!("Off")), theme::text_dim(), None);
    if !enabled {
        return off;
    }
    match status {
        Status::Idle => (tr!("Waiting").into(), theme::text_dim(), None),
        Status::Syncing => (tr!("Syncing…").into(), theme::status_color("on-hold"), None),
        Status::Synced { entries, .. } => (
            tr!("Connected").into(),
            theme::status_color("playing"),
            Some((
                trn!(*entries, "Up to date: {n} entry of Heronite's in Sunshine.", "Up to date: {n} entries of Heronite's in Sunshine."),
                false,
            )),
        ),
        Status::Failed(p) => (tr!("Not connected").into(), theme::status_color("abandoned"), Some((p.text(), true))),
    }
}

fn account_card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let (prefs, status, has_password) = {
        let svc = service::get(cx);
        let svc = svc.read(cx);
        (svc.prefs().clone(), svc.status().clone(), svc.has_password())
    };
    let (pill, colour, line) = status_parts(&status, prefs.enabled);
    let busy = status == Status::Syncing;
    let main = row_lead(
        icon_badge(Icon::Tv),
        tr!("Keep my games in Sunshine"),
        Some(tr!("Moonlight can then start them from a phone, a TV or another computer. Apollo works too.")),
        div().flex().items_center().gap(px(10.)).child(status_pill(pill, colour)).child(toggle(
            "sunshine-enabled-sw",
            prefs.enabled,
            false,
        )),
    )
    .id("sunshine-enabled")
    .cursor_pointer()
    .on_click(cx.listener(|_, _, _, cx| set(cx, |p| p.enabled = !p.enabled)));
    let s = &d.integrations;
    let password_label = if has_password { tr!("Password (saved)") } else { tr!("Password") };
    let form = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .py(px(12.))
        .child(labeled(tr!("Address of Sunshine's web page"), input_box(&s.address, busy)))
        .child(
            div()
                .flex()
                .gap(px(12.))
                .child(div().flex_1().min_w_0().child(labeled(tr!("User name"), input_box(&s.username, busy))))
                .child(div().flex_1().min_w_0().child(labeled(password_label, input_box(&s.password, busy)))),
        )
        .children(line.map(|(text, error)| {
            div().text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
        }))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .child(
                    button_sm("sunshine-connect", ButtonKind::Primary, if busy { tr!("Syncing…") } else { tr!("Connect") })
                        .when(busy, |b| b.opacity(0.7))
                        .when(!busy, |b| b.on_click(cx.listener(|d, _, window, cx| connect(d, window, cx)))),
                )
                .when(prefs.enabled && has_password, |r| {
                    r.child(
                        button_sm("sunshine-sync", ButtonKind::Glass, tr!("Sync now"))
                            .on_click(cx.listener(|_, _, _, cx| service::get(cx).update(cx, |s, cx| s.sync_now(cx)))),
                    )
                })
                .child(
                    div()
                        .id("sunshine-open")
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(13.))
                        .text_color(theme::accent())
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.85))
                        .child(tr!("Open Sunshine"))
                        .child(icon(Icon::OpenInNew, px(13.), theme::accent()))
                        .on_click({
                            let address = prefs.address.clone();
                            move |_, _, cx| cx.open_url(&address)
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(theme::text_dim())
                .child(icon(Icon::Lock, px(13.), theme::text_dim()))
                .child(tr!("The password is kept encrypted on this computer and only goes to Sunshine.")),
        );
    card([main.into_any_element(), form.into_any_element()])
}

fn options_card(cx: &mut Context<SettingsPage>) -> Div {
    let prefs = service::get(cx).read(cx).prefs().clone();
    let games = segmented(
        "sunshine-games",
        &[(Games::Installed, tr!("Installed games")), (Games::Favourites, tr!("Installed favourites"))],
        prefs.games,
        true,
        |g, _, cx| {
            service::get(cx).update(cx, |s, cx| s.set_prefs(|p| p.games = g, cx));
        },
    );
    let rows = [
        row(tr!("Games in Sunshine"), Some(tr!("Games leave Sunshine again when they are uninstalled.")), games).into_any_element(),
        toggle_row(
            "sunshine-stop",
            tr!("Quitting in Moonlight closes the game"),
            Some(tr!("Without it, the game keeps running on this computer after the stream.")),
            prefs.stop_on_quit,
        )
        .on_click(cx.listener(|_, _, _, cx| set(cx, |p| p.stop_on_quit = !p.stop_on_quit)))
        .into_any_element(),
        toggle_row(
            "sunshine-console",
            tr!("Add Heronite console mode"),
            Some(tr!("One more application in Sunshine that opens Heronite in console mode, for the couch.")),
            prefs.console_entry,
        )
        .on_click(cx.listener(|_, _, _, cx| set(cx, |p| p.console_entry = !p.console_entry)))
        .into_any_element(),
    ];
    card(rows).when(!prefs.enabled, |c| c.opacity(0.42))
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(caption("Sunshine"))
        .child(account_card(d, cx))
        .child(options_card(cx))
        .child(help_text(tr!(
            "Heronite only changes the applications it added to Sunshine; your own stay as they are. Turning this off removes Heronite's applications."
        )))
        .into_any_element()
}
