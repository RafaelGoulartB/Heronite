//! Settings → Achievements: the RetroAchievements account (checked against
//! its Web API before it is saved), the optional Steam Web API key, how
//! achievements show, and "Update now" for every game. Everything goes
//! through `services::achievements`, which saves it and reads the games that
//! were waiting.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::accounts::RetroAccount;
use heronite_achievements::prefs::Prefs;
use heronite_achievements::sources::retro;
use heronite_i18n::tr;
use heronite_metadata::{Endpoints, Http};

use super::SettingsPage;
use crate::pages::status_pill;
use crate::services::achievements as service;
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, row, toggle_row, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, new_input, on_commit};

pub struct State {
    username: Entity<InputState>,
    api_key: Entity<InputState>,
    steam_key: Entity<InputState>,
    checking: bool,
    error: Option<SharedString>,
    /// Points of the connected account, once read.
    profile: Option<retro::Profile>,
}

fn masked(window: &mut Window, cx: &mut Context<SettingsPage>, value: &str) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(|cx| {
        let mut st = InputState::new(window, cx).masked(true);
        st.set_value(value, window, cx);
        st
    })
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let svc = service::get(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        let steam = svc.read(cx).accounts().steam_web_api_key.clone().unwrap_or_default();
        let username = new_input(window, cx, "", tr!("Your RetroAchievements username"));
        let api_key = masked(window, cx, "");
        let steam_key = masked(window, cx, &steam);
        on_commit(&steam_key, window, cx, |_: &mut SettingsPage, value, _, cx| {
            service::get(cx).update(cx, |s, cx| s.set_steam_key(Some(value), cx));
        });
        Self { username, api_key, steam_key, checking: false, error: None, profile: None }
    }
}

