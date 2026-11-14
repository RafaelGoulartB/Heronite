//! "Store media" in the focus panel and the grid's drawer: the focused
//! game's trailers and screenshots from its store page (Steam, for a game
//! with a Steam app id), in one stage with a strip of thumbnails under it.
//! Off until turned on in Settings → Appearance → Game sections.
//!
//! Nothing is kept but what the user allows (`heronite_metadata::store_media`):
//! the lists for a few days, and the first seconds of the trailers of the
//! installed games they visit, within the video cache's size. A trailer
//! streams when played, through ffmpeg (`heronite_media::stream`), at 480p,
//! with sound on Linux. It stops when its game is no longer focused, when
//! the section is not drawn (another page, the drawer closed, the section
//! turned off), and when a game starts. Turning the section off deletes
//! everything it kept (`clear_kept`).
//!
//! Debug: `HERONITE_DEBUG_MEDIA=play|paused|pick:<n>` (see `media_debug`).

mod view;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::appearance::Section;
use heronite_data::settings::StoreMediaKinds;
use heronite_i18n::tr;
use heronite_images::Decoded;
use heronite_media::stream::{Stream, StreamEvent, StreamSpec};
use heronite_metadata::store_media::{self, cache, StoreMedia as Media};
use heronite_metadata::{Endpoints, Http, Lookup};
use smallvec::smallvec;

use super::CollectionView;

/// Frames of a playing trailer, and their size (480p, the stream's).
const FPS: u32 = 30;
const FRAME_W: u32 = 854;
const FRAME_H: u32 = 480;
/// A trailer stops when the window has not drawn for this long (minimized,
/// on another workspace): its frames would only pile up unseen.
const UNSEEN: Duration = Duration::from_millis(1500);
/// Browsing the list with the keys asks the store only for the game the
/// selection stays on.
const LIST_DELAY: Duration = Duration::from_millis(250);
/// A trailer's start is kept only for a game looked at this long.
const KEEP_AFTER: Duration = Duration::from_secs(3);

/// The focused game's list.
#[derive(Clone, Default)]
enum List {
    #[default]
    None,
    Loading,
    Ready(Arc<Media>),
    /// The store has none, or could not be reached.
    Missing,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PlayState {
    /// Getting the playlists, or waiting for the first frame.
    Loading,
    Playing,
    Paused,
    Ended,
    Failed(SharedString),
}

/// The trailer on the stage.
struct Player {
    /// Index into the list's videos.
    video: usize,
    /// Its streams, `None` while paused or before it starts.
    stream: Option<Stream>,
    state: PlayState,
    frame: Option<Arc<RenderImage>>,
    /// The newest frame the stream decoded, taken when the window draws (a
    /// frame never drawn is replaced, never queued).
    slot: Arc<Mutex<Option<Decoded>>>,
    /// Seconds into the trailer where the stream started.
    start: f64,
    /// Frames the stream decoded since then (its clock).
    count: Arc<AtomicU64>,
    /// Length in seconds; 0 until the playlist is read.
    seconds: f64,
    playable: Option<cache::Playable>,
    /// Played with sound.
    sound: bool,
    /// Tells the stream's events apart from an earlier stream's.
    generation: u64,
}

impl Player {
    fn position(&self) -> f64 {
        let at = self.start + self.count.load(Ordering::Relaxed) as f64 / FPS as f64;
        if self.seconds > 0. {
            at.min(self.seconds)
        } else {
            at
        }
    }
}

#[derive(Default)]
pub(super) struct StoreMediaUi {
    /// The game whose media are shown, and the Steam app id they come from.
    game: Option<SharedString>,
    app_id: Option<String>,
    list: List,
    /// The item on the stage: the videos first, then the screenshots.
    pick: usize,
    strip: ScrollHandle,
    player: Option<Player>,
    /// The list was read with autoplay on: the first trailer starts once
    /// the section is drawn.
    autoplay: bool,
    /// The section was drawn since the last frame (else a trailer stops).
    drawn: bool,
    /// Textures of frames shown before, dropped on the next frame.
    old_frames: Vec<Arc<RenderImage>>,
    generation: u64,
    /// Trailers whose start was kept (or asked) this session.
    kept: HashSet<String>,
    /// Kept files of a section turned off were looked for.
    swept: bool,
    /// What the list on screen keeps: videos, screenshots or both.
    kinds: StoreMediaKinds,
    /// When the window last drew.
    last_draw: Option<Instant>,
    /// ffmpeg's sound output was looked for off the UI thread.
    warmed: bool,
    /// Games running at the last frame.
    games_running: usize,
    /// Where the seek bar is, for its clicks.
    bar: Rc<std::cell::Cell<Bounds<Pixels>>>,
}

impl StoreMediaUi {
    fn reset(&mut self) {
        self.stop();
        self.game = None;
        self.app_id = None;
        self.list = List::None;
        self.pick = 0;
        self.strip = ScrollHandle::new();
    }

