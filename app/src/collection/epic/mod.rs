//! Store glue of the Collection (Epic Games, GOG, Amazon Games): install /
//! import dialogs, progress on cards and in the focus panel footer, the store
//! items of the context menu and the confirmations behind them. The work
//! itself is done by `services::Downloads` (download queue and operations).

mod console;
mod dialogs;
mod focus;

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_epic::queue::ElementKind;
use serde_json::Value;

use super::CollectionView;
use crate::dialogs::epic_confirm::{Confirm, EpicConfirmDialog};
use crate::pages::Page;
use crate::services::downloads::{Downloads, DownloadsEvent, QueueState};
use crate::services::launch_blocker::{launch_blocker, Blocker};
use crate::services::widgets::{fmt_percent, progress_bar};
use crate::theme;
use heronite_i18n::tr;

/// Something a click asked for that needs a `Window` (handled from `render`).
pub enum Request {
    Install {
        key: String,
        import: bool,
    },
    Uninstall(String),
    Move(String),
    Verify(String),
    UpdatePrompt(String),
    Message {
        title: String,
        text: String,
    },
    OpenDownloads,
    /// Modify installation (DLCs).
    Modify(String),
    /// "The EOS Overlay is not installed. Do you want to install it now?"
    EosInstall(String),
    /// Enable / disable the EOS Overlay for the game's prefix.
    EosToggle(String),
    /// Point the install at the folder the game was moved to.
    Relocate(String),
    /// Take the game off the installed games, keeping its files.
    Forget(String),
    /// Play was pressed but the game's folder is gone.
    FolderMissing(String),
}

/// The Epic items of a game's actions (context menu and palette).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EpicAction {
    ShowInDownloads,
    Import,
    Update,
    Verify,
    Move,
    Modify,
    /// Enable / disable the EOS Overlay.
    Eos,
    /// Cloud saves sync.
    Sync,
    /// The folder was moved by hand: tell the store where it is.
    Relocate,
    /// The folder is gone: off the installed games.
    Forget,
    Uninstall,
}

#[derive(Default)]
pub(super) struct EpicState {
    service: Option<Entity<Downloads>>,
    subs: Vec<Subscription>,
    pub(super) pending: Option<Request>,
}

/// What a card shows on top of its cover.
pub(super) struct CardEpic {
    /// `Installing 37%`, `Queued`, `Verifying 12%`; empty when only the
    /// update hint is shown.
    pub label: SharedString,
    pub percent: Option<f32>,
    pub update: bool,
}

/// Extra footer content of the focus panel.
pub(super) struct FocusEpic {
    pub element: AnyElement,
    /// The Play/Install button is replaced by `element`.
    pub replaces_play: bool,
}

/// What a queued game is doing: "Installing", "Updating" or "Paused".
fn queue_verb(kind: Option<ElementKind>, paused: bool) -> &'static str {
    if paused {
        tr!("Paused")
    } else if kind == Some(ElementKind::Update) {
        tr!("Updating")
    } else {
        tr!("Installing")
    }
}

/// The app name of a store game's key (`<runner>_<appName>`).
fn app_of(key: &str) -> Option<&str> {
    ["legendary_", "gog_", "nile_"].iter().find_map(|prefix| key.strip_prefix(prefix))
}

/// Games of a store that installs through the download queue.
pub(super) fn is_store_runner(runner: &str) -> bool {
    matches!(runner, "legendary" | "gog" | "nile")
}

/// Card overlay: bottom strip with label + bar, "Update" pill at the top.
pub(super) fn card_overlay(e: &CardEpic) -> Div {
    let mut layer = div().absolute().inset_0().rounded(px(6.)).overflow_hidden();
    if !e.label.is_empty() {
        layer = layer.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .px(px(6.))
                .pt(px(14.))
                .pb(px(6.))
                .bg(linear_gradient(180., linear_color_stop(theme::black_a(0.), 0.), linear_color_stop(theme::black_a(0.78), 1.)))
                .child(div().text_size(px(11.5)).font_weight(theme::W_BOLD).text_color(white()).child(e.label.clone()))
                .child(progress_bar(e.percent, 3., theme::accent())),
        );
    }
    if e.update {
        layer = layer.child(
            div()
                .absolute()
                .top(px(6.))
                .right(px(6.))
                .px(px(6.))
                .py(px(1.5))
                .rounded_full()
                .bg(theme::accent())
                .text_color(rgb(0x161616))
                .text_size(px(10.5))
                .font_weight(theme::W_BOLD)
                .child(tr!("UPDATE")),
        );
    }
    layer
}

/// The game keeps cloud saves its store can sync: Epic says so in its
/// library; GOG keeps them for its Windows builds (whether this game has any
/// is known when the sync runs).
pub(super) fn has_cloud_saves(game: &CollectionGame) -> bool {
    match game.runner.as_str() {
        "gog" => game.store_installed() && game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("windows")),
        "legendary" => game.game.rest.get("cloud_save_enabled").and_then(Value::as_bool).unwrap_or(false),
        _ => false,
    }
}

impl CollectionView {
    /// Cloud saves sync of a store game (Epic or GOG), from a menu.
    pub(super) fn epic_sync_saves(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        let (app, title) = (game.app_name.clone(), game.title.clone());
        let gog = game.runner == "gog";
        self.epic_service(cx).update(cx, |s, cx| {
            if gog {
                s.gog_sync_saves(&app, &title, heronite_gog::saves::SyncMode::Both, cx);
            } else {
                s.sync_saves(&app, &title, cx);
            }
        });
    }

