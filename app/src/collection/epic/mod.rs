//! Epic Games glue of the Collection: install / import dialogs, progress on
//! cards and in the focus panel footer, the Epic items of the context menu
//! and the confirmations behind them. The work itself is done by
//! `services::Downloads` (download queue and operations).

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
use crate::services::widgets::{fmt_percent, progress_bar};
use crate::theme;

/// Something a click asked for that needs a `Window` (handled from `render`).
pub enum Request {
    Install { key: String, import: bool },
    Uninstall(String),
    Move(String),
    Verify(String),
    UpdatePrompt(String),
    Message { title: String, text: String },
    OpenDownloads,
    /// Modify installation (DLCs).
    Modify(String),
    /// "The EOS Overlay is not installed. Do you want to install it now?"
    EosInstall(String),
    /// Enable / disable the EOS Overlay for the game's prefix.
    EosToggle(String),
}

#[derive(Default)]
pub struct EpicState {
    service: Option<Entity<Downloads>>,
    subs: Vec<Subscription>,
    pending: Option<Request>,
}

/// What a card shows on top of its cover.
pub struct CardEpic {
    /// `Installing 37%`, `Queued`, `Verifying 12%`; empty when only the
    /// update hint is shown.
    pub label: SharedString,
    pub percent: Option<f32>,
    pub update: bool,
}

/// Extra footer content of the focus panel.
pub struct FocusEpic {
    pub element: AnyElement,
    /// The Play/Install button is replaced by `element`.
    pub replaces_play: bool,
}

fn app_of(key: &str) -> Option<&str> {
    key.strip_prefix("legendary_")
}

/// Card overlay: bottom strip with label + bar, "Update" pill at the top.
pub fn card_overlay(e: &CardEpic) -> Div {
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
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(theme::black_a(0.), 0.),
                    linear_color_stop(theme::black_a(0.78), 1.),
                ))
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
                .child("UPDATE"),
        );
    }
    layer
}


impl CollectionView {
    /// The service, created and observed on first use.
    fn epic_service(&mut self, cx: &mut Context<Self>) -> Entity<Downloads> {
        if let Some(s) = &self.epic.service {
            return s.clone();
        }
        let svc = crate::services::downloads(cx);
        self.epic.subs.push(cx.observe(&svc, |_, _, cx| cx.notify()));
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

    /// Play/Install button of an Epic game. Returns `true` when handled here:
    /// uninstalled games open the install dialog, games in the queue open the
    /// Downloads page, outdated games ask to update first.
    pub(super) fn epic_primary(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> bool {
        if game.runner != "legendary" {
            return false;
        }
        let svc = self.epic_service(cx);
        let (queued, busy, update) = {
            let s = svc.read(cx);
            (s.queued(&game.app_name).is_some(), s.busy_of(&game.app_name).is_some(), s.has_update(&game.app_name))
        };
        if queued || busy {
            self.epic.pending = Some(Request::OpenDownloads);
        } else if !game.installed {
            self.epic.pending = Some(Request::Install { key: game.key.clone(), import: false });
        } else if update {
            self.epic.pending = Some(Request::UpdatePrompt(game.key.clone()));
        } else {
            return false;
        }
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
            let verb = if kind == Some(ElementKind::Update) { "Updating" } else { "Installing" };
            let p = svc.progress_of(app).map(|p| p.percent as f32);
            let label = match (state, p) {
                (QueueState::Downloading, Some(p)) => format!("{verb} {}", fmt_percent(p as f64)),
                (QueueState::Downloading, None) => format!("{verb}..."),
                (QueueState::Paused, Some(p)) => format!("Paused {}", fmt_percent(p as f64)),
                (QueueState::Paused, None) => "Paused".into(),
                (QueueState::Waiting, _) => "Queued".into(),
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
