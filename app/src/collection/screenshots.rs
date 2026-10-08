//! Screenshots of the focused game: state, background loading (listing,
//! thumbnails, full-resolution decode), the focus-panel strip, the gallery
//! modal and the capture-service wiring. The fullscreen viewer lives in
//! `screenshot_viewer.rs`. Port of `CollectionScreenshots.tsx`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::ScreenshotsSettings;
use heronite_images::Decoded;
use heronite_media::capture::{CaptureEvent, CaptureService};
use heronite_media::{list_screenshots, GameQuery, Screenshot, ThumbnailCache};

use super::{focus, CollectionView};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::icon;

pub const PREVIEW_SLOTS: usize = 4;
pub const GALLERY_PAGE: usize = 50;
const TEXTURE_CAP: usize = 160;
const ROW_H: f32 = 128.;
const GRID_GAP: f32 = 12.;

/// Shared with the settings dialog (cache info/clear, capture status).
#[derive(Clone)]
pub struct MediaGlobal {
    pub cache: ThumbnailCache,
    pub capture: Option<CaptureService>,
}
impl Global for MediaGlobal {}

pub enum Msg {
    Thumb(PathBuf, Result<Decoded, String>),
    Full { req: u64, path: PathBuf, res: Result<((u32, u32), Decoded), String> },
    Capture(CaptureEvent),
}

struct Queue {
    items: Mutex<VecDeque<PathBuf>>,
    cv: Condvar,
}

#[derive(Default)]
struct Textures {
    ready: HashMap<PathBuf, Arc<RenderImage>>,
    order: VecDeque<PathBuf>,
    pending: HashSet<PathBuf>,
    failed: HashSet<PathBuf>,
}

pub struct Full {
    pub path: PathBuf,
    /// Source size in pixels.
    pub src: (u32, u32),
    /// Decoded texture size in pixels.
    pub size: (u32, u32),
    pub img: Arc<RenderImage>,
}

#[derive(Clone, Copy)]
pub struct Drag {
    pub mouse: Point<Pixels>,
    pub pan: (f32, f32),
}

pub struct Shots {
    // what is listed
    pub game: Option<SharedString>,
    pub folder: String,
    pub items: Vec<Screenshot>,
    pub epoch: u64,
    listing: Option<Task<()>>,
    // workers
    started: bool,
    tx: Option<UnboundedSender<Msg>>,
    queue: Option<Arc<Queue>>,
    pub cache: Option<ThumbnailCache>,
    tex: Textures,
    evicted: Vec<Arc<RenderImage>>,
    // gallery
    pub gallery: bool,
    pub gallery_count: usize,
    pub gallery_scroll: ScrollHandle,
    // viewer
    pub view: Option<usize>,
    pub zoom: f32,
    pub pan: (f32, f32),
    pub fit: bool,
    pub controls_visible: bool,
    pub controls_task: Option<Task<()>>,
    pub drag: Option<Drag>,
    pub confirm: Option<usize>,
    pub deleting: bool,
    pub error: Option<SharedString>,
    pub full: Option<Full>,
    pub full_req: Arc<AtomicU64>,
    pub full_pending: Option<(PathBuf, (u32, u32))>,
    pub full_failed: Option<PathBuf>,
    pub boost: bool,
    /// `HERONITE_DEBUG_DIALOG=gallery|viewer|viewer-zoom|viewer-fit|confirm`.
    pub debug: Option<String>,
    pub debug_delete: bool,
}

impl Default for Shots {
    fn default() -> Self {
        Self {
            game: None,
            folder: String::new(),
            items: Vec::new(),
            epoch: 0,
            listing: None,
            started: false,
            tx: None,
            queue: None,
            cache: None,
            tex: Textures::default(),
            evicted: Vec::new(),
            gallery: false,
            gallery_count: GALLERY_PAGE,
            gallery_scroll: ScrollHandle::new(),
            view: None,
            zoom: 1.,
            pan: (0., 0.),
            fit: false,
            controls_visible: true,
            controls_task: None,
            drag: None,
            confirm: None,
            deleting: false,
            error: None,
            full: None,
            full_req: Arc::new(AtomicU64::new(0)),
            full_pending: None,
            full_failed: None,
            boost: false,
            debug: None,
            debug_delete: false,
        }
    }
}

impl Shots {
    pub fn tx_clone(&self) -> Option<UnboundedSender<Msg>> {
        self.tx.clone()
    }

    pub fn is_open(&self) -> bool {
        self.gallery || self.view.is_some()
    }