/// Reads the connected account's points when the section shows.
pub fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let Some(account) = service::get(cx).read(cx).accounts().retro_achievements.clone() else { return };
    if d.achievements.profile.is_some() {
        return;
    }
    cx.spawn(async move |this, cx| {
        let profile = cx
            .background_spawn(async move { retro::validate_account(&Http::new(Endpoints::default()), &account.username, &account.api_key) })
            .await;
        this.update(cx, |d, cx| {
            d.achievements.profile = profile.ok();
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Checks the username and key with RetroAchievements, then saves them.
fn connect(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let username = d.achievements.username.read(cx).value().trim().to_string();
    let api_key = d.achievements.api_key.read(cx).value().trim().to_string();
    if username.is_empty() || api_key.is_empty() {
        d.achievements.error = Some(tr!("Type your username and your Web API key.").into());
        cx.notify();
        return;
    }
    d.achievements.checking = true;
    d.achievements.error = None;
    cx.notify();
    cx.spawn(async move |this, cx| {
        let (u, k) = (username.clone(), api_key.clone());
        let checked = cx.background_spawn(async move { retro::validate_account(&Http::new(Endpoints::default()), &u, &k) }).await;
        this.update(cx, |d, cx| {
            d.achievements.checking = false;
            match checked {
                Ok(profile) => {
                    let account = RetroAccount { username: profile.username.clone(), api_key };
                    d.achievements.profile = Some(profile);
                    service::get(cx).update(cx, |s, cx| s.set_retro_account(Some(account), cx));
                }
                Err(e) => {
                    log::warn!("RetroAchievements account check: {e:#}");
                    d.achievements.error =
                        Some(tr!("RetroAchievements did not accept this username and key. Check them and try again.").into());
                }
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn set_prefs(cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut Prefs)) {
    let svc = service::get(cx);
    let mut prefs = svc.read(cx).prefs();
    change(&mut prefs);
    svc.update(cx, |s, cx| s.set_prefs(prefs, cx));
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let (account, prefs, bulk) = {
        let svc = service::get(cx);
        let svc = svc.read(cx);
        (svc.accounts().retro_achievements.clone(), svc.prefs(), svc.bulk)
    };
    div()
        .flex()
        .flex_col()
        .child(caption("RetroAchievements"))
        .child(retro_card(d, account, cx))
        .child(caption("Steam"))
        .child(card([
            row(
                tr!("Read from this PC's Steam"),
                Some(tr!("Achievements of the games played on this PC come from Steam's own files: no key needed.")),
                status_pill(tr!("Always on"), theme::status_color("playing")),
            )
            .into_any_element(),
            div()
                .py(px(11.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(labeled(tr!("Steam Web API key (optional)"), input_box(&d.achievements.steam_key, false)))
                .child(meta_text(tr!(
                    "For games never played on this PC. Get one at steamcommunity.com/dev/apikey; your Steam profile's game details must be public."
                )))
                .into_any_element(),
        ]))
        .child(caption(tr!("How they show")))
        .child(card([
            toggle_row("ach-notify", tr!("Notify new achievements"), Some(tr!("A desktop notification when a game unlocks something.")), prefs.notify)
                .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.notify = !p.notify)))
                .into_any_element(),
            toggle_row(
                "ach-timeline",
                tr!("Show achievements in the Timeline"),
                Some(tr!("Each day of a game's Timeline lists what it unlocked.")),
                prefs.timeline,
            )
            .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.timeline = !p.timeline)))
            .into_any_element(),
            toggle_row(
                "ach-secrets",
                tr!("Hide secret achievements"),
                Some(tr!("Their name and description stay hidden until you unlock them.")),
                prefs.hide_secrets,
            )
            .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.hide_secrets = !p.hide_secrets)))
            .into_any_element(),
        ]))
        .child(caption(tr!("Update")))
        .child(card([row(
            tr!("Update every game now"),
            Some(&if bulk.running {
                tr!("Reading {done} of {total} games…", done = bulk.done, total = bulk.total)
            } else {
                tr!("Games update by themselves when you open them and after you play.").to_string()
            }),
            button_sm("ach-update-all", ButtonKind::Glass, if bulk.running { tr!("Updating…") } else { tr!("Update now") }).on_click(
                cx.listener(|_, _, _, cx| {
                    service::get(cx).update(cx, |s, cx| s.refresh_all(cx));
                }),
            ),
        )
        .into_any_element()]))
        .into_any_element()
}

fn retro_card(d: &mut SettingsPage, account: Option<RetroAccount>, cx: &mut Context<SettingsPage>) -> Div {
    if let Some(account) = account {
        let detail = match &d.achievements.profile {
            Some(p) => tr!(
                "Connected · {hardcore} hardcore points, {softcore} softcore",
                hardcore = heronite_i18n::grouped(p.hardcore_points as i64),
                softcore = heronite_i18n::grouped(p.softcore_points as i64)
            ),
            None => tr!("Connected").to_string(),
        };
        return card([row(
            account.username.clone(),
            Some(&detail),
            button_sm("ach-ra-disconnect", ButtonKind::Ghost, tr!("Disconnect")).on_click(cx.listener(|d, _, _, cx| {
                d.achievements.profile = None;
                service::get(cx).update(cx, |s, cx| s.set_retro_account(None, cx));
            })),
        )
        .into_any_element()]);
    }
    let state = &d.achievements;
    card([div()
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(meta_text(tr!(
            "Connect your account to see the achievements of your ROMs. The Web API key is at retroachievements.org → Settings → Keys; it only reads your profile."
        )))
        .child(
            div()
                .flex()
                .gap(px(12.))
                .child(div().flex_1().child(labeled(tr!("Username"), input_box(&state.username, state.checking))))
                .child(div().flex_1().child(labeled(tr!("Web API key"), input_box(&state.api_key, state.checking)))),
        )
        .children(state.error.clone().map(|e| div().text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
        .child(
            div().flex().child(
                button_sm("ach-ra-connect", ButtonKind::Primary, if state.checking { tr!("Checking…") } else { tr!("Connect") })
                    .on_click(cx.listener(|d, _, _, cx| connect(d, cx))),
            ),
        )
        .into_any_element()])
}
