//! Settings → Achievements: the RetroAchievements account and the emulators
//! (`retro`), the optional Steam Web API key, Comet for GOG games (and, on
//! Windows, the GOG Galaxy service it needs), how achievements show, and
//! "Update now" for every game. Everything goes through
//! `services::achievements`, which saves it and reads the games that were
//! waiting.

mod retro;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::prefs::{NoticeFields, Prefs};
use heronite_data::appearance::Section;
use heronite_i18n::tr;
use heronite_platform::{supported, Feature};

use super::SettingsPage;
use crate::pages::status_pill;
use crate::services::achievements as service;
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, row, toggle_row, with_sub_rows, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, on_commit};

pub struct State {
    retro: retro::State,
    steam_key: Entity<InputState>,
    /// Windows: GOG Galaxy's service is registered (`None` until the
    /// section shows).
    galaxy_service: Option<bool>,
    /// Registering it.
    galaxy_busy: bool,
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
        Self { retro: retro::State::new(window, cx), steam_key, galaxy_service: None, galaxy_busy: false }
    }
}

/// The search jumped to `label`: open what hides it.
pub fn reveal(d: &mut SettingsPage, label: &str) {
    retro::reveal(d, label);
}

/// Reads the account's points and the emulators' state when the section shows.
pub fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    retro::shown(d, cx);
    if cfg!(windows) {
        // A registry read: GOG Galaxy may have been installed meanwhile.
        d.achievements.galaxy_service = Some(heronite_launch::galaxy::windows_service_registered());
    }
    service::get(cx).update(cx, |s, cx| s.check_emulators(cx));
}

/// "Over the game": two made-up achievements' notices in the overlay (one
/// from Steam, one from RetroAchievements) with the lines chosen below, or a
/// desktop notification saying why they cannot show (the overlay is off, or
/// this desktop cannot draw it).
fn overlay_test(d: &SettingsPage, cx: &mut App) {
    let fields = service::get(cx).read(cx).prefs().notice;
    let shown = crate::overlay::get(cx)
        .map(|o| {
            o.update(cx, |s, cx| {
                crate::overlay::made_up_unlocks("").into_iter().take(2).fold(true, |shown, u| shown & s.show_achievement(u, fields, cx))
            })
        })
        .unwrap_or(false);
    if !shown {
        let why = if d.overlay_enabled() {
            crate::overlay::desktop_status().0
        } else {
            tr!("Turn on the overlay in Settings → Overlay.").into()
        };
        crate::system::notify::send(tr!("The overlay can't show it"), why);
    }
}