    /// Texture of a thumbnail, queueing its generation once.
    pub fn thumb(&mut self, path: &Path) -> Option<ImageSource> {
        if let Some(img) = self.tex.ready.get(path) {
            let img = img.clone();
            if self.tex.order.back().map(|p| p.as_path()) != Some(path) {
                if let Some(pos) = self.tex.order.iter().position(|p| p == path) {
                    if let Some(p) = self.tex.order.remove(pos) {
                        self.tex.order.push_back(p);
                    }
                }
            }
            return Some(ImageSource::Render(img));
        }
        if let Some(queue) = &self.queue {
            if !self.tex.pending.contains(path) && !self.tex.failed.contains(path) {
                self.tex.pending.insert(path.to_path_buf());
                queue.items.lock().unwrap().push_back(path.to_path_buf());
                queue.cv.notify_one();
            }
        }
        None
    }

    pub fn drop_evicted(&mut self, window: &mut Window) {
        for img in self.evicted.drain(..) {
            let _ = window.drop_image(img);
        }
    }

    pub fn discard_full(&mut self) {
        if let Some(f) = self.full.take() {
            self.evicted.push(f.img);
        }
        self.full_pending = None;
        self.full_failed = None;
        self.boost = false;
        // Invalidates any decode still running.
        self.full_req.fetch_add(1, Ordering::SeqCst);
    }

    pub fn reset_transform(&mut self) {
        self.zoom = 1.;
        self.pan = (0., 0.);
        self.drag = None;
    }

    /// Forget the listed game (selection or folder changed).
    fn reset(&mut self) {
        self.items.clear();
        self.gallery = false;
        self.gallery_count = GALLERY_PAGE;
        self.view = None;
        self.fit = false;
        self.confirm = None;
        self.error = None;
        self.controls_visible = true;
        self.controls_task = None;
        self.reset_transform();
        self.discard_full();
        if let Some(q) = &self.queue {
            let mut items = q.items.lock().unwrap();
            for p in items.drain(..) {
                self.tex.pending.remove(&p);
            }
        }
        self.listing = None;
        self.epoch += 1;
    }

    pub fn set_items(&mut self, items: Vec<Screenshot>) {
        self.items = items;
        if self.items.is_empty() {
            self.gallery = false;
            self.view = None;
        } else if let Some(v) = self.view {
            self.view = Some(v.min(self.items.len() - 1));
        }
        if let Some(d) = self.debug.take() {
            if !self.items.is_empty() {
                match d.as_str() {
                    "gallery" => self.gallery = true,
                    "viewer" => self.view = Some(0),
                    "viewer-zoom" => {
                        self.view = Some(0);
                        self.zoom = 2.5;
                        self.pan = (-120., 40.);
                        self.boost = true;
                    }
                    "viewer-fit" => {
                        self.view = Some(0);
                        self.fit = true;
                    }
                    "confirm" | "delete" => {
                        self.view = Some(0);
                        self.confirm = Some(0);
                        self.debug_delete = d == "delete";
                    }
                    _ => {}
                }
            }
        }
    }

    fn on_thumb(&mut self, path: PathBuf, res: Result<Decoded, String>) {
        self.tex.pending.remove(&path);
        match res {
            Ok(d) => {
                self.tex.ready.insert(path.clone(), texture(d));
                self.tex.order.push_back(path);
                while self.tex.order.len() > TEXTURE_CAP {
                    if let Some(old) = self.tex.order.pop_front() {
                        if let Some(img) = self.tex.ready.remove(&old) {
                            self.evicted.push(img);
                        }
                    }
                }
            }
            Err(err) => {
                log::debug!("screenshot thumbnail {}: {err}", path.display());
                self.tex.failed.insert(path);
            }
        }
    }
}

pub fn texture(d: Decoded) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(d.into_image_buffer())]))
}

/// Decodes the full image at (at most) `target` physical px, keeping aspect.
/// Returns the source size too.
pub fn load_full(path: &Path, bx: (u32, u32)) -> Result<((u32, u32), Decoded), String> {
    let (sw, sh) = image::image_dimensions(path).map_err(|e| e.to_string())?;
    let s = (bx.0 as f64 / sw as f64).min(bx.1 as f64 / sh as f64).min(1.);
    let (tw, th) = (((sw as f64 * s).round() as u32).max(1), ((sh as f64 * s).round() as u32).max(1));
    let mut opts = heronite_images::Opts::new(PathBuf::new(), None);
    opts.allow_network = false;
    let d = heronite_images::load_cover(&[heronite_images::ImageRef::File(path.to_path_buf())], tw, th, &opts)
        .map_err(|e| e.to_string())?;
    Ok(((sw, sh), d))
}

