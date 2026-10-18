//! What the panel reads off the UI thread when it opens: total play time,
//! the game's status, its recent captures and their previews, the audio
//! state and the controllers' batteries.

use std::path::PathBuf;

use gpui_kit::*;
use heronite_data::models::{CompletionStatus, LocalMetaFile};
use heronite_data::{json_store, status, DataPaths};
use heronite_media::{list_screenshots, GameQuery, Screenshot, ThumbnailCache};
use heronite_overlay::audio::{self, AudioState};
use heronite_overlay::controller::{self, Battery};
use heronite_overlay::input::Controller;

use super::OverlayPanel;
use crate::runtime::SessionInfo;

/// Captures listed (newest first).
const SHOTS: usize = 12;

#[derive(Default)]
pub struct Data {
    pub total_minutes: Option<u64>,
    pub statuses: Vec<CompletionStatus>,
    /// The game's status id (resolved: never empty once loaded).
    pub status_id: Option<String>,
    pub shots: Vec<Screenshot>,
    /// Preview file of each capture, once made (`None` while it is made).
    pub thumbs: std::collections::HashMap<PathBuf, Option<PathBuf>>,
    pub thumb_cache: Option<ThumbnailCache>,
    /// `None` until read; `Some(None)` = no audio control.
    pub audio: Option<Option<AudioState>>,
    pub controllers: Vec<(Controller, Option<Battery>)>,
}

struct Loaded {
    total_minutes: u64,
    statuses: Vec<CompletionStatus>,
    status_id: String,
    shots: Vec<Screenshot>,
    thumb_cache: ThumbnailCache,
}

fn load(paths: &DataPaths, app_name: &str, info: Option<&SessionInfo>) -> Loaded {
    let total_minutes = heronite_data::playtime::get_playtime_minutes(paths, app_name);
    let statuses = status::load_statuses(paths).unwrap_or_else(|_| status::default_statuses());
    let meta = json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap_or_default();
    let stored = meta.games.get(app_name).and_then(|m| m.completion_status_id.clone());
    let status_id = status::display_status(stored.as_deref(), &statuses)
        .map(|s| s.id.clone())
        .unwrap_or_else(|| status::resolve_status_id(stored.as_deref(), &statuses));
    let settings = heronite_data::settings::load(paths).ok();
    let (folder, limit) =
        settings.map(|s| (s.screenshots.folder.clone(), s.screenshots.cache_limit_mb.get())).unwrap_or_else(|| (String::new(), 50.));
    let shots = match info {
        Some(info) if !folder.is_empty() => {
            let query = GameQuery { title: info.title.clone(), steam_app_id: info.steam_app_id.clone(), aliases: info.aliases.clone() };
            let mut items = list_screenshots(std::path::Path::new(&folder), &query).items;
            items.reverse();
            items.truncate(SHOTS);
            items
        }
        _ => Vec::new(),
    };
    Loaded { total_minutes, statuses, status_id, shots, thumb_cache: ThumbnailCache::new(paths.screenshot_thumbnails_dir(), limit) }
}

impl OverlayPanel {
    /// Starts every read; each lands in `data` and redraws.
    pub(super) fn load_all(&mut self, paths: DataPaths, cx: &mut Context<Self>) {
        self.load_audio(cx);
        self.load_batteries(cx);
        // Play time, status and captures are the game's.
        let Some(game) = &self.game else { return };
        let app_name = game.game.app_name.clone();
        let info = game.info.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { load(&paths, &app_name, info.as_ref()) }).await;
            this.update(cx, |this, cx| {
                let d = &mut this.data;
                d.total_minutes = Some(loaded.total_minutes);
                d.statuses = loaded.statuses;
                d.status_id = Some(loaded.status_id);
                d.shots = loaded.shots;
                d.thumb_cache = Some(loaded.thumb_cache);
                this.load_thumbs(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads the audio state again (after a change, or when the panel opens).
    pub(super) fn load_audio(&mut self, cx: &mut Context<Self>) {
        let pid = self.game.as_ref().and_then(|g| g.game.pid);
        cx.spawn(async move |this, cx| {
            let state = cx.background_spawn(async move { audio::read(pid) }).await;
            this.update(cx, |this, cx| {
                this.data.audio = Some(state);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn load_batteries(&mut self, cx: &mut Context<Self>) {
        let list = self.controllers.clone();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_spawn(async move {
                    list.into_iter()
                        .map(|c| {
                            let b = controller::battery(&c.node);
                            (c, b)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.data.controllers = read;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Makes the previews of the listed captures, newest first, one at a time.
    fn load_thumbs(&mut self, cx: &mut Context<Self>) {
        let Some(cache) = self.data.thumb_cache.clone() else { return };
        let paths: Vec<PathBuf> = self.data.shots.iter().map(|s| s.path.clone()).filter(|p| !self.data.thumbs.contains_key(p)).collect();
        for p in &paths {
            self.data.thumbs.insert(p.clone(), None);
        }
        cx.spawn(async move |this, cx| {
            for source in paths {
                let (c, src) = (cache.clone(), source.clone());
                let thumb = cx.background_spawn(async move { c.thumbnail_path(&src).ok() }).await;
                let alive = this
                    .update(cx, |this, cx| {
                        this.data.thumbs.insert(source, thumb);
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

    /// The preview of a capture, once made.
    pub(super) fn thumb(&self, path: &std::path::Path) -> Option<PathBuf> {
        self.data.thumbs.get(path).cloned().flatten()
    }
}
