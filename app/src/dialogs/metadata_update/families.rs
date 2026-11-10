//! A view that starts drawing a cover family (the desktop's Cover aspect or
//! the console theme changed) fetches that family's covers for the games
//! lacking them: one job, queued off the UI thread
//! (`MetadataService::cover_family_job`; nothing when the option
//! `fillCoverFamilies` is off or no source can give the family).
//!
//! The places that change a view's family call [`started_using`] from
//! anywhere (the console theme is changed without a window context); the
//! hub, which owns the job queue, picks the families up.

use std::sync::Mutex;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::console_theme::ConsoleTheme;
use heronite_data::cover_family::{console_families, CoverFamily};
use heronite_data::settings::CoverAspect;

use super::hub::MetadataHub;

static STARTED: Mutex<Option<UnboundedSender<CoverFamily>>> = Mutex::new(None);

/// Views started drawing `families`: the hub queues a download of each one
/// (the tall family needs none). Does nothing before the hub exists.
pub fn started_using(families: impl IntoIterator<Item = CoverFamily>) {
    let sender = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(tx) = sender.as_ref() else { return };
    for family in families.into_iter().filter(|f| *f != CoverFamily::Tall) {
        let _ = tx.unbounded_send(family);
    }
}

/// The desktop's Cover aspect went from `before` to `after`.
pub fn aspect_changed(before: CoverAspect, after: CoverAspect) {
    if before.family() != after.family() {
        started_using([after.family()]);
    }
}

/// The console theme in use went from `before` to `after`: the families its
/// tiles draw that the old one did not.
pub fn console_theme_changed(before: &ConsoleTheme, after: &ConsoleTheme) {
    let old = console_families(before);
    started_using(console_families(after).into_iter().filter(|f| !old.contains(f)));
}

impl MetadataHub {
    /// Listens to [`started_using`] for as long as the hub lives.
    pub(super) fn listen_families(cx: &mut Context<Self>) {
        let (tx, mut rx): (_, UnboundedReceiver<CoverFamily>) = unbounded();
        *STARTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        cx.spawn(async move |this, cx| {
            while let Some(family) = rx.next().await {
                if this.update(cx, |hub, cx| hub.cover_family_used(family, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// A view started drawing `family`: queues the download of its covers
    /// for every game of the Collection lacking them. Never blocks.
    pub fn cover_family_used(&mut self, family: CoverFamily, cx: &mut Context<Self>) {
        if self.faking() {
            return;
        }
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let job = cx.background_spawn(async move { service.cover_family_job(family, None) }).await;
            if let Some(args) = job {
                log::info!("{} covers: {} game(s) lack one", family.id(), args.games.len());
                this.update(cx, |hub, cx| hub.start(args, cx)).ok();
            }
        })
        .detach();
    }
}
