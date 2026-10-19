//! Starting the thumbnail workers and the capture wiring, listing the
//! screenshots of the selected game and handling worker messages.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::ScreenshotsSettings;
use heronite_media::capture::CaptureEvent;
use heronite_media::{list_screenshots, GameQuery, ThumbnailCache};

use super::{texture, Full, MediaGlobal, Msg, Queue};
use crate::collection::CollectionView;

impl CollectionView {
    /// Called every render: starts workers, follows the selection and keeps
    /// the viewer's image loaded.
    pub(in crate::collection) fn sync_shots(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shots.drop_evicted(window);
        self.shots.prune_queue();
        let Some(lib) = self.lib.clone() else { return };
        let settings = lib.settings.screenshots.clone();
        self.shots_start(&settings, cx);
        let want = self.selected.clone();
        if self.shots.game != want || self.shots.folder != settings.folder {
            self.shots.reset();
            self.shots.game = want;
            self.shots.folder = settings.folder.clone();
            self.refresh_shots(cx);
        }
        self.shots_apply_capture_config(&settings, cx);
        if std::mem::take(&mut self.shots.debug_delete) {
            self.shots_delete(cx);
        }
        self.sync_viewer(window, cx);
    }

    fn shots_start(&mut self, settings: &ScreenshotsSettings, cx: &mut Context<Self>) {
        if let Some(cache) = &self.shots.cache {
            cache.set_limit_mb(settings.cache_limit_mb.get());
        }
        if self.shots.started {
            return;
        }
        self.shots.started = true;
        let cache = ThumbnailCache::new(self.data_paths.screenshot_thumbnails_dir(), settings.cache_limit_mb.get());
        let (tx, mut rx) = unbounded::<Msg>();
        let queue = Arc::new(Queue { items: Mutex::new(VecDeque::new()), cv: Condvar::new() });
        for n in 0..heronite_media::thumbnails::MAX_CONCURRENT_JOBS {
            let (queue, cache, tx) = (queue.clone(), cache.clone(), tx.clone());
            let _ = std::thread::Builder::new().name(format!("screenshot-thumbs-{n}")).spawn(move || loop {
                let path = {
                    let mut items = queue.items.lock().unwrap();
                    loop {
                        if let Some(p) = items.pop_front() {
                            break p;
                        }
                        items = queue.cv.wait(items).unwrap();
                    }
                };
                let res = cache.thumbnail(&path).map_err(|e| e.to_string());
                if tx.unbounded_send(Msg::Thumb(path, res)).is_err() {
                    break;
                }
            });
        }
        self.shots.cache = Some(cache.clone());
        self.shots.queue = Some(queue);
        self.shots.tx = Some(tx);
        cx.set_global(MediaGlobal { cache, capture: None });
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                if this
                    .update(cx, |this, cx| {
                        for m in batch {
                            this.shots_on_msg(m, cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(in crate::collection) fn shots_on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Thumb(path, res) => self.shots.on_thumb(path, res),
            Msg::Full { req, path, res } => {
                if req != self.shots.full_req.load(Ordering::SeqCst) {
                    return;
                }
                self.shots.full_pending = None;
                match res {
                    Ok((src, d)) => {
                        let size = (d.width, d.height);
                        let img = texture(d);
                        if let Some(old) = self.shots.full.replace(Full { path, src, size, img }) {
                            self.shots.evicted.push(old.img);
                        }
                    }
                    Err(err) => {
                        log::warn!("screenshot {}: {err}", path.display());
                        self.shots.full_failed = Some(path);
                    }
                }
            }
            Msg::Capture(CaptureEvent::Saved { app_name, runner, .. }) => {
                let current = self
                    .selected
                    .as_ref()
                    .and_then(|k| self.lib.as_ref()?.get(k))
                    .is_some_and(|g| g.app_name == app_name && g.runner == runner);
                if current {
                    self.refresh_shots(cx);
                }
            }
            Msg::Capture(_) => {}
        }
    }

    /// Re-lists the selected game's screenshots in the background.
    fn refresh_shots(&mut self, cx: &mut Context<Self>) {
        let (Some(lib), Some(key)) = (self.lib.clone(), self.shots.game.clone()) else { return };
        let Some(game) = lib.get(&key) else { return };
        if self.shots.folder.is_empty() {
            self.shots.set_items(Vec::new());
            return;
        }
        let query = GameQuery::from_game(game);
        let folder = PathBuf::from(&self.shots.folder);
        self.shots.epoch += 1;
        let epoch = self.shots.epoch;
        self.shots.listing = Some(cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { list_screenshots(&folder, &query) }).await;
            this.update(cx, |this, cx| {
                if this.shots.epoch == epoch {
                    this.shots.set_items(res.items);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The capture service itself lives in `services::session` (it must work
    /// with the window closed); the open Collection keeps it up to date with
    /// the settings.
    fn shots_apply_capture_config(&mut self, s: &ScreenshotsSettings, cx: &mut Context<Self>) {
        if let Some(session) = crate::services::session::get(cx) {
            session.update(cx, |session, _| session.apply_capture_config(s));
        }
    }
}
