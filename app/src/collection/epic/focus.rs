//! Epic items of the focus panel footer and of the context menu.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_epic::queue::ElementKind;
use serde_json::Value;

use super::{app_of, FocusEpic, Request};
use crate::collection::{context_menu, CollectionView};
use crate::icons::Icon;
use crate::services::downloads::{EosGame, QueueState};
use crate::services::widgets::{fmt_percent, progress_bar};
use crate::theme;
use crate::ui::icon;

fn menu_row(id: impl Into<ElementId>, ic: Icon, label: &str) -> Stateful<Div> {
    context_menu::item(id).child(icon(ic, px(18.), theme::text_muted())).child(label.to_string())
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
    /// Footer content of the focus panel for an Epic game.
    pub(in crate::collection) fn epic_focus(&self, key: &str, installed: bool, cx: &mut Context<Self>) -> Option<FocusEpic> {
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


    /// Appends the Epic items to the context menu of `game`.
    pub(in crate::collection) fn epic_context_items(&mut self, mut menu: Div, game: &CollectionGame, cx: &mut Context<Self>) -> Div {
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
        // The Collection's game model leaves these out; the service read them from the library file.
        let flags = svc.read(cx).game_flags(&game.app_name);
        let (third_party, has_dlcs) = (flags.third_party, flags.has_dlcs);
        let add = |menu: Div, id: &str, ic: Icon, label: &str, req: Box<dyn Fn(&str) -> Option<Request>>, cx: &mut Context<Self>| {
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
        // EA app / Ubisoft Connect keep their games: legendary cannot import,
        // update, verify or move them (Heroic hides these too).
        if !game.installed {
            if !third_party {
                menu = add(
                    menu,
                    "import",
                    Icon::FolderOpen,
                    "Import game",
                    Box::new(|k| Some(Request::Install { key: k.to_string(), import: true })),
                    cx,
                );
            }
            return menu;
        }
        if busy || queued {
            return menu;
        }
        if !third_party {
            if update {
                menu = add(menu, "update", Icon::Update, "Update", Box::new(|_| None), cx);
            }
            menu = add(menu, "verify", Icon::Build, "Verify and repair", Box::new(|k| Some(Request::Verify(k.to_string()))), cx);
            menu = add(menu, "move", Icon::Folder, "Move install", Box::new(|k| Some(Request::Move(k.to_string()))), cx);
        }
        if has_dlcs && !third_party {
            menu = add(menu, "modify", Icon::Tune, "Modify installation", Box::new(|k| Some(Request::Modify(k.to_string()))), cx);
        }
        // The overlay is enabled per Wine prefix on Linux only.
        if cfg!(target_os = "linux") && !third_party {
            let app = game.app_name.clone();
            svc.update(cx, |s, cx| s.eos_load_game(&app, cx));
            let state = svc.read(cx).eos_game(&game.app_name);
            if let Some(state @ (EosGame::Enabled | EosGame::Disabled)) = state {
                let label = if state == EosGame::Enabled { "Disable EOS Overlay" } else { "Enable EOS Overlay" };
                menu = add(menu, "eos", Icon::Fullscreen, label, Box::new(|k| Some(Request::EosToggle(k.to_string()))), cx);
            }
        }
        if cloud {
            menu = add(menu, "sync", Icon::CloudSync, "Cloud saves sync", Box::new(|_| None), cx);
        }
        add(menu, "uninstall", Icon::DeleteForever, "Uninstall", Box::new(|k| Some(Request::Uninstall(k.to_string()))), cx)
    }
}
