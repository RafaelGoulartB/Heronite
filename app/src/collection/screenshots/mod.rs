//! Screenshots of the focused game: shared state and the thumbnail texture
//! cache (here), background listing and workers (`background`), and the
//! focus-panel strip and gallery modal (`gallery`). The fullscreen viewer
//! lives in `screenshot_viewer.rs`. Port of `CollectionScreenshots.tsx`.

mod background;
mod gallery;

pub use gallery::{icon_button, thumb_tile};

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use futures::channel::mpsc::UnboundedSender;
use gpui_kit::*;
use heronite_images::Decoded;
use heronite_media::capture::{CaptureEvent, CaptureService};
use heronite_media::{Screenshot, ThumbnailCache};

pub(super) const PREVIEW_SLOTS: usize = 4;
pub(super) const GALLERY_PAGE: usize = 50;
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

pub(super) enum Msg {
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

pub(super) struct Full {
    pub path: PathBuf,
    /// Source size in pixels.
    pub src: (u32, u32),
    /// Decoded texture size in pixels.
    pub size: (u32, u32),
    pub img: Arc<RenderImage>,
}

#[derive(Clone, Copy)]
pub(super) struct Drag {
    pub mouse: Point<Pixels>,
    pub pan: (f32, f32),
}

pub(super) struct Shots {
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
    pub(super) fn tx_clone(&self) -> Option<UnboundedSender<Msg>> {
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

    pub(super) fn discard_full(&mut self) {
        if let Some(f) = self.full.take() {
            self.evicted.push(f.img);
        }
        self.full_pending = None;
        self.full_failed = None;
        self.boost = false;
        // Invalidates any decode still running.
        self.full_req.fetch_add(1, Ordering::SeqCst);
    }

    pub(super) fn reset_transform(&mut self) {
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

    pub(super) fn set_items(&mut self, items: Vec<Screenshot>) {
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

    /// Listed files were replaced by others with the same pixels (lossless
    /// compression): the items, textures and open image follow them, so
    /// nothing is decoded again. Returns whether a listed item changed.
    pub(super) fn follow_renames(&mut self, renamed: &[(PathBuf, PathBuf)]) -> bool {
        let mut changed = false;
        for (from, to) in renamed {
            let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(item) = self.items.iter_mut().find(|i| i.path == *from) {
                item.id = match item.id.rsplit_once('/') {
                    Some((dir, _)) => format!("{dir}/{name}"),
                    None => name.clone(),
                };
                item.name = name;
                item.path = to.clone();
                changed = true;
            }
            if let Some(img) = self.tex.ready.remove(from) {
                self.tex.ready.insert(to.clone(), img);
                if let Some(p) = self.tex.order.iter_mut().find(|p| *p == from) {
                    *p = to.clone();
                }
            }
            if let Some(full) = self.full.as_mut().filter(|f| f.path == *from) {
                full.path = to.clone();
            }
        }
        changed
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
pub(super) fn load_full(path: &Path, bx: (u32, u32)) -> Result<((u32, u32), Decoded), String> {
    let (sw, sh) = image::image_dimensions(path).map_err(|e| e.to_string())?;
    let s = (bx.0 as f64 / sw as f64).min(bx.1 as f64 / sh as f64).min(1.);
    let (tw, th) = (((sw as f64 * s).round() as u32).max(1), ((sh as f64 * s).round() as u32).max(1));
    let mut opts = heronite_images::Opts::new(PathBuf::new(), None);
    opts.allow_network = false;
    let d =
        heronite_images::load_cover(&[heronite_images::ImageRef::File(path.to_path_buf())], tw, th, &opts).map_err(|e| e.to_string())?;
    Ok(((sw, sh), d))
}

/// The shared cache / capture service, once the Collection started them.
pub fn screenshots_global(cx: &App) -> Option<MediaGlobal> {
    let mut media = cx.try_global::<MediaGlobal>().cloned()?;
    media.capture = crate::services::session::capture_of(cx);
    Some(media)
}
