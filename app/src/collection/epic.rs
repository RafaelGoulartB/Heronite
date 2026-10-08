//! Epic Games glue of the Collection: install / import dialogs, progress on
//! cards and in the focus panel footer, the Epic items of the context menu
//! and the confirmations behind them. The work itself is done by
//! `services::Downloads` (download queue and operations).

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::CollectionGame;
use heronite_epic::queue::ElementKind;
use serde_json::Value;

use super::CollectionView;
use crate::dialogs::epic_confirm::{Confirm, EpicConfirmDialog, Toggle};
use crate::dialogs::epic_install::{EpicInstallDialog, Mode};
use crate::icons::Icon;
use crate::pages::Page;
use crate::services::downloads::{Downloads, DownloadsEvent, QueueState};
use crate::services::widgets::{fmt_percent, progress_bar};
use crate::theme;
use crate::ui::icon;

/// Something a click asked for that needs a `Window` (handled from `render`).
pub enum Request {
    Install { key: String, import: bool },
    Uninstall(String),
    Move(String),
    Verify(String),
    UpdatePrompt(String),
    Message { title: String, text: String },
    OpenDownloads,
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

fn menu_row(id: impl Into<ElementId>, ic: Icon, label: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(6.))
        .min_h(px(36.))
        .text_size(px(16.))
        .text_color(theme::accent())
        .cursor_pointer()
        .hover(|s| s.bg(theme::navbar_active_background()))
        .child(icon(ic, px(20.), rgb(0xcaf3fd)))
        .child(label.to_string())
}