    /// The service, created and observed on first use.
    fn epic_service(&mut self, cx: &mut Context<Self>) -> Entity<Downloads> {
        if let Some(s) = &self.epic.service {
            return s.clone();
        }
        let svc = crate::services::downloads(cx);
        self.epic.subs.push(cx.observe(&svc, |this, svc, cx| {
            this.sync_update_apps(&svc, cx);
            cx.notify();
        }));
        self.epic.subs.push(cx.subscribe(&svc, |this, _, ev: &DownloadsEvent, cx| {
            match ev {
                DownloadsEvent::LibraryChanged => this.reload(cx),
                DownloadsEvent::Toast(text, error) => this.show_toast(text.clone(), *error, cx),
                DownloadsEvent::Message { title, text } => {
                    this.epic.pending = Some(Request::Message { title: title.clone(), text: text.clone() });
                }
                DownloadsEvent::EosNotInstalled { app_name } => {
                    this.epic.pending = Some(Request::EosInstall(app_name.clone()));
                }
            }
            cx.notify();
        }));
        self.epic.service = Some(svc.clone());
        svc
    }

    /// Opens what clicks asked for (needs a `Window`, so it runs from `render`).
    pub(super) fn drive_epic(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.epic_service(cx);
        let Some(req) = self.epic.pending.take() else { return };
        match req {
            Request::OpenDownloads => self.navigate(Page::Downloads, window, cx),
            Request::Install { key, import } => self.epic_open_install(&key, import, window, cx),
            Request::Uninstall(key) => self.epic_open_uninstall(&key, window, cx),
            Request::Move(key) => self.epic_open_move(&key, window, cx),
            Request::Verify(key) => self.epic_open_verify(&key, window, cx),
            Request::UpdatePrompt(key) => self.epic_open_update_prompt(&key, window, cx),
            Request::Modify(key) => self.epic_open_modify(&key, cx),
            Request::Relocate(key) => self.epic_open_relocate(&key, false, window, cx),
            Request::FolderMissing(key) => self.epic_open_relocate(&key, true, window, cx),
            Request::Forget(key) => self.epic_open_forget(&key, window, cx),
            Request::EosInstall(app) => self.epic_open_eos_install(&app, window, cx),
            Request::EosToggle(key) => {
                if let Some(game) = self.epic_game(&key) {
                    let app = game.app_name.clone();
                    self.epic_service(cx).update(cx, |s, cx| s.eos_toggle_game(&app, cx));
                }
            }
            Request::Message { title, text } => {
                let mut spec = Confirm::new("epic-message", title, "");
                spec.text = Some(text);
                spec.width = 620.;
                let view = cx.new(|cx| EpicConfirmDialog::new(spec, |_, _| {}, window, cx));
                self.open_dialog(view, cx);
            }
        }
    }

    fn epic_game(&self, key: &str) -> Option<Arc<CollectionGame>> {
        self.lib.as_ref()?.get(key).cloned()
    }

    fn epic_game_info(game: &CollectionGame) -> Value {
        serde_json::to_value(&*game.game).unwrap_or(Value::Null)
    }

    /// The store can import an existing install of `game` (not EA app /
    /// Ubisoft Connect games, which live in their store's prefix).
    pub(super) fn epic_can_import(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> bool {
        if !is_store_runner(&game.runner) || game.store_installed() {
            return false;
        }
        let svc = self.epic_service(cx);
        let third_party = svc.read(cx).game_flags(&game.app_name).third_party;
        !third_party
    }

    /// Play/Install button of a store game. Returns `true` when handled here:
    /// uninstalled games open the install dialog, games in the queue open the
    /// Downloads page, outdated games ask to update first.
    pub(super) fn epic_primary(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> bool {
        if !is_store_runner(&game.runner) {
            return false;
        }
        // Starts the service, which the blocker reads.
        self.epic_service(cx);
        self.epic.pending = Some(match launch_blocker(cx, game) {
            Some(b) if b.is_busy() => Request::OpenDownloads,
            None if !game.installed => Request::Install { key: game.key.clone(), import: false },
            Some(Blocker::FolderMissing) => Request::FolderMissing(game.key.clone()),
            Some(_) => Request::UpdatePrompt(game.key.clone()),
            None => return false,
        });
        cx.notify();
        true
    }

    /// Overlay data of a card, if the game is being installed / updated /
    /// verified or has an update.
    pub(super) fn epic_card(&self, key: &str, cx: &App) -> Option<CardEpic> {
        let app = app_of(key)?;
        let svc = self.epic.service.as_ref()?.read(cx);
        if let Some(b) = svc.busy_of(app) {
            let label = match b.percent {
                Some(p) => format!("{} {}", b.label, fmt_percent(p as f64)),
                None => format!("{}...", b.label),
            };
            return Some(CardEpic { label: label.into(), percent: b.percent, update: false });
        }
        if let Some(state) = svc.queue_state(app) {
            let kind = svc.queued(app).map(|(_, e)| e.kind);
            let verb = queue_verb(kind, false);
            let p = svc.progress_of(app).map(|p| p.percent as f32);
            let label = match (state, p) {
                (QueueState::Downloading, Some(p)) => format!("{verb} {}", fmt_percent(p as f64)),
                (QueueState::Downloading, None) => format!("{verb}..."),
                (QueueState::Paused, Some(p)) => format!("{} {}", queue_verb(kind, true), fmt_percent(p as f64)),
                (QueueState::Paused, None) => queue_verb(kind, true).into(),
                (QueueState::Waiting, _) => tr!("Queued").into(),
            };
            return Some(CardEpic { label: label.into(), percent: p, update: false });
        }
        svc.has_update(app).then(|| CardEpic { label: SharedString::default(), percent: None, update: true })
    }

    fn epic_update(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        let info = Self::epic_game_info(&game);
        let svc = self.epic_service(cx);
        svc.update(cx, |s, cx| s.queue_update(info, cx));
    }
}