    fn stop(&mut self) {
        if let Some(p) = self.player.take() {
            self.old_frames.extend(p.frame);
        }
    }
}

/// `<cache>/store-media`.
fn root(app: &heronite_data::AppPaths) -> PathBuf {
    app.store_media_dir()
}

/// Deletes everything the section kept: the lists, the trailers' starts and
/// the copies of its pictures in the shared image cache. Blocking.
pub(crate) fn clear_kept(app: &heronite_data::AppPaths, images_cache: &Path) {
    let dir = root(app);
    if !dir.exists() {
        return;
    }
    for url in store_media::kept_pictures(&dir) {
        let _ = std::fs::remove_file(heronite_images::cache_path(images_cache, &url));
    }
    if let Err(e) = store_media::clear(&dir) {
        log::warn!("could not delete {}: {e}", dir.display());
    }
}

/// Trims the kept trailers to `limit_mb` (all of them for 0). Blocking.
pub(crate) fn trim_kept(app: &heronite_data::AppPaths, limit_mb: u32) {
    cache::trim(&root(app), u64::from(limit_mb) << 20, None);
}

fn shown() -> bool {
    crate::theme::appearance().sections.shown(Section::Media)
}

impl CollectionView {
    /// Every frame, before the layout: follows the focused game, stops a
    /// trailer the panel no longer draws, drops the frames shown before.
    pub(super) fn sync_store_media(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.media.last_draw = Some(Instant::now());
        // The newest frame becomes the one drawn; the one before goes from the GPU now.
        if let Some(p) = self.media.player.as_mut() {
            let newest = p.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(f) = newest {
                let image = Arc::new(RenderImage::new(smallvec![image::Frame::new(f.into_image_buffer())]));
                self.media.old_frames.extend(p.frame.replace(image));
                if p.state == PlayState::Loading {
                    p.state = PlayState::Playing;
                }
            }
        }
        for old in self.media.old_frames.drain(..) {
            let _ = window.drop_image(old);
        }
        if !self.media.swept {
            self.media.swept = true;
            if !shown() {
                let (app, images) = (self.app_paths.clone(), self.data_paths.images_cache_dir());
                cx.background_spawn(async move { clear_kept(&app, &images) }).detach();
            }
        }
        let drawn = std::mem::take(&mut self.media.drawn);
        // A game that starts stops the trailer (one running already does not).
        let games = self.playing.len();
        let game_started = games > std::mem::replace(&mut self.media.games_running, games);
        if self.media.player.as_ref().is_some_and(|p| p.stream.is_some()) && (!drawn || game_started) {
            self.media_pause(cx);
        }
        if drawn && std::mem::take(&mut self.media.autoplay) && self.media.pick == 0 && self.media.player.is_none() {
            self.media_play(0, 0., false, cx);
        }
        if !shown() || crate::console::active() {
            if self.media.game.is_some() {
                self.media.reset();
            }
            return;
        }
        let kinds = self.lib.as_ref().map(|l| l.settings.store_media_kinds).unwrap_or_default();
        if self.media.game != self.selected || self.media.kinds != kinds {
            self.media.reset();
            self.media.game = self.selected.clone();
            self.media.kinds = kinds;
            self.load_store_media(cx);
        }
    }

    /// The Steam app id of the focused game (any of its copies).
    fn media_app_id(&self, key: &str) -> Option<String> {
        self.with_copies(key)
            .iter()
            .find_map(|g| g.steam_app_id.clone().or_else(|| g.metadata.as_ref().and_then(|m| m.steam_app_id.clone())))
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    }

