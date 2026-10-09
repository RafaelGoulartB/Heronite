//! UI side of the image pipeline: asks `heronite-images` for display-sized
//! rasters on worker threads, turns them into GPUI textures on the UI
//! thread, and keeps a bounded LRU so GPU memory tracks what is on screen.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use gpui_kit::{RenderImage, SharedString, Window};
use heronite_data::AppPaths;
use heronite_images::{Decoded, ImageError, Loader, LoaderConfig, Opts, Request, Variant};

/// A finished load: texture id, request generation, result.
pub type Loaded = (SharedString, u64, Result<Arc<Decoded>, ImageError>);

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

/// Pixel bytes the ready textures may hold. Each one also has a copy in the
/// GPU atlas, so the real cost is about twice this.
const BUDGET_BYTES: usize = 48 << 20;

/// Window-sized kinds (stage, glass backdrop) keep only the current texture
/// and the previous one (shown while the next loads), whatever the budget.
const KEEP_LATEST: [(&str, usize); 2] = [("s:", 2), ("g:", 2)];

pub struct ImageStore {
    loader: Loader,
    tx: UnboundedSender<Loaded>,
    ready: HashMap<SharedString, (Arc<RenderImage>, usize)>,
    /// Least recently used first.
    order: VecDeque<SharedString>,
    bytes: usize,
    /// Loads in flight and their generation: a result whose generation is no
    /// longer here was [`invalidate`](Self::invalidate)d and is dropped.
    pending: HashMap<SharedString, u64>,
    next_gen: u64,
    failed: HashSet<SharedString>,
    evicted: Vec<Arc<RenderImage>>,
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
            bytes: 0,
            pending: HashMap::new(),
            next_gen: 0,
            failed: HashSet::new(),
            evicted: Vec::new(),
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
        if let Some((img, _)) = self.ready.get(id) {
            if self.order.back() != Some(id) {
                if let Some(pos) = self.order.iter().position(|k| k == id) {
                    self.order.remove(pos);
                }
                self.order.push_back(id.clone());
            }
            return Some(img.clone());
        }
        if self.pending.contains_key(id) || self.failed.contains(id) {
            return None;
        }
        let chain = chain();
        if chain.is_empty() {
            self.failed.insert(id.clone());
            return None;
        }
        self.next_gen += 1;
        let generation = self.next_gen;
        self.pending.insert(id.clone(), generation);
        log::debug!("image request {id}");
        let tx = self.tx.clone();
        let key = id.clone();
        self.loader.submit(Request { chain, width, height, variant, priority }, move |res| {
            let _ = tx.unbounded_send((key, generation, res));
        });
        None
    }

    /// The texture if it is ready, without requesting it or marking it
    /// recently used (for things drawn off screen).
    pub fn peek(&self, id: &SharedString) -> Option<Arc<RenderImage>> {
        self.ready.get(id).map(|(img, _)| img.clone())
    }

    pub fn on_loaded(&mut self, (id, generation, res): Loaded) {
        log::debug!("image loaded {id}: {}", if res.is_ok() { "ok" } else { "err" });
        if self.pending.get(&id) != Some(&generation) {
            return;
        }
        self.pending.remove(&id);
        match res {
            Ok(decoded) => {
                let decoded = Decoded::from_shared(decoded);
                let size = decoded.byte_len();
                let frame = image::Frame::new(decoded.into_image_buffer());
                let img = Arc::new(RenderImage::new(smallvec::smallvec![frame]));
                if let Some((old, old_size)) = self.ready.insert(id.clone(), (img, size)) {
                    self.bytes -= old_size;
                    self.evicted.push(old);
                    self.order.retain(|k| k != &id);
                }
                self.bytes += size;
                self.order.push_back(id);
                self.evict();
            }
            Err(err) => {
                log::debug!("image {id}: {err}");
                self.failed.insert(id);
            }
        }
    }

    fn remove(&mut self, id: &SharedString) {
        if let Some((img, size)) = self.ready.remove(id) {
            self.bytes -= size;
            self.evicted.push(img);
        }
    }

    /// Drops the oldest textures over [`KEEP_LATEST`] and [`BUDGET_BYTES`]
    /// (the newest one always stays).
    fn evict(&mut self) {
        for (prefix, keep) in KEEP_LATEST {
            let of_kind: Vec<SharedString> = self.order.iter().filter(|k| k.starts_with(prefix)).cloned().collect();
            for id in of_kind.iter().take(of_kind.len().saturating_sub(keep)) {
                self.remove(id);
                self.order.retain(|k| k != id);
            }
        }
        while self.bytes > BUDGET_BYTES && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front() {
                self.remove(&old);
            }
        }
        log::debug!("image store: {} textures, {:.1} MB", self.order.len(), self.bytes as f64 / 1048576.);
    }

    /// Frees GPU atlas space of evicted textures (needs the window).
    pub fn drop_evicted(&mut self, window: &mut Window) {
        for img in self.evicted.drain(..) {
            let _ = window.drop_image(img);
        }
    }

    /// Drops the ready textures whose id starts with `prefix` (they are freed
    /// by the next [`drop_evicted`](Self::drop_evicted)): used when a whole
    /// kind of texture is switched off (stage art, glass backdrop).
    pub fn forget_prefix(&mut self, prefix: &str) {
        let ids: Vec<SharedString> = self.ready.keys().filter(|k| k.starts_with(prefix)).cloned().collect();
        for id in ids {
            self.remove(&id);
            self.order.retain(|k| k != &id);
        }
    }

    /// Forgets every texture whose id starts with one of `prefixes`: ready,
    /// failed and in flight (its result is dropped), so the next request loads
    /// the image again. Used when the image behind an id changed.
    pub fn invalidate(&mut self, prefixes: &[String]) {
        let hit = |k: &SharedString| prefixes.iter().any(|p| k.starts_with(p.as_str()));
        let ids: Vec<SharedString> = self.ready.keys().filter(|k| hit(k)).cloned().collect();
        for id in &ids {
            self.remove(id);
        }
        self.order.retain(|k| !hit(k));
        self.failed.retain(|k| !hit(k));
        self.pending.retain(|k, _| !hit(k));
    }

    /// Forgets the failed loads so they are tried again (e.g. after the library changed on disk).
    pub fn clear_failed(&mut self) {
        self.failed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use heronite_data::HeroicPaths;

    fn store(dir: &std::path::Path) -> (ImageStore, UnboundedReceiver<Loaded>) {
        ImageStore::new(dir.join("images-cache"), &AppPaths::for_heroic(&HeroicPaths::at(dir)))
    }

    /// A missing file: the load fails, which is enough to see where results go.
    fn request(s: &mut ImageStore, dir: &std::path::Path, id: &str) {
        let chain = vec![heronite_images::ImageRef::File(dir.join("missing.png"))];
        assert!(s.get_or_request(&id.into(), || chain, 10, 10, Variant::Cover, 1).is_none());
    }

    #[test]
    fn results_of_invalidated_requests_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, mut rx) = store(dir.path());
        request(&mut s, dir.path(), "c:a:10x10");
        let first = futures::executor::block_on(rx.next()).unwrap();

        // The art changed while the old image was loading: its result must not count.
        s.invalidate(&["c:a:".to_string()]);
        request(&mut s, dir.path(), "c:a:10x10");
        s.on_loaded(first);
        assert!(!s.failed.contains("c:a:10x10"));
        assert!(s.pending.contains_key("c:a:10x10"));

        let second = futures::executor::block_on(rx.next()).unwrap();
        s.on_loaded(second);
        assert!(s.failed.contains("c:a:10x10"));
        assert!(s.pending.is_empty());

        // Invalidating also forgets the failure, so the next request loads again.
        s.invalidate(&["c:a:".to_string()]);
        assert!(!s.failed.contains("c:a:10x10"));
        request(&mut s, dir.path(), "c:a:10x10");
        assert!(s.pending.contains_key("c:a:10x10"));
        // Other games are left alone.
        request(&mut s, dir.path(), "c:ab:10x10");
        s.invalidate(&["c:a:".to_string()]);
        assert!(s.pending.contains_key("c:ab:10x10"));
    }
}