impl CollectionView {
    /// Called every render: starts workers, follows the selection and keeps
    /// the viewer's image loaded.
    pub(super) fn sync_shots(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shots.drop_evicted(window);
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
        let cache = ThumbnailCache::new(self.heroic.screenshot_thumbnails_dir(), settings.cache_limit_mb.get());
        let (tx, mut rx) = unbounded::<Msg>();
        let queue = Arc::new(Queue { items: Mutex::new(VecDeque::new()), cv: Condvar::new() });
        for n in 0..2 {
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
                if this.update(cx, |this, cx| {
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

    pub(super) fn shots_on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
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
    pub(super) fn refresh_shots(&mut self, cx: &mut Context<Self>) {
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

    // --- capture service ---------------------------------------------------

    /// The capture service itself lives in `services::session` (it must work
    /// with the window closed); the open Collection keeps it up to date with
    /// the settings.
    fn shots_apply_capture_config(&mut self, s: &ScreenshotsSettings, cx: &mut Context<Self>) {
        if let Some(session) = crate::services::session::get(cx) {
            session.update(cx, |session, _| session.apply_capture_config(s));
        }
    }

    // --- strip -------------------------------------------------------------

    /// "Screenshots" section of the focus panel (none without screenshots).
    pub(super) fn screenshots_section(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.shots.items.len();
        if n == 0 {
            return None;
        }
        let has_more = n > PREVIEW_SLOTS;
        let visible = if has_more { PREVIEW_SLOTS - 1 } else { n };
        let remaining = n - visible;
        let mut row = div().flex().justify_start().gap(px(8.)).h(px(89.6)).overflow_hidden();
        for i in 0..visible {
            let path = self.shots.items[i].path.clone();
            let src = self.shots.thumb(&path);
            row = row.child(
                thumb_tile(SharedString::from(format!("shot-strip-{i}")), src, true)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(i, window, cx))),
            );
        }
        if remaining > 0 {
            let path = self.shots.items[visible].path.clone();
            let src = self.shots.thumb(&path);
            row = row.child(
                thumb_tile("shot-strip-more".into(), src, true)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme::rgb_a(8, 10, 14, 0.62))
                            .text_color(white())
                            .font_weight(theme::W_BOLD)
                            .text_size(px(16.8))
                            .child(format!("+{remaining}")),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.shots.gallery = true;
                        this.shots.gallery_count = GALLERY_PAGE;
                        this.shots.view = None;
                        window.focus(&this.focus_handle, cx);
                        cx.notify();
                    })),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .child(focus::h3("Screenshots"))
                .child(focus::panel(true).child(row))
                .into_any_element(),
        )
    }

