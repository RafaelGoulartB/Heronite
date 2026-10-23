//! Store items of the focus panel footer and of the game actions (context
//! menu, palette).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;

use super::{app_of, EpicAction, FocusEpic, Request};
use crate::collection::game_actions::{GameAction, GameActionItem};
use crate::collection::CollectionView;
use crate::icons::Icon;
use crate::services::downloads::{EosGame, QueueState};
use crate::services::widgets::{fmt_percent, progress_bar};
use crate::theme;
use crate::ui::icon;
use heronite_i18n::tr;

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
        .bg(theme::round_fill(0.72))
        .hover(|s| s.bg(theme::round_fill_hover()))
        .child(icon(ic, px(16.), theme::round_icon()))
}
impl CollectionView {
    /// Footer content of the focus panel for a store game.
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
                (None, Some(QueueState::Waiting)) => (tr!("Queued").to_string(), None),
                (None, Some(state)) => {
                    let verb = super::queue_verb(kind, state == QueueState::Paused);
                    match &progress {
                        Some(p) => {
                            let (percent, speed) = (fmt_percent(p.percent), heronite_i18n::decimal(p.down_speed, 1));
                            let text = if p.eta.is_empty() {
                                tr!("{verb} {percent} - {speed} MiB/s", verb, percent, speed)
                            } else {
                                tr!("{verb} {percent} - {speed} MiB/s - ETA {eta}", verb, percent, speed, eta = p.eta.as_str())
                            };
                            (text, Some(p.percent as f32))
                        }
                        None => (format!("{verb}..."), None),
                    }
                }
                (None, None) => (String::new(), None),
            };
            let mut row = div().flex().w_full().items_center().gap(px(10.)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.33)).text_color(theme::text_bright()).child(label))
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
                    .child(round_btn("epic-focus-pause", if paused { Icon::PlayArrow } else { Icon::Pause }).on_click(cx.listener(
                        move |this, _, _, cx| {
                            let svc = this.epic_service(cx);
                            svc.update(cx, |s, _| if s.is_running() { s.pause() } else { s.resume() });
                        },
                    )))
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
            let key = key.to_string();
            let el = div()
                .id("epic-focus-update")
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .px(px(16.))
                .py(px(8.32))
                .rounded_full()
                .cursor_pointer()
                .bg(theme::accent())
                .hover(|s| s.bg(theme::accent_overlay()))
                .text_color(theme::on_accent())
                .font_weight(theme::W_BOLD)
                .text_size(px(15.2))
                .child(icon(Icon::Update, px(19.2), theme::on_accent()))
                .child(tr!("Update"))
                .on_click(cx.listener(move |this, _, _, cx| this.epic_update(&key, cx)))
                .into_any_element();
            return Some(FocusEpic { element: el, replaces_play: false });
        }
        None
    }

    /// The store actions `game` offers now (none for games of no store).
    pub(in crate::collection) fn epic_actions(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> Vec<GameActionItem> {
        let mut v = Vec::new();
        if !super::is_store_runner(&game.runner) {
            return v;
        }
        let svc = self.epic_service(cx);
        let (busy, queued, update) = {
            let s = svc.read(cx);
            (s.busy_of(&game.app_name).is_some(), s.queued(&game.app_name).is_some(), s.has_update(&game.app_name))
        };
        let cloud = super::has_cloud_saves(game);
        // Moving or verifying the files of a running game would pull them from under it.
        let playing = self.playing.contains_key(&game.app_name);
        // The Collection's game model leaves these out; the service read them from the library file.
        let flags = svc.read(cx).game_flags(&game.app_name);
        let (third_party, has_dlcs) = (flags.third_party, flags.has_dlcs);
        let item = |a: EpicAction, id: &str, ic: Icon, label: &str, kw: &'static str| {
            GameActionItem::new(GameAction::Epic(a), format!("epic-{id}"), label.to_string(), ic, kw)
        };
        if queued {
            v.push(item(
                EpicAction::ShowInDownloads,
                "downloads",
                Icon::Downloading,
                tr!("Show in Downloads"),
                "queue progress fila progresso downloads",
            ));
        }
        // EA app / Ubisoft Connect keep their games: legendary cannot import,
        // update, verify or move them (Heroic hides these too).
        if !game.store_installed() {
            // Otherwise it is in the Install submenu (`install_options`).
            if !third_party && !crate::collection::install_options::has_more_ways(game) {
                v.push(item(
                    EpicAction::Import,
                    "import",
                    Icon::FolderOpen,
                    tr!("Import game"),
                    "existing installed copy importar jogo copia instalada",
                ));
            }
            return v;
        }
        if busy || queued {
            return v;
        }
        // The folder is gone: only telling where it went, or forgetting it, can work.
        if game.install_folder_missing() {
            v.push(item(
                EpicAction::Relocate,
                "relocate",
                Icon::FolderOpen,
                tr!("Change install location\u{2026}"),
                "locate path moved renamed folder missing localizar caminho movido renomeado pasta sumiu",
            ));
            v.push(item(
                EpicAction::Forget,
                "forget",
                Icon::DeleteOutline,
                tr!("Remove from installed games\u{2026}"),
                "missing deleted folder forget uninstall pasta apagada sumiu esquecer desinstalar",
            ));
            return v;
        }
        if !third_party {
            if update {
                v.push(item(EpicAction::Update, "update", Icon::Update, tr!("Update"), "upgrade patch atualizar atualizacao"));
            }
            if !playing {
                v.push(item(
                    EpicAction::Verify,
                    "verify",
                    Icon::Build,
                    tr!("Verify and repair"),
                    "check fix integrity verificar reparar integridade",
                ));
                v.push(item(
                    EpicAction::Move,
                    "move",
                    Icon::Folder,
                    tr!("Move install"),
                    "relocate folder disk mover instalacao pasta disco",
                ));
                v.push(item(
                    EpicAction::Relocate,
                    "relocate",
                    Icon::FolderOpen,
                    tr!("Change install location\u{2026}"),
                    "locate path moved renamed folder localizar caminho movido renomeado pasta",
                ));
            }
        }
        if has_dlcs && !third_party {
            v.push(item(EpicAction::Modify, "modify", Icon::Tune, tr!("Modify installation"), "dlc content modificar instalacao conteudo"));
        }
        // The overlay is enabled per Wine prefix on Linux only.
        if cfg!(target_os = "linux") && !third_party && game.runner == "legendary" {
            let app = game.app_name.clone();
            svc.update(cx, |s, cx| s.eos_load_game(&app, cx));
            let state = svc.read(cx).eos_game(&game.app_name);
            if let Some(state @ (EosGame::Enabled | EosGame::Disabled)) = state {
                let label = if state == EosGame::Enabled { tr!("Disable EOS Overlay") } else { tr!("Enable EOS Overlay") };
                v.push(item(EpicAction::Eos, "eos", Icon::Fullscreen, label, "epic online services overlay sobreposicao ativar desativar"));
            }
        }
        // The game settings' cloud saves tab has "Sync now"; the palette keeps it.
        if cloud {
            v.push(
                item(
                    EpicAction::Sync,
                    "sync",
                    Icon::CloudSync,
                    tr!("Cloud saves sync"),
                    "upload download saves sincronizar nuvem salvamentos",
                )
                .palette_only(),
            );
        }
        v.push(item(
            EpicAction::Uninstall,
            "uninstall",
            Icon::DeleteForever,
            tr!("Uninstall"),
            "remove delete desinstalar remover excluir",
        ));
        v
    }

    pub(in crate::collection) fn run_epic_action(&mut self, key: &str, action: EpicAction, cx: &mut Context<Self>) {
        let k = key.to_string();
        self.epic.pending = match action {
            EpicAction::ShowInDownloads => Some(Request::OpenDownloads),
            EpicAction::Import => Some(Request::Install { key: k, import: true }),
            EpicAction::Verify => Some(Request::Verify(k)),
            EpicAction::Move => Some(Request::Move(k)),
            EpicAction::Modify => Some(Request::Modify(k)),
            EpicAction::Eos => Some(Request::EosToggle(k)),
            EpicAction::Uninstall => Some(Request::Uninstall(k)),
            EpicAction::Relocate => Some(Request::Relocate(k)),
            EpicAction::Forget => Some(Request::Forget(k)),
            EpicAction::Update => {
                self.epic_update(key, cx);
                None
            }
            EpicAction::Sync => {
                let Some(game) = self.epic_game(key) else { return };
                self.epic_sync_saves(&game, cx);
                None
            }
        };
        cx.notify();
    }
}
