//! Long-lived tasks of the Collection view: decoded images, launch events and
//! the watch on the library files.

use std::time::Duration;

use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use gpui_kit::prelude::*;
use heronite_launch::LaunchEvent;

use super::CollectionView;
use crate::images::Loaded;

impl CollectionView {
    /// Applies decoded images as they arrive, one notify per burst.
    pub(super) fn spawn_image_pump(mut rx: UnboundedReceiver<Loaded>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let alive = this
                    .update(cx, |this, cx| {
                        for item in batch {
                            this.images.on_loaded(item);
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
    }

    pub(super) fn spawn_launch_events(mut events: UnboundedReceiver<LaunchEvent>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(ev) = events.next().await {
                if this.update(cx, |this, cx| this.on_launch_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Background jobs and external edits write the same files: reload when they change.
    pub(super) fn spawn_disk_watch(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let Ok(changed) = this.update(cx, |this, _| {
                !this.refreshing && this.lib.as_ref().is_some_and(|l| l.changed_on_disk())
            }) else {
                break;
            };
            if changed
                && this.update(cx, |this, cx| this.reload(cx)).is_err() {
                    break;
                }
        })
        .detach();
    }
}