/// Registers Comet's stand-in for GOG Galaxy's service (Windows asks for the
/// administrator), then says how it went.
fn register_galaxy_service(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.achievements.galaxy_busy {
        return;
    }
    d.achievements.galaxy_busy = true;
    cx.notify();
    let paths = d.ctx.data_paths.clone();
    cx.spawn(async move |this, cx| {
        let res = cx
            .background_spawn(async move { heronite_launch::galaxy::install_windows_service(&paths, &heronite_launch::net::SystemNet) })
            .await;
        this.update(cx, |d, cx| {
            d.achievements.galaxy_busy = false;
            d.achievements.galaxy_service = Some(heronite_launch::galaxy::windows_service_registered());
            if let Err(e) = res {
                log::warn!("registering GOG Galaxy's service: {e}");
                crate::system::notify::send(tr!("GOG Galaxy's service was not registered"), e);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// GOG: Comet beside GOG games, and on Windows the service some games look
/// for first.
fn gog_rows(d: &SettingsPage, prefs: &Prefs, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
    let mut rows = vec![toggle_row(
        "ach-gog-comet",
        tr!("Unlock GOG achievements while you play"),
        Some(tr!(
            "Heronite runs Comet, an open-source stand-in for GOG Galaxy's service, beside each GOG game, signed in with your GOG account: what the game unlocks counts on GOG and shows here while you play. Off, games that need GOG Galaxy for their achievements unlock nothing."
        )),
        prefs.gog_comet,
    )
    .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.gog_comet = !p.gog_comet)))
    .into_any_element()];
    let Some(registered) = d.achievements.galaxy_service.filter(|_| cfg!(windows) && prefs.gog_comet) else { return rows };
    let action = if registered {
        status_pill(tr!("Registered"), theme::status_color("playing")).into_any_element()
    } else {
        let label = if d.achievements.galaxy_busy { tr!("Registering…") } else { tr!("Register") };
        button_sm("ach-gog-service", ButtonKind::Glass, label)
            .on_click(cx.listener(|d, _, _, cx| register_galaxy_service(d, cx)))
            .into_any_element()
    };
    rows.push(
        row(
            tr!("GOG Galaxy's service"),
            Some(tr!(
                "Some games look for it before they unlock anything. Without GOG Galaxy installed, Heronite can register Comet's stand-in for it; Windows asks for the administrator once."
            )),
            action,
        )
        .into_any_element(),
    );
    rows
}

pub(super) fn set_prefs(cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut Prefs)) {
    let svc = service::get(cx);
    let mut prefs = svc.read(cx).prefs();
    change(&mut prefs);
    svc.update(cx, |s, cx| s.set_prefs(prefs, cx));
}

/// What a notice over the game shows besides the name: one switch per line,
/// each shown only when the achievement's source gives it.
fn notice_rows(prefs: &Prefs, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
    let n = prefs.notice;
    let rows: [(&'static str, &str, &str, bool, fn(&mut NoticeFields)); 6] = [
        ("ach-notice-description", tr!("Show the description"), tr!("On one line, cut when it is long."), n.description, |f| {
            f.description ^= true
        }),
        (
            "ach-notice-rarity",
            tr!("Show how rare it is"),
            tr!("The share of players who have it, in its colour when it is rare."),
            n.rarity,
            |f| f.rarity ^= true,
        ),
        ("ach-notice-points", tr!("Show points or XP"), tr!("RetroAchievements points, Epic Games XP."), n.points, |f| f.points ^= true),
        (
            "ach-notice-group",
            tr!("Show the achievement set"),
            tr!("Base game or DLC, for Steam games whose achievements come in more than one set."),
            n.group,
            |f| f.group ^= true,
        ),
        ("ach-notice-kind", tr!("Show the RetroAchievements type"), tr!("Progression, Win condition or Missable."), n.kind, |f| {
            f.kind ^= true
        }),
        (
            "ach-notice-hardcore",
            tr!("Show hardcore unlocks"),
            tr!("A badge when RetroAchievements counts it in hardcore mode."),
            n.hardcore,
            |f| f.hardcore ^= true,
        ),
    ];
    rows.into_iter()
        .map(|(id, label, sub, on, flip)| {
            toggle_row(id, label, Some(sub), on)
                .on_click(cx.listener(move |_, _, _, cx| set_prefs(cx, |p| flip(&mut p.notice))))
                .into_any_element()
        })
        .collect()
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
                    "Only for the achievements of games never played on this PC: it does not bring games in. Your whole Steam library comes in from Stores \u{2192} Steam. Get a key at steamcommunity.com/dev/apikey; your Steam profile's game details must be public."
                )))
                .into_any_element(),
            toggle_row(
                "ach-live-steam",
                tr!("Check Steam games while they run"),
                Some(tr!(
                    "Their new achievements show while you play, from Steam's stats files, when Steam writes them. Off by default: Steam's own overlay already shows them."
                )),
                prefs.live_steam,
            )
            .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.live_steam = !p.live_steam)))
            .into_any_element(),
        ]))
        .child(caption("GOG"))
        .child(card(gog_rows(d, &prefs, cx)))
        .child(caption(tr!("Game files")))
        .child(card([toggle_row(
            "ach-game-files",
            tr!("Read achievements from game files"),
            Some(tr!(
                "For DRM-free and offline copies that keep their Steam achievements next to the game, as the open-source Steam API emulators do. Heronite only reads them; names and icons may come from Steam's public pages."
            )),
            prefs.game_files,
        )
        .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.game_files = !p.game_files)))
        .into_any_element()]))
        .child(caption(tr!("How they show")))
        .child(card([
            with_sub_rows(
                toggle_row(
                    "ach-overlay-notices",
                    tr!("Show new achievements over the game"),
                    Some(tr!(
                        "While a game runs, its game files are checked every few seconds and what it unlocks shows over it. Needs the overlay (Settings → Overlay)."
                    )),
                    prefs.overlay_notices,
                )
                .on_click(cx.listener(|_, _, _, cx| set_prefs(cx, |p| p.overlay_notices = !p.overlay_notices)))
                .into_any_element(),
                notice_rows(&prefs, cx),
            ),
            with_sub_rows(
                toggle_row(
                    "ach-notify",
                    tr!("Notify new achievements"),
                    Some(tr!("A desktop notification when a game unlocks something, while it runs and after it ends.")),
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
                        )
                        .when(supported(Feature::Overlay), |d| {
                            d.child(
                                button_sm("ach-notify-test-overlay", ButtonKind::Glass, tr!("Over the game"))
                                    .on_click(cx.listener(|d, _, _, cx| overlay_test(d, cx))),
                            )
                        }),
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