fn round_btn(id: impl Into<ElementId>, ic: Icon) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .bg(theme::rgb_a(16, 18, 22, 0.72))
        .hover(|s| s.bg(theme::rgb_a(10, 12, 14, 0.9)))
        .child(icon(ic, px(16.), rgb(0xf4f6f8)))
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

    /// Footer content of the focus panel for an Epic game.
    pub(super) fn epic_focus(&self, key: &str, installed: bool, cx: &mut Context<Self>) -> Option<FocusEpic> {
        let app = app_of(key)?.to_string();
        let svc = self.epic.service.as_ref()?.clone();
        let s = svc.read(cx);
        let active = s.busy_of(&app).cloned();
        let queue = s.queue_state(&app);
        let progress = s.progress_of(&app).cloned();
        let update = installed && s.has_update(&app);
        let kind = s.queued(&app).map(|(_, e)| e.kind);
        let running = s.is_running();
        let first = s.queued(&app).is_some_and(|(i, _)| i == 0);
        if active.is_some() || queue.is_some() {
            let (label, percent) = match (&active, queue) {
                (Some(b), _) => (
                    match b.percent {
                        Some(p) => format!("{} {}", b.label, fmt_percent(p as f64)),
                        None => format!("{}...", b.label),
                    },
                    b.percent,
                ),
                (None, Some(QueueState::Waiting)) => ("Queued".to_string(), None),
                (None, Some(state)) => {
                    let verb = if kind == Some(ElementKind::Update) { "Updating" } else { "Installing" };
                    let verb = if state == QueueState::Paused { "Paused" } else { verb };
                    match &progress {
                        Some(p) => (
                            format!(
                                "{verb} {} - {:.1} MiB/s{}",
                                fmt_percent(p.percent),
                                p.down_speed,
                                if p.eta.is_empty() { String::new() } else { format!(" - ETA {}", p.eta) }
                            ),
                            Some(p.percent as f32),
                        ),
                        None => (format!("{verb}..."), None),
                    }
                }
                (None, None) => (String::new(), None),
            };
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .w(px(250.))
                        .child(div().text_size(px(13.33)).text_color(white()).child(label))
                        .child(progress_bar(percent, 5., theme::accent())),
                );
            if queue.is_some() && active.is_none() && !first {
                // Waiting behind other downloads: it can only be taken out of the queue.
                let remove_app = app.clone();
                row = row
                    .child(round_btn("epic-focus-remove", Icon::Close).on_click(cx.listener(move |this, _, _, cx| {
                        let svc = this.epic_service(cx);
                        let a = remove_app.clone();
                        svc.update(cx, |s, _| s.remove_from_queue(&a));
                    })))
                    .child(round_btn("epic-focus-open", Icon::Downloading).on_click(cx.listener(|this, _, _, cx| {
                        this.epic.pending = Some(Request::OpenDownloads);
                        cx.notify();
                    })));
            } else if queue.is_some() && active.is_none() {
                let paused = !running;
                row = row
                    .child(
                        round_btn("epic-focus-pause", if paused { Icon::PlayArrow } else { Icon::Pause }).on_click(cx.listener(
                            move |this, _, _, cx| {
                                let svc = this.epic_service(cx);
                                svc.update(cx, |s, _| if s.is_running() { s.pause() } else { s.resume() });
                            },
                        )),
                    )
                    .child(round_btn("epic-focus-open", Icon::Downloading).on_click(cx.listener(|this, _, _, cx| {
                        this.epic.pending = Some(Request::OpenDownloads);
                        cx.notify();
                    })));
            } else if let Some(b) = active {
                if let Some(cancel) = b.cancel {
                    row = row.child(round_btn("epic-focus-cancel", Icon::Close).on_click(move |_, _, _| cancel.request()));
                }
            }
            return Some(FocusEpic { element: row.into_any_element(), replaces_play: true });
        }
        if update {
            let el = div()
                .id("epic-focus-update")
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(16.))
                .py(px(8.32))
                .rounded_full()
                .cursor_pointer()
                .bg(theme::accent())
                .hover(|s| s.bg(theme::accent_overlay()))
                .text_color(rgb(0x161616))
                .font_weight(theme::W_BOLD)
                .text_size(px(15.2))
                .child(icon(Icon::Update, px(19.2), rgb(0x161616)))
                .child("Update")
                .on_click(cx.listener(move |this, _, _, cx| this.epic_update(&format!("legendary_{app}"), cx)))
                .into_any_element();
            return Some(FocusEpic { element: el, replaces_play: false });
        }
        None
    }

    fn epic_update(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        let info = Self::epic_game_info(&game);
        let svc = self.epic_service(cx);
        svc.update(cx, |s, cx| s.queue_update(info, cx));
    }

    /// Appends the Epic items to the context menu of `game`.
    pub(super) fn epic_context_items(&mut self, mut menu: Div, game: &CollectionGame, cx: &mut Context<Self>) -> Div {
        if game.runner != "legendary" {
            return menu;
        }
        let svc = self.epic_service(cx);
        let (busy, queued, update) = {
            let s = svc.read(cx);
            (s.busy_of(&game.app_name).is_some(), s.queued(&game.app_name).is_some(), s.has_update(&game.app_name))
        };
        let key = game.key.clone();
        let cloud = game
            .game
            .rest
            .get("cloud_save_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut add = |menu: Div, id: &str, ic: Icon, label: &str, req: Box<dyn Fn(&str) -> Option<Request>>, cx: &mut Context<Self>| {
            let k = key.clone();
            menu.child(menu_row(SharedString::from(format!("cm-epic-{id}")), ic, label).on_click(cx.listener(move |this, _, _, cx| {
                this.context_menu = None;
                match req(&k) {
                    Some(r) => this.epic.pending = Some(r),
                    None => this.epic_direct(&k, cx),
                }
                cx.notify();
            })))
        };
        if queued {
            menu = add(menu, "downloads", Icon::Downloading, "Show in Downloads", Box::new(|_| Some(Request::OpenDownloads)), cx);
        }
        if !game.installed {
            menu = add(
                menu,
                "import",
                Icon::FolderOpen,
                "Import game",
                Box::new(|k| Some(Request::Install { key: k.to_string(), import: true })),
                cx,
            );
            return menu;
        }
        if busy || queued {
            return menu;
        }
        if update {
            menu = add(menu, "update", Icon::Update, "Update", Box::new(|_| None), cx);
        }
        menu = add(menu, "verify", Icon::Build, "Verify and repair", Box::new(|k| Some(Request::Verify(k.to_string()))), cx);
        menu = add(menu, "move", Icon::Folder, "Move install", Box::new(|k| Some(Request::Move(k.to_string()))), cx);
        if cloud {
            menu = add(menu, "sync", Icon::CloudSync, "Cloud saves sync", Box::new(|_| None), cx);
        }
        add(menu, "uninstall", Icon::DeleteForever, "Uninstall", Box::new(|k| Some(Request::Uninstall(k.to_string()))), cx)
    }

    /// Context-menu actions that need no dialog (the row id tells which).
    fn epic_direct(&mut self, key: &str, cx: &mut Context<Self>) {
        // `update` and `sync`: decided by what the game offers.
        let Some(game) = self.epic_game(key) else { return };
        let svc = self.epic_service(cx);
        let update = svc.read(cx).has_update(&game.app_name);
        if update {
            self.epic_update(key, cx);
        } else {
            let (app, title) = (game.app_name.clone(), game.title.clone());
            svc.update(cx, |s, cx| s.sync_saves(&app, &title, cx));
        }
    }

    fn epic_open_install(&mut self, key: &str, import: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        let info = Self::epic_game_info(&game);
        let (heroic, app) = (self.heroic.clone(), self.app_paths.clone());
        let mode = if import { Mode::Import } else { Mode::Install };
        let view = cx.new(|cx| EpicInstallDialog::new(heroic, app, info, mode, window, cx));
        self.open_dialog(view, cx);
    }

    fn epic_busy_guard(&mut self, cx: &mut Context<Self>, what: &str) -> bool {
        let svc = self.epic_service(cx);
        let downloading = svc.read(cx).snapshot().state == heronite_epic::manager::DmState::Running;
        if downloading {
            self.show_toast(format!("{what} is not possible while an Epic game is being installed."), true, cx);
        }
        downloading
    }

    fn epic_open_uninstall(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        if self.epic_busy_guard(cx, "Uninstalling") {
            return;
        }
        if self.playing.contains_key(&game.app_name) {
            self.show_toast(format!("{} is running. Close the game to uninstall it.", game.title), true, cx);
            return;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let settings = game_config::load_game(&self.heroic, &game.app_name, &home);
        let global = game_config::load_global(&self.heroic, &home);
        let prefix = settings.get("winePrefix").and_then(Value::as_str).unwrap_or_default().to_string();
        let default_prefix = global.get("winePrefix").and_then(Value::as_str).unwrap_or_default().to_string();
        let windows = game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("windows"));
        let mut spec = Confirm::new("epic-uninstall", "Uninstall", format!("Do you want to uninstall \"{}\"?", game.title));
        if windows && cfg!(target_os = "linux") && !prefix.is_empty() {
            let protect = prefix == default_prefix;
            spec.toggles.push(Toggle {
                label: format!("Remove prefix: {prefix}"),
                value: false,
                disabled: protect,
                note: Some(if protect {
                    "The Wine prefix for this game is the default prefix. If you really want to delete it, you have to do it manually.".into()
                } else {
                    "Note: This can't be undone and will also remove not backed up save files.".into()
                }),
            });
        }
        spec.toggles.push(Toggle {
            label: "Erase settings and remove log".into(),
            value: false,
            disabled: false,
            note: Some("Note: This can't be undone. Any modified settings will be forgotten and log will be deleted.".into()),
        });
        spec.confirm = Some("Uninstall".into());
        spec.danger = true;
        let has_prefix_toggle = spec.toggles.len() == 2;
        let app = game.app_name.clone();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |choice, cx| {
                    let (prefix, settings) = if has_prefix_toggle {
                        (choice.toggles[0], choice.toggles[1])
                    } else {
                        (false, choice.toggles[0])
                    };
                    let svc = crate::services::downloads(cx);
                    let app = app.clone();
                    svc.update(cx, |s, cx| s.uninstall(&app, prefix, settings, cx));
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    fn epic_open_move(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        if self.epic_busy_guard(cx, "Moving a game") {
            return;
        }
        let Some(current) = game.game.install.install_path.clone() else { return };
        let mut spec = Confirm::new(
            "epic-move",
            "Move Game Installation",
            format!("{}\nThis can take a long time, are you sure?", game.title),
        );
        let parent = std::path::Path::new(&current).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        spec.path = Some(("Choose where you want to move".into(), parent, "Choose where you want to move".into()));
        spec.confirm = Some("Move".into());
        let app = game.app_name.clone();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |choice, cx| {
                    let svc = crate::services::downloads(cx);
                    let (app, from) = (app.clone(), current.clone());
                    svc.update(cx, |s, cx| s.move_install(&app, &from, &choice.path, cx));
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    fn epic_open_verify(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        if self.epic_busy_guard(cx, "Verifying a game") {
            return;
        }
        let mut spec = Confirm::new(
            "epic-verify",
            "Verify and Repair",
            format!("{}\nDo you want to try to repair this game? This can take a long time.", game.title),
        );
        spec.confirm = Some("Verify".into());
        let app = game.app_name.clone();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |_, cx| {
                    let svc = crate::services::downloads(cx);
                    let app = app.clone();
                    svc.update(cx, |s, cx| s.verify(&app, cx));
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    fn epic_open_update_prompt(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        let mut spec = Confirm::new(
            "epic-update",
            "Game Needs Update",
            format!("{}\nThis game has an update, apply update now?", game.title),
        );
        spec.confirm = Some("Update".into());
        let weak = cx.entity().downgrade();
        let k = key.to_string();
        let k2 = key.to_string();
        let weak2 = weak.clone();
        spec.secondary = Some((
            "Play anyway".into(),
            Rc::new(move |cx| {
                weak2
                    .update(cx, |this, cx| {
                        if let Some(g) = this.epic_game(&k2) {
                            if let Err(err) = this.runtime.play(&g) {
                                this.show_toast(err, true, cx);
                            }
                        }
                    })
                    .ok();
            }),
        ));
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |_, cx| {
                    let k = k.clone();
                    weak.update(cx, |this, cx| this.epic_update(&k, cx)).ok();
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    /// `HERONITE_DEBUG_DIALOG=epic-install:<appName>` / `epic-import:<appName>` /
    /// `epic-uninstall:<appName>` / `epic-move:..` / `epic-verify:..` /
    /// `epic-update:..` / `epic-message`.
    pub(super) fn epic_debug_dialog(&mut self, which: &str, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((kind, app)) = which.split_once(':').map(|(k, a)| (k, a.to_string())).or_else(|| which.starts_with("epic-").then(|| (which, String::new()))) else {
            return false;
        };
        let key = format!("legendary_{app}");
        self.epic.pending = match kind {
            "epic-install" => Some(Request::Install { key, import: false }),
            "epic-import" => Some(Request::Install { key, import: true }),
            "epic-uninstall" => Some(Request::Uninstall(key)),
            "epic-move" => Some(Request::Move(key)),
            "epic-verify" => Some(Request::Verify(key)),
            "epic-update" => Some(Request::UpdatePrompt(key)),
            "epic-message" => Some(Request::Message {
                title: "Cloud saves: Control".into(),
                text: "[cli] INFO: Cloud save sync for Control\n[cli] INFO: Remote save is newer, downloading 3 files\n[cli] INFO: Download finished".into(),
            }),
            _ => return false,
        };
        cx.notify();
        true
    }
}

use std::sync::Arc;
