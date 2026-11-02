//! The RetroAchievements card of Settings → Achievements: sign in with the
//! username and password (the password goes to the site once and is never
//! kept: the login's token is), the account's points, signing the emulators
//! in too (a tile per emulator, and opening one that has no settings yet),
//! hardcore mode, and the optional Web API key.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::accounts::RetroAccount;
use heronite_achievements::emulators::{Emulator, State as EmuState, Status};
use heronite_achievements::sources::retro::{api, connect};
use heronite_i18n::{tr, trn};
use heronite_launch::plan::{open_emulator_command, PlanContext};
use heronite_launch::probe::{Probe, SystemProbe};
use heronite_launch::settings::GameSettings;
use heronite_metadata::{Endpoints, Http};

use super::super::SettingsPage;
use super::{masked, set_prefs};
use crate::icons::Icon;
use crate::services::achievements as service;
use crate::theme;
use crate::ui::controls::{button_sm, card, icon_button_sm, row, sub_rows, toggle_row, toggle_row_info, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, new_input, on_commit};
use crate::ui::icon;

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
    /// What happened to the last emulator opened from its tile (`true`: an error).
    opened: Option<(SharedString, bool)>,
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
        Self { username, password, api_key, busy: false, error: None, session: None, expired: false, opened: None }
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
        EmuState::NotSetUp => (tr!("Open it once to set it up").into(), None),
        EmuState::Running => (tr!("Open now: it is signed in once it closes").into(), Some(theme::status_color("on-hold"))),
        EmuState::Failed(e) => (tr!("Could not sign it in: {error}", error = e.as_str()), Some(theme::status_color("abandoned"))),
    }
}

/// One emulator's state across its settings files (DuckStation can have two):
/// the one that needs a look first.
fn merged_state(list: &[&Status]) -> EmuState {
    let rank = |s: &EmuState| match s {
        EmuState::Failed(_) => 0,
        EmuState::Running => 1,
        EmuState::SignedOut => 2,
        EmuState::SignedIn => 3,
        EmuState::NotSetUp => 4,
    };
    list.iter().map(|s| &s.state).min_by_key(|s| rank(s)).cloned().unwrap_or(EmuState::NotSetUp)
}