    fn load_store_media(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.media.game.clone() else { return };
        let Some(id) = self.media_app_id(&key) else {
            self.media.list = List::Missing;
            return;
        };
        let installed = self.with_copies(&key).iter().any(|g| g.installed);
        self.media.app_id = Some(id.clone());
        self.media.list = List::Loading;
        let dir = root(&self.app_paths);
        if !std::mem::replace(&mut self.media.warmed, true) {
            cx.background_spawn(async {
                heronite_media::posters::available();
                heronite_media::stream::sound_supported();
            })
            .detach();
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LIST_DELAY).await;
            if this.read_with(cx, |this, _| this.media.game.as_ref() != Some(&key)).unwrap_or(true) {
                return;
            }
            let found = cx.background_spawn(async move { store_media::steam(&Http::new(Endpoints::default()), &dir, &id) }).await;
            this.update(cx, |this, cx| {
                if this.media.game.as_ref() != Some(&key) {
                    return;
                }
                let kinds = this.media.kinds;
                this.media.list = match found {
                    Lookup::Found(mut m) => {
                        if !kinds.videos() {
                            m.videos.clear();
                        }
                        if !kinds.screenshots() {
                            m.screenshots.clear();
                        }
                        if m.is_empty() {
                            List::Missing
                        } else {
                            List::Ready(Arc::new(m))
                        }
                    }
                    _ => List::Missing,
                };
                if installed {
                    this.keep_first_trailer(cx);
                }
                let autoplay = this.lib.as_ref().is_some_and(|l| l.settings.video_autoplay);
                this.media.autoplay = autoplay && matches!(&this.media.list, List::Ready(m) if !m.videos.is_empty());
                this.media_debug(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keeps the start of the focused game's first trailer, so it plays at
    /// once next time (an installed game looked at for [`KEEP_AFTER`], within
    /// the cache's size), on a thread at idle priority.
    fn keep_first_trailer(&mut self, cx: &mut Context<Self>) {
        let List::Ready(m) = &self.media.list else { return };
        let Some(video) = m.videos.first().cloned() else { return };
        let limit = self.lib.as_ref().map_or(0, |l| l.settings.video_cache_limit_mb);
        if limit == 0 || !self.media.kept.insert(video.id.clone()) {
            return;
        }
        let dir = root(&self.app_paths);
        let key = self.media.game.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(KEEP_AFTER).await;
            let still = this.read_with(cx, |this, _| this.media.game == key && shown()).unwrap_or(false);
            if !still {
                this.update(cx, |this, _| this.media.kept.remove(&video.id)).ok();
                return;
            }
            // A thread of its own: idle priority stays with it.
            let spawned = std::thread::Builder::new().name("trailer-keep".into()).spawn(move || {
                heronite_data::proc::idle_priority();
                if let Err(e) = cache::keep(&Http::new(Endpoints::default()), &dir, &video, u64::from(limit) << 20) {
                    log::info!("could not keep the start of {}: {e:#}", video.name);
                }
            });
            if let Err(e) = spawned {
                log::warn!("trailer-keep thread: {e}");
            }
        })
        .detach();
    }

    /// `HERONITE_DEBUG_MEDIA=play|paused|pick:<n>`: plays the first trailer
    /// (without sound; `paused` pauses it after 5 s) or puts item `n` on the
    /// stage, once the list is read.
    fn media_debug(&mut self, cx: &mut Context<Self>) {
        let Some(spec) = crate::env::value("HERONITE_DEBUG_MEDIA") else { return };
        if spec == "play" || spec == "paused" {
            self.media_play(0, 0., false, cx);
        }
        if spec == "paused" {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
                this.update(cx, |this, cx| this.media_pause(cx)).ok();
            })
            .detach();
        } else if let Some(n) = spec.strip_prefix("pick:").and_then(|n| n.parse().ok()) {
            self.media_pick(n, cx);
        }
    }

    /// Puts item `i` (videos, then screenshots) on the stage.
    pub(super) fn media_pick(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.media.pick != i {
            self.media.stop();
            self.media.pick = i;
            self.media.strip.scroll_to_item(i);
        }
        cx.notify();
    }

    /// The trailer on the stage: plays it from where it stopped (from the
    /// start once it ended, always without sound), or pauses it.
    pub(super) fn media_toggle_play(&mut self, cx: &mut Context<Self>) {
        let video = self.media.pick;
        match &self.media.player {
            Some(p) if p.video == video && matches!(p.state, PlayState::Playing | PlayState::Loading) => self.media_pause(cx),
            Some(p) if p.video == video && p.state == PlayState::Paused => {
                let (at, sound) = (p.position(), p.sound);
                self.media_play(video, at, sound, cx);
            }
            _ => self.media_play(video, 0., false, cx),
        }
    }

    pub(super) fn media_pause(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.media.player.as_mut() {
            if p.stream.take().is_some() {
                p.start = p.position();
                p.count = Arc::default();
                p.state = PlayState::Paused;
            }
        }
        cx.notify();
    }

    /// Plays at `fraction` (0..=1) of the trailer on the stage.
    pub(super) fn media_seek(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let Some(p) = self.media.player.as_ref() else { return };
        if p.seconds <= 0. {
            return;
        }
        let at = (p.seconds * fraction.clamp(0., 1.) as f64).min((p.seconds - 1.).max(0.));
        let (video, sound) = (p.video, p.sound);
        self.media_play(video, at, sound, cx);
    }

