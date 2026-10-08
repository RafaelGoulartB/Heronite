//! Bulk metadata download: [`MetadataHub`] (owns the `MetadataService`, the
//! bulk job progress and the idle auto-fill; lives as long as the window) and
//! `CollectionMetadataWizard` (the "Download metadata" dialog).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::HeroicPaths;
use heronite_metadata::{BulkArgs, BulkItem, BulkProgress, BulkState, MetadataService};

use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};

pub enum HubEvent {
    /// The metadata job or idle fill changed data on disk: reload the library.
    DataChanged,
    /// The settings tab asks the Collection to open the wizard.
    OpenWizard,
}

pub struct MetadataHub {
    pub service: Arc<MetadataService>,
    pub progress: BulkProgress,
    /// The header chip is hidden once a finished job was dismissed.
    pub dismissed: bool,
    tx: Arc<Mutex<UnboundedSender<BulkProgress>>>,
}

impl EventEmitter<HubEvent> for MetadataHub {}

/// Lets any dialog reach the hub without threading it through `DialogCtx`.
pub struct HubGlobal(pub Entity<MetadataHub>);
impl Global for HubGlobal {}

pub fn hub(cx: &App) -> Entity<MetadataHub> {
    cx.global::<HubGlobal>().0.clone()
}

impl MetadataHub {
    pub fn new(heroic: HeroicPaths, cx: &mut Context<Self>) -> Self {
        let service = Arc::new(MetadataService::new(heroic));
        let (tx, mut rx) = unbounded::<BulkProgress>();
        cx.spawn(async move |this, cx| {
            while let Some(p) = rx.next().await {
                let alive = this
                    .update(cx, |this, cx| {
                        let was_done = this.progress.state == BulkState::Done;
                        let done = p.state == BulkState::Done;
                        this.progress = p;
                        this.dismissed = false;
                        if done && !was_done {
                            cx.emit(HubEvent::DataChanged);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        let progress = service.bulk_progress();
        Self { service, progress, dismissed: true, tx: Arc::new(Mutex::new(tx)) }
    }

    pub fn running(&self) -> bool {
        matches!(self.progress.state, BulkState::Running | BulkState::Cancelling)
    }

    /// Header chip visible (`running`, `cancelling` or `done` and not dismissed).
    pub fn chip_visible(&self) -> bool {
        self.running() || (self.progress.state == BulkState::Done && !self.dismissed)
    }

    pub fn start(&mut self, args: BulkArgs, cx: &mut Context<Self>) {
        let tx = self.tx.clone();
        self.progress = self.service.start_bulk_metadata(args, move |p| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.unbounded_send(p.clone());
            }
        });
        self.dismissed = false;
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        let tx = self.tx.clone();
        self.progress = self.service.cancel_bulk_metadata(&move |p| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.unbounded_send(p.clone());
            }
        });
        cx.notify();
    }

    pub fn request_wizard(&mut self, cx: &mut Context<Self>) {
        cx.emit(HubEvent::OpenWizard);
    }

    /// Idle maintenance like Heronite's `scheduleIdleMaintenance` (90 s after
    /// startup): fills missing Collection metadata when `autoFillMissing` is
    /// on, and local covers. Runs off the UI thread.
    pub fn schedule_idle(&mut self, delay: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(service) = this.update(cx, |this, _| this.service.clone()) else { return };
            let changed = cx
                .background_spawn(async move {
                    let filled = match service.refresh_missing_collection_metadata() {
                        Ok(n) => n,
                        Err(err) => {
                            log::warn!("idle metadata fill: {err:#}");
                            0
                        }
                    };
                    let covers = match service.refresh_missing_local_covers() {
                        Ok(n) => n,
                        Err(err) => {
                            log::warn!("idle cover fill: {err:#}");
                            0
                        }
                    };
                    filled + covers
                })
                .await;
            if changed > 0 {
                log::info!("idle fill updated {changed} item(s)");
                this.update(cx, |_, cx| cx.emit(HubEvent::DataChanged)).ok();
            }
        })
        .detach();
    }
}

pub struct MetadataWizard {
    hub: Entity<MetadataHub>,
    games: Vec<BulkItem>,
    skip_existing: bool,
    keep_manual: bool,
    download_images: bool,
    _sub: Subscription,
}

impl EventEmitter<DialogEvent> for MetadataWizard {}

