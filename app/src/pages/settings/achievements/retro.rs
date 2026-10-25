//! The RetroAchievements card of Settings → Achievements: sign in with the
//! username and password (the password goes to the site once and is never
//! kept: the login's token is), the account's points, signing the emulators
//! in too (each one's state), hardcore mode, and the optional Web API key.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::accounts::RetroAccount;
use heronite_achievements::emulators::{State as EmuState, Status};
use heronite_achievements::sources::retro::{api, connect};
use heronite_i18n::tr;
use heronite_metadata::{Endpoints, Http};

use super::super::SettingsPage;
use super::{masked, set_prefs};
use crate::pages::status_pill;
use crate::services::achievements as service;
use crate::theme;
use crate::ui::controls::{button_sm, card, row, toggle_row, with_sub_rows, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, new_input, on_commit};

pub struct State {
    username: Entity<InputState>,
    password: Entity<InputState>,
    api_key: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    /// The signed-in account as the site last answered (its points).
    session: Option<connect::Session>,
    /// The kept token was refused: the password is needed again.
    expired: bool,
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let key = service::get(cx).read(cx).accounts().retro_achievements.as_ref().map(|a| a.api_key.clone()).unwrap_or_default();
        let username = new_input(window, cx, "", tr!("Your RetroAchievements username"));
        let password = masked(window, cx, "");
        cx.subscribe_in(&password, window, |d: &mut SettingsPage, _, ev: &gpui_kit::component::input::InputEvent, window, cx| {
            if matches!(ev, gpui_kit::component::input::InputEvent::PressEnter { .. }) {
                sign_in(d, window, cx);
            }
        })
        .detach();
        let api_key = masked(window, cx, &key);
        on_commit(&api_key, window, cx, |_: &mut SettingsPage, value, _, cx| {
            service::get(cx).update(cx, |s, cx| s.set_retro_api_key(value, cx));
        });
        Self { username, password, api_key, busy: false, error: None, session: None, expired: false }
    }
}

