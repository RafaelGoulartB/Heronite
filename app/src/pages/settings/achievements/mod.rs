//! Settings → Achievements: the RetroAchievements account and the emulators
//! (`retro`), the optional Steam Web API key, how achievements show, and
//! "Update now" for every game. Everything goes through
//! `services::achievements`, which saves it and reads the games that were
//! waiting.

mod retro;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::prefs::Prefs;
use heronite_data::appearance::Section;
use heronite_i18n::tr;

use super::SettingsPage;
use crate::pages::status_pill;
use crate::services::achievements as service;
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, row, toggle_row, with_sub_rows, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, on_commit};

pub struct State {
    retro: retro::State,
    steam_key: Entity<InputState>,
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
        let steam_key = masked(window, cx, &steam);
        on_commit(&steam_key, window, cx, |_: &mut SettingsPage, value, _, cx| {
            service::get(cx).update(cx, |s, cx| s.set_steam_key(Some(value), cx));
        });
        Self { retro: retro::State::new(window, cx), steam_key }
    }
}

/// Reads the account's points and the emulators' state when the section shows.
pub fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    retro::shown(d, cx);
    service::get(cx).update(cx, |s, cx| s.check_emulators(cx));
}

pub(super) fn set_prefs(cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut Prefs)) {
    let svc = service::get(cx);
    let mut prefs = svc.read(cx).prefs();
    change(&mut prefs);
    svc.update(cx, |s, cx| s.set_prefs(prefs, cx));
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let (prefs, bulk) = {
        let svc = service::get(cx);
        let svc = svc.read(cx);
        (svc.prefs(), svc.bulk)
    };
    let on = service::enabled();
    let main = card([toggle_row(
        "ach-enabled",
        tr!("Turn on achievements"),
        Some(tr!(
            "Shows the Achievements section on each game's page and keeps them up to date. Off, the section, the Timeline days, the updates and the notifications stop. It is the same switch as Achievements in Settings → Appearance → Game sections."
        )),
        on,
    )
    .on_click(cx.listener(|d, _, _, cx| {
        super::appearance::set_look(d, cx, |a| a.sections = a.sections.toggled(Section::Achievements));
        // Turned on: read the focused game and the rest as they show.
        cx.notify();
    }))
    .into_any_element()]);
    let rest = div()
        .flex()
        .flex_col()
        .when(!on, |d| d.opacity(0.5))
        .child(caption("RetroAchievements"))
        .child(retro::card_view(d, cx))
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
            with_sub_rows(
                toggle_row(
                    "ach-notify",
                    tr!("Notify new achievements"),
                    Some(tr!("A desktop notification when a game unlocks something.")),
                    prefs.notify,
                )
                .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.notify = !p.notify)))
                .into_any_element(),
                vec![row(
                    tr!("Try the notification"),
                    Some(tr!("Shows what a game's new achievements look like, with made-up ones.")),
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button_sm("ach-notify-test-1", ButtonKind::Glass, tr!("One achievement"))
                                .on_click(|_, _, _| service::notify_test(1)),
                        )
                        .child(
                            button_sm("ach-notify-test-3", ButtonKind::Glass, tr!("Several achievements"))
                                .on_click(|_, _, _| service::notify_test(4)),
                        ),
                )
                .into_any_element()],
            ),
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
        ;
    div().flex().flex_col().child(main).child(rest).into_any_element()
}