impl MetadataWizard {
    pub fn new(_ctx: DialogCtx, games: Vec<BulkItem>, cx: &mut Context<Self>) -> Self {
        let hub = hub(cx);
        let sub = cx.observe(&hub, |_, _, cx| cx.notify());
        Self { hub, games, skip_existing: true, keep_manual: true, download_images: false, _sub: sub }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let args = BulkArgs {
            games: self.games.clone(),
            overwrite_existing: Some(!self.skip_existing),
            preserve_manual: Some(self.keep_manual),
            download_images: Some(self.download_images),
        };
        self.hub.update(cx, |h, cx| h.start(args, cx));
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        self.hub.update(cx, |h, cx| {
            if h.progress.state == BulkState::Done {
                h.dismissed = true;
            }
            cx.notify();
        });
        cx.emit(DialogEvent::Close);
    }
}

impl Render for MetadataWizard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.hub.read(cx).progress.clone();
        let running = matches!(p.state, BulkState::Running | BulkState::Cancelling);
        let done = p.state == BulkState::Done;

        let status = if running {
            format!("{}% \u{2014} {} of {}", p.percent, p.processed, p.total)
        } else if done {
            format!("Updated {}, skipped {}, failed {}.", p.updated, p.skipped, p.failed)
        } else {
            format!("{} games in Collection will be checked.", self.games.len())
        };

        let mut body = div().flex().flex_col().gap(px(12.)).child(
            div().text_size(px(16.)).text_color(rgb(0xcaf3fd)).child(
                "Fetches IGDB first, then Steam, store, Lutris, and Playnite data. You can hide this window; the download keeps going. Open it again from the icon in the Collection header.",
            ),
        );
        if !running && !done {
            body = body
                .child(checkbox("wiz-skip", self.skip_existing, "Skip games that already have metadata").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.skip_existing = !this.skip_existing;
                        cx.notify();
                    }),
                ))
                .child(checkbox("wiz-manual", self.keep_manual, "Keep fields edited by hand").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.keep_manual = !this.keep_manual;
                        cx.notify();
                    },
                )))
                .child(help_text(
                    "Name, notes, and any field you changed in Game settings stay as they are. Other empty or downloaded fields can still update from IGDB.",
                ))
                .child(checkbox("wiz-images", self.download_images, "Copy covers locally (uses more disk)").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.download_images = !this.download_images;
                        cx.notify();
                    }),
                ));
        }
        body = body
            .child(
                div().h(px(8.)).rounded_full().bg(theme::white_a(0.08)).overflow_hidden().child(
                    div().h_full().w(relative(p.percent.min(100) as f32 / 100.)).bg(theme::accent()),
                ),
            )
            .child(div().text_size(px(16.)).text_color(rgb(0xcaf3fd)).child(status));
        if running {
            if let Some(title) = p.current_title.clone() {
                body = body.child(help_text(if p.state == BulkState::Cancelling {
                    "Stopping after this game\u{2026}".to_string()
                } else {
                    title
                }));
            }
        }
        if !p.errors.is_empty() {
            body = body.child(
                div()
                    .id("wiz-errors")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .text_size(px(13.33))
                    .text_color(theme::danger())
                    .child(p.errors.join("\n")),
            );
        }

        let mut footer = div().flex().gap(px(16.));
        if !running {
            footer = footer.child(action_button(
                "wiz-start",
                ButtonKind::Primary,
                if done { "Run again" } else { "Start" },
                !self.games.is_empty(),
                cx.listener(|this, _, _, cx| this.start(cx)),
            ));
        } else {
            footer = footer.child(
                button(
                    "wiz-stop",
                    ButtonKind::Outline,
                    if p.state == BulkState::Cancelling { "Stopping after this game\u{2026}" } else { "Stop" },
                )
                .on_click(cx.listener(|this, _, _, cx| this.hub.update(cx, |h, cx| h.cancel(cx)))),
            );
        }
        footer = footer.child(
            button(
                "wiz-close",
                if running { ButtonKind::Primary } else { ButtonKind::Outline },
                if running { "Hide" } else { "Close" },
            )
            .on_click(cx.listener(|this, _, _, cx| this.hide(cx))),
        );

        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "metadata-wizard".into(), title: "Download metadata".into(), width: 584., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |this, cx| this.hide(cx)).ok();
            },
        )
    }
}