/// Reads the account's points again (and finds out a refused token).
pub fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let Some(account) = service::get(cx).read(cx).accounts().retro_achievements.clone().filter(|a| !a.token.is_empty()) else { return };
    if d.achievements.retro.session.is_some() {
        return;
    }
    cx.spawn(async move |this, cx| {
        let checked = cx
            .background_spawn(async move { connect::check(&Http::new(Endpoints::default()), api::BASE, &account.username, &account.token) })
            .await;
        this.update(cx, |d, cx| {
            match checked {
                Ok(session) => d.achievements.retro.session = Some(session),
                Err(e) if connect::is_rejected(&e) => d.achievements.retro.expired = true,
                Err(e) => log::info!("RetroAchievements account check: {e:#}"),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn sign_in(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let state = &mut d.achievements.retro;
    if state.busy {
        return;
    }
    let username = state.username.read(cx).value().trim().to_string();
    let password = state.password.read(cx).value().to_string();
    if username.is_empty() || password.is_empty() {
        state.error = Some(tr!("Type your username and your password.").into());
        cx.notify();
        return;
    }
    state.busy = true;
    state.error = None;
    // The password leaves the field at once: only the token is kept.
    state.password.update(cx, |s, cx| s.set_value("", window, cx));
    cx.notify();
    cx.spawn(async move |this, cx| {
        let login =
            cx.background_spawn(async move { connect::login(&Http::new(Endpoints::default()), api::BASE, &username, &password) }).await;
        this.update(cx, |d, cx| {
            let state = &mut d.achievements.retro;
            state.busy = false;
            match login {
                Ok(session) => {
                    let svc = service::get(cx);
                    let old = svc.read(cx).accounts().retro_achievements.clone().unwrap_or_default();
                    let same_user = old.username.eq_ignore_ascii_case(&session.username);
                    let account = RetroAccount {
                        username: session.username.clone(),
                        token: session.token.clone(),
                        api_key: if same_user { old.api_key } else { String::new() },
                    };
                    state.session = Some(session);
                    state.expired = false;
                    svc.update(cx, |s, cx| s.set_retro_account(Some(account), cx));
                }
                Err(e) if connect::is_rejected(&e) => state.error = Some(tr!("Wrong username or password.").into()),
                Err(e) => {
                    log::warn!("RetroAchievements sign-in: {e:#}");
                    state.error = Some(tr!("Could not reach RetroAchievements. Check the connection and try again.").into());
                }
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn emulator_text(state: &EmuState) -> (String, Option<Rgba>) {
    match state {
        EmuState::SignedIn => (tr!("Signed in").into(), Some(theme::status_color("playing"))),
        EmuState::SignedOut => (tr!("Not signed in").into(), None),
        EmuState::NotSetUp => (tr!("Not found: open it once so it creates its settings").into(), None),
        EmuState::Running => (tr!("Open now: it is signed in once it closes").into(), Some(theme::status_color("on-hold"))),
        EmuState::Failed(e) => (tr!("Could not sign it in: {error}", error = e.as_str()), Some(theme::status_color("abandoned"))),
    }
}

fn emulator_rows(statuses: &[Status]) -> Vec<AnyElement> {
    statuses
        .iter()
        .map(|s| {
            let (text, colour) = emulator_text(&s.state);
            let detail = s.config.as_ref().map(|p| p.display().to_string());
            let status: AnyElement = match colour {
                Some(c) => status_pill(text, c).into_any_element(),
                None => div().text_size(px(12.)).text_color(theme::ink_a(0.55)).child(text).into_any_element(),
            };
            row(s.emulator.name(), detail.as_deref(), status).into_any_element()
        })
        .collect()
}

pub fn card_view(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let (account, prefs, statuses) = {
        let svc = service::get(cx);
        let svc = svc.read(cx);
        (svc.accounts().retro_achievements.clone(), svc.prefs(), svc.emulator_status().to_vec())
    };
    let state = &d.achievements.retro;
    let signed_in = account.as_ref().is_some_and(|a| !a.token.is_empty()) && !state.expired;
    let mut rows: Vec<AnyElement> = Vec::new();
    if let (true, Some(account)) = (signed_in, account.as_ref()) {
        let detail = match &state.session {
            Some(s) => tr!(
                "Connected · {hardcore} hardcore points, {softcore} softcore",
                hardcore = heronite_i18n::grouped(s.points as i64),
                softcore = heronite_i18n::grouped(s.softcore_points as i64)
            ),
            None => tr!("Connected").to_string(),
        };
        rows.push(
            row(
                account.username.clone(),
                Some(&detail),
                button_sm("ach-ra-sign-out", ButtonKind::Ghost, tr!("Sign out")).on_click(cx.listener(|d, _, _, cx| {
                    d.achievements.retro.session = None;
                    service::get(cx).update(cx, |s, cx| s.set_retro_account(None, cx));
                })),
            )
            .into_any_element(),
        );
    } else {
        rows.push(login_form(d, account.as_ref(), cx).into_any_element());
    }
    let emulators = toggle_row(
        "ach-emulators",
        tr!("Sign the emulators in too"),
        Some(tr!("RetroArch, DuckStation, PCSX2, PPSSPP, Dolphin and Flycast get this account, so games unlock achievements while you play. Emulators without RetroAchievements (RPCS3, Cemu, Ryujinx…) are left alone.")),
        prefs.emulators,
    )
    .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.emulators = !p.emulators)))
    .into_any_element();
    let shown_statuses = if prefs.emulators && signed_in { emulator_rows(&statuses) } else { Vec::new() };
    rows.push(with_sub_rows(emulators, shown_statuses));
    rows.push(
        toggle_row(
            "ach-hardcore",
            tr!("Hardcore mode in the emulators"),
            Some(tr!("No save states, rewind or cheats; earns hardcore points.")),
            prefs.hardcore,
        )
        .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.hardcore = !p.hardcore)))
        .into_any_element(),
    );
    rows.push(
        div()
            .py(px(11.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(labeled(tr!("Web API key (optional)"), input_box(&d.achievements.retro.api_key, false)))
            .child(meta_text(tr!("Adds the time of unlocks made before you connected. It is at retroachievements.org → Settings → Keys.")))
            .into_any_element(),
    );
    card(rows)
}

fn login_form(d: &mut SettingsPage, account: Option<&RetroAccount>, cx: &mut Context<SettingsPage>) -> Div {
    let state = &d.achievements.retro;
    let lead = if state.expired {
        tr!("RetroAchievements no longer accepts the saved sign-in (the password changed?). Sign in again.")
    } else if account.is_some_and(|a| a.token.is_empty()) {
        tr!("Sign in with your password to also connect the emulators.")
    } else {
        tr!("Sign in to see the achievements of your ROMs and to connect the emulators. The password goes to RetroAchievements once and is never saved.")
    };
    div()
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(meta_text(lead))
        .child(
            div()
                .flex()
                .gap(px(12.))
                .child(div().flex_1().child(labeled(tr!("Username"), input_box(&state.username, state.busy))))
                .child(div().flex_1().child(labeled(tr!("Password"), input_box(&state.password, state.busy)))),
        )
        .children(state.error.clone().map(|e| div().text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
        .child(
            div().flex().child(
                button_sm("ach-ra-sign-in", ButtonKind::Primary, if state.busy { tr!("Signing in…") } else { tr!("Sign in") })
                    .on_click(cx.listener(|d, _, window, cx| sign_in(d, window, cx))),
            ),
        )
}
