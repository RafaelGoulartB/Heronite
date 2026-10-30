//! The Collection of a new setup (no games at all): what Heronite found on
//! this PC and is bringing in by itself (Steam's library, emulators and
//! their ROM folders), the other sources, and empty covers on the side
//! where the games will show. While it shows, Steam is checked every few
//! seconds so its games come in as soon as Steam opens, and emulation is set
//! up once by itself. Also the screen shown when the library cannot be
//! loaded.
//!
//! `HERONITE_DEBUG_WELCOME=<steam>[:<emulation>]` shows a state for
//! screenshots (with `HERONITE_DISABLE_IDLE_JOBS`): `checking`, `missing`,
//! `closed`, `opening`, `restarting`, `restart`, `failed`, `reading` or
//! `connected`; `looking`, `none`, `ready` or `unplayable`.

use heronite_platform::Feature;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::clients::Client;
use heronite_emulation::setup::SetupReport;
use heronite_i18n::{tr, trn};

mod parts;

use self::parts::{client_tile, label, shelf, source_row, status, tile, Status};
use super::CollectionView;
use crate::icons::Icon;
use crate::pages::Page;
use crate::services::steam_library::SteamState;
use crate::theme;
use crate::ui::controls::{button, button_sm, icon_button_sm, kicker, ButtonKind};
use crate::ui::icon;

/// How often Steam is checked while the welcome shows.
const STEAM_POLL: Duration = Duration::from_secs(4);
const LEFT_W: f32 = 620.;
/// Below this width the shelf is left out.
const WIDE: f32 = 1180.;

#[derive(Default)]
pub(super) struct Welcome {
    /// The Steam check of an empty Collection (stops once games come in),
    /// and its watch of the Steam service where the client is read.
    watch: Option<(Task<()>, Option<Subscription>)>,
    /// Emulation set up by itself: `None` while it runs (or before).
    emulation: Option<Result<SetupReport, String>>,
    emulation_started: bool,
    /// The first load failed: shown instead of the Collection.
    pub load_error: Option<String>,
}

/// `HERONITE_DEBUG_WELCOME`'s two parts.
fn debug_welcome() -> Option<(String, String)> {
    let v = crate::env::value("HERONITE_DEBUG_WELCOME")?;
    let (steam, emu) = v.split_once(':').unwrap_or((&v, "none"));
    Some((steam.to_owned(), emu.to_owned()))
}

fn debug_steam(name: &str) -> SteamState {
    match name {
        "missing" => SteamState::NotInstalled,
        "closed" => SteamState::Closed,
        "opening" => SteamState::Opening,
        "restarting" => SteamState::Restarting,
        "restart" => SteamState::NeedsRestart,
        "failed" => SteamState::Failed,
        "reading" => SteamState::Reading,
        "connected" => SteamState::Connected,
        _ => SteamState::Checking,
    }
}

/// A sample of what emulation setup can find.
fn debug_emulation(name: &str) -> Option<Result<SetupReport, String>> {
    let report = match name {
        "looking" => return None,
        "ready" => SetupReport { emulators: vec!["RetroArch".into(), "PCSX2".into()], ..Default::default() },
        "unplayable" => SetupReport { unplayable: vec!["PlayStation 2".into(), "Nintendo GameCube".into()], ..Default::default() },
        _ => SetupReport::default(),
    };
    Some(Ok(report))
}