    /// Sound on or off for the trailer on the stage (it starts again where
    /// it is; a paused one keeps it for when it plays).
    pub(super) fn media_toggle_sound(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.media.player.as_mut() else { return };
        p.sound = !p.sound;
        if p.stream.is_some() {
            let (video, at, sound) = (p.video, p.position(), p.sound);
            self.media_play(video, at, sound, cx);
        }
        cx.notify();
    }

    /// Starts the trailer `video` at `at` seconds, with `sound` or muted:
    /// its playlists first (the kept start read from this PC), then the
    /// stream. A trailer started from the beginning is always muted.
    fn media_play(&mut self, video: usize, at: f64, sound: bool, cx: &mut Context<Self>) {
        let List::Ready(m) = &self.media.list else { return };
        let Some(v) = m.videos.get(video).cloned() else { return };
        let app_id = self.media.app_id.clone();
        self.media.generation += 1;
        let generation = self.media.generation;
        let sound = sound && heronite_media::stream::sound_known() == Some(true);
        let (frame, playable, seconds) = match self.media.player.take() {
            Some(p) if p.video == video => (p.frame, p.playable, p.seconds),
            Some(p) => {
                self.media.old_frames.extend(p.frame);
                (None, None, 0.)
            }
            None => (None, None, 0.),
        };
        self.media.player = Some(Player {
            video,
            stream: None,
            state: PlayState::Loading,
            frame,
            slot: Arc::default(),
            start: at,
            count: Arc::default(),
            seconds,
            playable: playable.clone(),
            sound,
            generation,
        });
        cx.notify();
        if !heronite_media::posters::available() {
            if let Some(p) = self.media.player.as_mut() {
                p.state = PlayState::Failed(tr!("Trailers play with ffmpeg, which is not installed.").into());
            }
            return;
        }
        let dir = root(&self.app_paths);
        cx.spawn(async move |this, cx| {
            let ready = match playable {
                Some(p) => Ok(p),
                None => cx.background_spawn(async move { cache::prepare(&Http::new(Endpoints::default()), &dir, &v) }).await,
            };
            this.update(cx, |this, cx| {
                let Some(p) = this.media.player.as_mut().filter(|p| p.generation == generation) else { return };
                match ready {
                    Ok(playable) => {
                        p.seconds = playable.seconds;
                        p.playable = Some(playable.clone());
                        this.media_stream(playable, at, sound, generation, cx);
                    }
                    Err(e) => {
                        log::warn!("trailer: {e:#}");
                        p.state = PlayState::Failed(tr!("The trailer could not be loaded.").into());
                        // Its address may have changed: the next visit asks the store again.
                        if let Some(id) = app_id {
                            let dir = root(&this.app_paths);
                            cx.background_spawn(async move { store_media::forget(&dir, &id) }).detach();
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs ffmpeg on `playable`. Its frames wait in the player's slot for
    /// the next draw; each one only asks for a draw.
    fn media_stream(&mut self, playable: cache::Playable, at: f64, sound: bool, generation: u64, cx: &mut Context<Self>) {
        let Some(p) = self.media.player.as_mut() else { return };
        let (slot, count) = (p.slot.clone(), p.count.clone());
        let (tx, mut rx) = unbounded::<Option<StreamEvent>>();
        let spec = StreamSpec { video: playable.video, audio: playable.audio, start: at, width: FRAME_W, height: FRAME_H, fps: FPS, sound };
        p.stream = Some(Stream::start(spec, move |ev| match ev {
            StreamEvent::Frame(f) => {
                let n = count.fetch_add(1, Ordering::Relaxed) + 1;
                let was_empty = slot.lock().unwrap_or_else(|e| e.into_inner()).replace(f).is_none();
                // Also once a second while nothing draws, so an unseen trailer stops.
                if was_empty || n % u64::from(FPS) == 0 {
                    let _ = tx.unbounded_send(None);
                }
            }
            end => {
                let _ = tx.unbounded_send(Some(end));
            }
        }));
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                let alive = this
                    .update(cx, |this, cx| {
                        let unseen = this.media.last_draw.is_none_or(|t| t.elapsed() > UNSEEN);
                        let Some(p) = this.media.player.as_mut().filter(|p| p.generation == generation) else { return false };
                        match ev {
                            Some(StreamEvent::Ended) => {
                                p.stream = None;
                                p.state = PlayState::Ended;
                            }
                            Some(StreamEvent::Failed(why)) => {
                                log::warn!("trailer: {why}");
                                p.stream = None;
                                p.state = PlayState::Failed(tr!("The trailer could not be played.").into());
                            }
                            _ if unseen => {
                                this.media_pause(cx);
                                return false;
                            }
                            _ => {}
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }
}