/// Starts `emulator` on its own (it writes its settings the first time),
/// through the copy added on the Emulators page.
fn open_emulator(d: &mut SettingsPage, emulator: Emulator, cx: &mut Context<SettingsPage>) {
    let name = emulator.name();
    d.achievements.retro.opened = Some((tr!("Opening {name}\u{2026}", name).into(), false));
    let paths = d.ctx.data_paths.clone();
    cx.spawn(async move |this, cx| {
        let res = cx
            .background_spawn(async move {
                let key = name.to_lowercase().replace(' ', "");
                let matches = |s: &str| s.to_lowercase().replace([' ', '-', '_'], "").contains(&key);
                let list = heronite_emulation::store::list_user_emulators(&paths).map_err(|e| format!("{e:#}"))?;
                let Some(e) = list.into_iter().find(|e| matches(&e.definition_id) || matches(&e.name)) else { return Ok(false) };
                let probe = SystemProbe;
                let ctx = PlanContext::load(&paths);
                let home = probe.home();
                let settings = GameSettings::from_map(&heronite_data::game_config::load_global(&paths, &home), &home);
                let c = open_emulator_command(&ctx, &probe, &settings, &e)?;
                log::info!("opening emulator {}: {}", e.name, c.display());
                crate::runtime::spawn_detached(&c).map(|()| true)
            })
            .await;
        this.update(cx, |d, cx| {
            let opened = &mut d.achievements.retro.opened;
            match res {
                Ok(true) => {}
                Ok(false) => *opened = Some((tr!("{name} is not on the Emulators page yet: add it there first.", name).into(), true)),
                Err(e) => *opened = Some((e.into(), true)),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// The emulators as a grid of tiles, one per emulator (its settings files in
/// the tooltip), under how many are signed in and a button that looks again.
/// A tile without settings yet can open its emulator.
fn emulator_grid(d: &SettingsPage, statuses: &[Status], cx: &mut Context<SettingsPage>) -> Div {
    let mut emulators: Vec<(Emulator, Vec<&Status>)> = Vec::new();
    for s in statuses {
        match emulators.iter_mut().find(|(e, _)| *e == s.emulator) {
            Some((_, list)) => list.push(s),
            None => emulators.push((s.emulator, vec![s])),
        }
    }
    let states: Vec<EmuState> = emulators.iter().map(|(_, list)| merged_state(list)).collect();
    let signed = states.iter().filter(|s| matches!(s, EmuState::SignedIn)).count();
    let ok = theme::status_color("playing");
    let ok_a = |a: f32| Hsla { a, ..ok.into() };
    let mut meter = div().flex().gap(px(3.));
    for i in 0..states.len() {
        meter = meter.child(div().w(px(18.)).h(px(5.)).rounded_full().bg(if i < signed { ok.into() } else { theme::ink_a(0.12) }));
    }
    let head = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(div().text_size(px(13.5)).text_color(theme::text_primary()).child(tr!("Emulators")))
        .child(meta_text(tr!("{signed} of {total} signed in", signed = signed, total = states.len())))
        .child(meter)
        .child(div().flex_1())
        .child(icon_button_sm("ach-emu-check", ButtonKind::Ghost, Icon::Refresh, tr!("Check again"), true, {
            let svc = service::get(cx);
            move |_, _, cx| svc.update(cx, |s, cx| s.check_emulators(cx))
        }));
    // Signed in first, then the rest in the usual order.
    let mut order: Vec<usize> = (0..emulators.len()).collect();
    order.sort_by_key(|&i| !matches!(states[i], EmuState::SignedIn));
    let mut grid = div().grid().grid_cols(3).gap(px(8.));
    for i in order {
        let ((emulator, list), state) = (&emulators[i], &states[i]);
        let emulator = *emulator;
        let signed_in = matches!(state, EmuState::SignedIn);
        let (mut text, colour) = emulator_text(state);
        if signed_in && list.len() > 1 {
            text = trn!(list.len(), "Signed in · {n} config", "Signed in · {n} configs");
        }
        let status_icon = match state {
            EmuState::SignedIn => Some(icon(Icon::Check, px(13.), ok).into_any_element()),
            EmuState::NotSetUp => Some(
                div()
                    .flex_none()
                    .size(px(11.))
                    .rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(theme::text_dim())
                    .into_any_element(),
            ),
            _ => None,
        };
        let open = matches!(state, EmuState::NotSetUp).then(|| {
            button_sm(ElementId::NamedInteger("ach-emu-open".into(), i as u64), ButtonKind::Ghost, tr!("Open"))
                .h(px(24.))
                .px(px(8.))
                .on_click(cx.listener(move |d, _, _, cx| open_emulator(d, emulator, cx)))
        });
        let paths: Vec<String> = list.iter().filter_map(|s| s.config.as_ref().map(|p| p.display().to_string())).collect();
        let tile = div()
            .id(ElementId::NamedInteger("ach-emu".into(), i as u64))
            .flex()
            .flex_col()
            .gap(px(3.))
            .min_w_0()
            .px(px(12.))
            .py(px(10.))
            .rounded(px(11.))
            .border_1()
            .border_color(if signed_in { ok_a(0.28) } else { theme::ink_a(0.08) })
            .bg(if signed_in { ok_a(0.05) } else { theme::ink_a(0.02) })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .h(px(24.))
                    .child(div().text_size(px(13.5)).text_color(theme::text_primary()).child(emulator.name()))
                    .children(open),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(colour.map_or(theme::text_dim(), Into::into))
                    .children(status_icon)
                    .child(text),
            )
            .when(!paths.is_empty(), |t| t.tooltip(crate::ui::tooltip(paths.join("\n"))));
        grid = grid.child(tile);
    }
    let opened = d.achievements.retro.opened.clone().map(|(text, error)| {
        div().text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
    });
    div().flex().flex_col().gap(px(10.)).py(px(10.)).child(head).child(grid).children(opened)
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
    let emulators = toggle_row_info(
        "ach-emulators",
        tr!("Sign the emulators in too"),
        tr!("Games unlock achievements while you play."),
        tr!("RetroArch, DuckStation, PCSX2, PPSSPP, Dolphin and Flycast get this account, so games unlock achievements while you play. Emulators without RetroAchievements (RPCS3, Cemu, Ryujinx…) are left alone."),
        prefs.emulators,
    )
    .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.emulators = !p.emulators)))
    .into_any_element();
    // Hardcore only reaches the emulators: it sits under them, greyed out and
    // inert while they are off.
    let hardcore = toggle_row(
        "ach-hardcore",
        tr!("Hardcore mode in the emulators"),
        Some(tr!("No save states, rewind or cheats; earns hardcore points.")),
        prefs.hardcore,
    )
    .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.hardcore = !p.hardcore)))
    .into_any_element();
    let mut subs = Vec::new();
    if prefs.emulators && signed_in && !statuses.is_empty() {
        subs.push(emulator_grid(d, &statuses, cx).into_any_element());
    }
    subs.push(
        div()
            .relative()
            .child(hardcore)
            .when(!prefs.emulators, |d| d.opacity(0.42).child(div().absolute().inset_0().occlude()))
            .into_any_element(),
    );
    rows.push(div().flex().flex_col().child(emulators).child(sub_rows(subs)).into_any_element());
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