    pub(super) fn shots_open_view(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.shots.view = Some(i);
        self.shots.fit = false;
        self.shots.controls_visible = true;
        self.shots.error = None;
        self.shots.reset_transform();
        self.shots.discard_full();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    // --- gallery -----------------------------------------------------------

    pub(super) fn shots_close_gallery(&mut self) {
        self.shots.gallery = false;
        self.shots.gallery_count = GALLERY_PAGE;
        self.shots.view = None;
        self.shots.fit = false;
        self.shots.discard_full();
    }

    pub(super) fn render_gallery(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shots.gallery || self.shots.items.is_empty() {
            return None;
        }
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let modal_w = 1440f32.min(vw - 64.);
        let max_h = (vh * 0.9).min(1100.);
        let inner = modal_w - 2. - 30.4 - 2.4;
        let col_min = 224f32.min(inner);
        let cols = (((inner + GRID_GAP) / (col_min + GRID_GAP)).floor() as usize).max(1);
        let col_w = (inner - (cols - 1) as f32 * GRID_GAP) / cols as f32;
        let total = self.shots.items.len();
        let shown = self.shots.gallery_count.min(total);

        // Rows near the viewport get their thumbnails.
        let scroll = self.shots.gallery_scroll.clone();
        let view_h = f32::from(scroll.bounds().size.height).max(max_h - 80.);
        let off = -f32::from(scroll.offset().y);
        let step = ROW_H + GRID_GAP;
        let first_row = ((off / step).floor() as isize - 1).max(0) as usize;
        let last_row = ((off + view_h) / step).floor() as usize + 1;

        let mut grid = div().flex().flex_col().gap(px(GRID_GAP));
        for (r, chunk) in (0..shown).collect::<Vec<_>>().chunks(cols).enumerate() {
            let mut row = div().flex().gap(px(GRID_GAP)).h(px(ROW_H)).flex_none();
            for &i in chunk {
                let src = if (first_row..=last_row).contains(&r) {
                    let p = self.shots.items[i].path.clone();
                    self.shots.thumb(&p)
                } else {
                    None
                };
                    row = row.child(
                    thumb_tile(SharedString::from(format!("shot-grid-{i}")), src, false)
                        .w(px(col_w))
                        .h(px(ROW_H))
                        .flex_none()
                        .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(i, window, cx))),
                );
            }
            grid = grid.child(row);
        }
        if shown < total {
            grid = grid.child(
                div().flex().justify_center().my(px(5.6)).child(
                    button(
                        "shot-load-more",
                        ButtonKind::Glass,
                        format!("Load more ({} remaining)", total - shown),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shots.gallery_count = (this.shots.gallery_count + GALLERY_PAGE).min(this.shots.items.len());
                        cx.notify();
                    })),
                ),
            );
        }

        let close = icon_button("shot-gallery-close", Icon::Close, 36., 0.42, 0.72)
            .on_click(cx.listener(|this, _, _, cx| {
                this.shots_close_gallery();
                cx.notify();
            }));
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .mb(px(12.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.8))
                    .child(
                        div()
                            .text_size(px(16.8))
                            .font_weight(theme::W_BOLD)
                            .text_color(white())
                            .font_family(theme::FONT_BODY)
                            .child("Screenshots"),
                    )
                    .child(div().text_size(px(13.6)).text_color(theme::white_a(0.62)).child(total.to_string())),
            )
            .child(close);

        let modal = div()
            .id("shot-gallery-modal")
            .occlude()
            .flex()
            .flex_col()
            .w(px(modal_w))
            .max_h(px(max_h))
            .pt(px(13.6))
            .px(px(15.2))
            .pb(px(16.))
            .rounded(px(12.))
            .bg(theme::rgb_a(18, 22, 28, 0.96))
            .border_1()
            .border_color(theme::white_a(0.1))
            .shadow(vec![BoxShadow {
                color: theme::black_a(0.45),
                offset: point(px(0.), px(24.)),
                blur_radius: px(60.),
                spread_radius: px(0.),
                inset: false,
            }])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(
                div()
                    .id("shot-gallery-scroll")
                    .flex_1()
                    .min_h_0()
                    .pr(px(2.4))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .child(grid),
            );
        Some(
            deferred(
                div()
                    .id("shot-gallery")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(32.))
                    // Heronite blurs what is behind (16px); darker instead.
                    .bg(theme::rgb_a(4, 6, 8, 0.8))
                    .font_family(theme::FONT_BODY)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.shots_close_gallery();
                            cx.notify();
                        }),
                    )
                    .child(modal),
            )
            .with_priority(50)
            .into_any_element(),
        )
    }
}

/// `.collectionScreenshots__thumb`: rounded tile with cover image; the image
/// grows 4% on hover (no transform in GPUI, so the box is enlarged instead).
fn thumb_tile(id: SharedString, src: Option<ImageSource>, hover_zoom: bool) -> Stateful<Div> {
    let group: SharedString = format!("{id}-g").into();
    let mut tile = div()
        .id(id)
        .group(group.clone())
        .relative()
        .rounded(px(8.))
        .overflow_hidden()
        .bg(theme::black_a(0.35))
        .cursor_pointer();
    if let Some(src) = src {
        let mut holder = div().absolute().top_0().left_0().size_full();
        if hover_zoom {
            holder = holder.group_hover(group, |s| s.top(relative(-0.02)).left(relative(-0.02)).w(relative(1.04)).h(relative(1.04)));
        }
        tile = tile.child(holder.child(img(src).size_full().rounded(px(8.)).object_fit(ObjectFit::Cover)));
    }
    tile
}

/// Round translucent icon button (`collectionFocus__iconBtn`).
pub fn icon_button(id: impl Into<ElementId>, ic: Icon, d: f32, alpha: f32, hover_alpha: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(d))
        .rounded_full()
        .cursor_pointer()
        .bg(theme::rgb_a(10, 12, 14, alpha))
        .hover(move |s| s.bg(theme::rgb_a(10, 12, 14, hover_alpha)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon(ic, px(24.), rgb(0xf4f6f8)))
}

/// The shared cache / capture service, once the Collection started them.
pub fn screenshots_global(cx: &App) -> Option<MediaGlobal> {
    let mut media = cx.try_global::<MediaGlobal>().cloned()?;
    media.capture = crate::services::session::capture_of(cx);
    Some(media)
}
