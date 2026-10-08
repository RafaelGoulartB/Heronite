//! UI side of the image pipeline: asks `heronite-images` for display-sized
//! rasters on worker threads, turns them into GPUI textures on the UI
//! thread, and keeps a bounded LRU so GPU memory tracks what is on screen.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use gpui_kit::{RenderImage, SharedString, Window};
use heronite_data::AppPaths;
use heronite_images::{Decoded, ImageError, Loader, LoaderConfig, Opts, Request, Variant};

pub type Loaded = (SharedString, Result<Arc<Decoded>, ImageError>);

/// Converts the data layer's chain (which ends with the bundled placeholder
/// marker) to the loader's.
pub fn to_chain(chain: &[heronite_data::ImageRef]) -> Vec<heronite_images::ImageRef> {
    chain
        .iter()
        .filter_map(|r| match r {
            heronite_data::ImageRef::File(p) => Some(heronite_images::ImageRef::File(p.clone())),
            heronite_data::ImageRef::Url(u) => Some(heronite_images::ImageRef::Url(u.clone())),
            heronite_data::ImageRef::Fallback => None,
        })
        .collect()
}

pub struct ImageStore {
    loader: Loader,
    tx: UnboundedSender<Loaded>,
    ready: HashMap<SharedString, Arc<RenderImage>>,
    order: VecDeque<SharedString>,
    pending: HashSet<SharedString>,
    failed: HashSet<SharedString>,
    evicted: Vec<Arc<RenderImage>>,
    capacity: usize,
}

impl ImageStore {
    pub fn new(images_cache: std::path::PathBuf, app: &AppPaths) -> (Self, UnboundedReceiver<Loaded>) {
        let opts = Opts::new(images_cache, Some(app.thumbs_dir()));
        let loader = Loader::new(LoaderConfig { threads: 3, opts });
        let (tx, rx) = unbounded();
        let store = Self {
            loader,
            tx,
            ready: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            evicted: Vec::new(),
            // ~3 screens of covers at 125px: a few MB of textures.
            capacity: 160,
        };
        (store, rx)
    }

    /// Returns the texture if ready (and marks it recently used), otherwise
    /// queues the load once.
    pub fn get_or_request(
        &mut self,
        id: &SharedString,
        chain: impl FnOnce() -> Vec<heronite_images::ImageRef>,
        width: u32,
        height: u32,
        variant: Variant,
        priority: i32,
    ) -> Option<Arc<RenderImage>> {
        if let Some(img) = self.ready.get(id) {
            if self.order.back() != Some(id) {
                if let Some(pos) = self.order.iter().position(|k| k == id) {
                    self.order.remove(pos);
                }
                self.order.push_back(id.clone());
            }
            return Some(img.clone());
        }
        if self.pending.contains(id) || self.failed.contains(id) {
            return None;
        }
        let chain = chain();
        if chain.is_empty() {
            self.failed.insert(id.clone());
            return None;
        }
        self.pending.insert(id.clone());
        log::debug!("image request {id}");
        let tx = self.tx.clone();
        let key = id.clone();
        self.loader.submit(Request { chain, width, height, variant, priority }, move |res| {
            let _ = tx.unbounded_send((key, res));
        });
        None
    }

    pub fn peek(&self, id: &SharedString) -> Option<Arc<RenderImage>> {
        self.ready.get(id).cloned()
    }

    pub fn on_loaded(&mut self, (id, res): Loaded) {
        log::debug!("image loaded {id}: {}", if res.is_ok() { "ok" } else { "err" });
        self.pending.remove(&id);
        match res {
            Ok(decoded) => {
                let decoded = Decoded::from_shared(decoded);
                let frame = image::Frame::new(decoded.into_image_buffer());
                let img = Arc::new(RenderImage::new(smallvec::smallvec![frame]));
                self.ready.insert(id.clone(), img);
                self.order.push_back(id);
                while self.order.len() > self.capacity {
                    if let Some(old) = self.order.pop_front() {
                        if let Some(img) = self.ready.remove(&old) {
                            self.evicted.push(img);
                        }
                    }
                }
            }
            Err(err) => {
                log::debug!("image {id}: {err}");
                self.failed.insert(id);
            }
        }
    }

    /// Frees GPU atlas space of evicted textures (needs the window).
    pub fn drop_evicted(&mut self, window: &mut Window) {
        for img in self.evicted.drain(..) {
            let _ = window.drop_image(img);
        }
    }

    /// Forget everything (e.g. after the library changed on disk).
    pub fn clear_failed(&mut self) {
        self.failed.clear();
    }
}