impl CollectionView {
    /// Checks Steam every few seconds while the Collection is empty and
    /// reads its library as soon as it can (the service alone slows down
    /// once it read the library), and sets emulation up once.
    /// Off with `HERONITE_DISABLE_IDLE_JOBS`.
    pub(super) fn start_welcome_watch(&mut self, cx: &mut Context<Self>) {
        if crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") {
            return;
        }
        if heronite_platform::supported(Feature::Emulation) {
            self.start_emulation_setup(cx);
        }
        if self.welcome.watch.is_some() {
            return;
        }
        if !heronite_platform::supported(Feature::SteamClient) {
            self.start_manifest_watch(cx);
            return;
        }
        let steam = crate::services::steam_library(cx);
        let sub = cx.observe(&steam, |_, _, cx| cx.notify());
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STEAM_POLL).await;
                let empty = this.update(cx, |this, cx| {
                    let empty = this.lib.as_ref().is_some_and(|lib| lib.games.is_empty());
                    if empty {
                        // Read once it turns ready; afterwards only watch the state.
                        let read = steam.read(cx).state() == SteamState::Connected;
                        steam.update(cx, |s, cx| if read { s.check(cx) } else { s.sync_if_ready(cx) });
                    }
                    empty
                });
                if !matches!(empty, Ok(true)) {
                    break;
                }
            }
            this.update(cx, |this, _| this.welcome.watch = None).ok();
        });
        self.welcome.watch = Some((task, Some(sub)));
    }

    /// Where Steam's client cannot be read: looks at Steam's manifests while
    /// the Collection is empty, so a game installed meanwhile comes in.
    fn start_manifest_watch(&mut self, cx: &mut Context<Self>) {
        let paths = self.data_paths.clone();
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STEAM_POLL).await;
                let Ok(true) = this.update(cx, |this, _| this.lib.as_ref().is_some_and(|lib| lib.games.is_empty())) else { break };
                let p = paths.clone();
                if cx.background_spawn(async move { super::emulation::refresh_install_states(&p) }).await {
                    this.update(cx, |this, cx| this.reload(cx)).ok();
                }
            }
            this.update(cx, |this, _| this.welcome.watch = None).ok();
        });
        self.welcome.watch = Some((task, None));
    }

    /// Registers the emulators of this PC and imports the usual ROM folders
    /// (`heronite_emulation::setup`), once per run.
    fn start_emulation_setup(&mut self, cx: &mut Context<Self>) {
        if self.welcome.emulation_started {
            return;
        }
        self.welcome.emulation_started = true;
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let media = PathBuf::from("/run/media").join(std::env::var("USER").unwrap_or_default());
                    heronite_emulation::setup::auto_setup(&paths, &heronite_emulation::SystemEnv, &media).map_err(|e| format!("{e:#}"))
                })
                .await;
            this.update(cx, |this, cx| {
                match &res {
                    Ok(r) if r.imported > 0 => {
                        this.show_toast(
                            trn!(r.imported, "Added {n} game from your ROM folders", "Added {n} games from your ROM folders"),
                            false,
                            cx,
                        );
                        this.reload(cx);
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("Setting up emulation: {e}"),
                }
                this.welcome.emulation = Some(res);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_welcome(&self, vw: f32, vh: f32, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(64.))
            .p(px(32.))
            .child(self.welcome_left(cx))
            .when(vw >= WIDE, |d| d.child(shelf(vh - 64.)))
            .into_any_element()
    }

    fn steam_state(cx: &App) -> SteamState {
        if let Some((steam, _)) = debug_welcome() {
            return debug_steam(&steam);
        }
        crate::services::steam_library_started(cx).map_or(SteamState::Checking, |s| s.read(cx).state())
    }

    fn steam_status(&self, cx: &App) -> Status {
        let busy = theme::accent();
        if !heronite_platform::supported(Feature::SteamClient) && debug_welcome().is_none() {
            return status(
                tr!("Installed games"),
                theme::text_muted(),
                tr!("Your installed Steam games come in by themselves. Install one in Steam and it shows up here."),
            );
        }
        match Self::steam_state(cx) {
            SteamState::Checking => status(tr!("Checking"), busy, tr!("Looking for Steam on this PC\u{2026}")),
            SteamState::NotInstalled => status(
                tr!("Not found"),
                theme::text_dim(),
                tr!("Steam is not on this PC. Install it and sign in: Heronite notices by itself."),
            ),
            SteamState::Closed => status(
                tr!("Closed"),
                theme::text_muted(),
                tr!("Open Steam and your games come in by themselves, installed or not. No login here."),
            ),
            SteamState::Opening => status(tr!("Opening"), busy, tr!("Steam is starting. If it asks, sign in: the rest is automatic.")),
            SteamState::Restarting => {
                status(tr!("Restarting"), busy, tr!("Steam is restarting. Your library comes in as soon as it is back."))
            }
            SteamState::NeedsRestart => status(
                tr!("Restart once"),
                theme::warning(),
                tr!("Steam was open before Heronite could prepare it. Restart it once to bring in your whole library."),
            ),
            SteamState::Failed => status(
                tr!("Could not connect"),
                theme::danger_text(),
                tr!("Steam restarted but did not let Heronite read its library. Its installed games still come in."),
            ),
            SteamState::Reading => status(tr!("Reading your library"), busy, tr!("Reading your Steam library\u{2026} a few seconds.")),
            SteamState::Connected => {
                status(tr!("Connected"), theme::success(), tr!("Steam is connected, but its library has no games yet."))
            }
        }
    }

    /// The Steam row's button and the note under it, for its state.
    fn steam_action(&self, cx: &mut Context<Self>) -> (Option<Stateful<Div>>, Option<(String, Rgba)>) {
        let state = Self::steam_state(cx);
        let error = crate::services::steam_library_started(cx).and_then(|s| s.read(cx).error.clone());
        let note = error.map(|e| (e, theme::danger_text()));
        let button = match state {
            SteamState::Closed => Some(
                button("welcome-steam-open", ButtonKind::Primary, tr!("Open Steam"))
                    .on_click(cx.listener(|_, _, _, cx| crate::services::steam_library(cx).update(cx, |s, cx| s.open(cx)))),
            ),
            SteamState::NeedsRestart if heronite_steam::can_restart() => Some(
                button("welcome-steam-restart", ButtonKind::Primary, tr!("Restart Steam"))
                    .on_click(cx.listener(|_, _, _, cx| crate::services::steam_library(cx).update(cx, |s, cx| s.restart(cx)))),
            ),
            SteamState::Failed => Some(
                button_sm("welcome-steam-page", ButtonKind::Ghost, tr!("Steam page"))
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(Page::Steam, window, cx))),
            ),
            _ => None,
        };
        let note = note.or_else(|| {
            (state == SteamState::NeedsRestart).then(|| {
                (
                    tr!("Games running from Steam close. Or wait: it comes in by itself the next time Steam opens.").to_string(),
                    theme::warning(),
                )
            })
        });
        (button, note)
    }

    fn welcome_left(&self, cx: &mut Context<Self>) -> Div {
        let (steam_button, steam_note) = self.steam_action(cx);
        let steam = source_row(Icon::CloudSync, "Steam".into(), self.steam_status(cx), steam_button, steam_note, true);
        let store = |id: &'static str, page: Page, name: &'static str, tint: (u8, u8, u8)| {
            tile(id, Icon::Storefront, name, tr!("Log in").into(), tint)
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx)))
        };
        let stores = div()
            .flex()
            .gap(px(10.))
            .child(store("welcome-epic", Page::Epic, "Epic Games", (226, 232, 240)))
            .child(store("welcome-gog", Page::Gog, "GOG", (185, 166, 255)))
            .child(store("welcome-amazon", Page::Amazon, "Amazon Games", (255, 196, 107)));
        let clients = self.client_tiles(cx);
        let add = div().mt(px(10.)).flex().child(icon_button_sm(
            "welcome-add",
            ButtonKind::Ghost,
            Icon::Add,
            tr!("Or add a game yourself"),
            true,
            cx.listener(|this, _, window, cx| {
                this.open_add_game(window, cx);
            }),
        ));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(LEFT_W))
            .max_w_full()
            .child(kicker(tr!("Your Collection is empty")))
            .child(
                div()
                    .mt(px(12.))
                    .font_family(theme::font_heading())
                    .text_size(px(40.))
                    .line_height(px(46.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(tr!("Let's bring in your games")),
            )
            .child(
                div()
                    .mt(px(10.))
                    .text_size(px(16.))
                    .line_height(px(23.))
                    .text_color(theme::text_muted())
                    .child(tr!("Heronite looks on this PC and brings in what it finds by itself.")),
            )
            .child(label(Icon::TravelExplore, tr!("Found on this PC")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(steam)
                    .when(heronite_platform::supported(Feature::Emulation), |d| d.child(self.emulation_row(cx))),
            )
            .when_some(heronite_platform::supported(Feature::LauncherImports).then(|| self.launcher_tiles(cx)).flatten(), |d, tiles| {
                d.child(label(Icon::CallMerge, tr!("Bring in from another launcher"))).child(tiles)
            })
            .when(heronite_platform::supported(Feature::StoreAccounts), |d| {
                d.child(label(Icon::Storefront, tr!("Log in to a store"))).child(stores)
            })
            .when(heronite_platform::supported(Feature::StoreClients), |d| d.child(clients))
            .child(add)
            .child(
                div()
                    .mt(px(30.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(theme::text_dim())
                    .child(icon(Icon::Lightbulb, px(14.), theme::text_dim()))
                    .child(tr!("Getting started opens by itself once your first games arrive.")),
            )
    }

    /// The stores that run their own program under Wine (Battle.net, the EA
    /// app, Ubisoft Connect, Rockstar Games): "Set up", or "Ready" once they
    /// are. A click opens their page.
    fn client_tiles(&self, cx: &mut Context<Self>) -> Div {
        let svc = crate::services::clients_started(cx);
        let tiles = Client::ALL.into_iter().map(|client| {
            let page = Page::of_client(client);
            let ready = svc.as_ref().is_some_and(|s| s.read(cx).state(client).installed());
            let tint = match client {
                Client::BattleNet => (110, 180, 255),
                Client::Ea => (255, 125, 125),
                Client::Ubisoft => (130, 225, 205),
                Client::Rockstar => (255, 205, 90),
            };
            let action = if ready { tr!("Ready") } else { tr!("Set up") };
            client_tile(SharedString::from(format!("welcome-{}", client.id())), page.icon(), page.title(), action, ready, tint)
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(page, window, cx)))
        });
        div().mt(px(10.)).flex().gap(px(8.)).children(tiles)
    }

    /// The launchers on this PC with games to bring in (Heroic, Lutris,
    /// Faugus): a click turns the import on, and the games come in.
    fn launcher_tiles(&self, cx: &mut Context<Self>) -> Option<Div> {
        let svc = crate::services::launcher_imports(cx);
        let s = svc.read(cx);
        let found: Vec<_> = s
            .found
            .iter()
            .flatten()
            .filter(|f| f.games > 0 && (!s.prefs.enabled.contains(&f.launcher) || s.busy.contains(&f.launcher)))
            .map(|f| (f.launcher, f.games, s.busy.contains(&f.launcher)))
            .collect();
        if found.is_empty() {
            return None;
        }
        let tiles = found.into_iter().map(|(launcher, games, busy)| {
            let action = if busy {
                tr!("Bringing in\u{2026}").to_string()
            } else {
                trn!(games, "{n} game \u{b7} Bring in", "{n} games \u{b7} Bring in")
            };
            let svc = svc.clone();
            tile(
                SharedString::from(format!("welcome-launcher-{}", launcher.id())),
                crate::pages::settings::launcher_icon(launcher),
                launcher.name(),
                action,
                (165, 220, 235),
            )
            .when(!busy, |t| t.on_click(move |_, _, cx| svc.update(cx, |s, cx| s.set_enabled(launcher, true, cx))))
        });
        Some(div().flex().gap(px(10.)).children(tiles))
    }

    fn emulation_row(&self, cx: &mut Context<Self>) -> Div {
        let setup = |label: &'static str| {
            Some(
                button_sm("welcome-emulators", ButtonKind::Glass, label)
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(Page::Emulators, window, cx))),
            )
        };
        let debug = debug_welcome().map(|(_, emu)| debug_emulation(&emu));
        let emulation = debug.as_ref().unwrap_or(&self.welcome.emulation);
        let looking = debug.as_ref().map_or(self.welcome.emulation_started, Option::is_none);
        let report = match emulation {
            None if looking => {
                let st = status(tr!("Looking"), theme::accent(), tr!("Looking for emulators and ROM folders on this PC\u{2026}"));
                return source_row(Icon::SportsEsports, tr!("Emulators and ROMs").into(), st, None, None, false);
            }
            Some(Ok(r)) => r.clone(),
            _ => SetupReport::default(),
        };
        if report.imported > 0 {
            let st = status(trn!(report.imported, "{n} game added", "{n} games added"), theme::success(), tr!("From your ROM folders."));
            return source_row(Icon::SportsEsports, tr!("Emulators and ROMs").into(), st, None, None, false);
        }
        if !report.unplayable.is_empty() {
            let line = tr!("{systems}: set up an emulator for them and they come in.", systems = report.unplayable.join(", "));
            let st = status(tr!("No emulator"), theme::warning(), line);
            return source_row(Icon::Folder, tr!("ROMs").into(), st, setup(tr!("Set up emulators")), None, false);
        }
        if !report.emulators.is_empty() {
            let st = status(tr!("Ready"), theme::success(), tr!("Ready to play. Add the folder with your ROMs and they come in."));
            return source_row(Icon::SportsEsports, report.emulators.join(", "), st, setup(tr!("Add ROM folder")), None, false);
        }
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(18.))
            .py(px(12.))
            .rounded(px(18.))
            .border_1()
            .border_dashed()
            .border_color(theme::ink_a(0.12))
            .text_size(px(13.5))
            .text_color(theme::text_dim())
            .child(icon(Icon::SportsEsports, px(18.), theme::text_dim()))
            .child(div().flex_1().child(tr!("No emulator or ROM folder on this PC.")))
            .child(
                button_sm("welcome-emulators", ButtonKind::Ghost, tr!("Set up emulators"))
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(Page::Emulators, window, cx))),
            )
    }
}
