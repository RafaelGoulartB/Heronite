//! State screenshots as raw image bytes, cached by path and mtime.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::{EmulatorSaveState, Engine};
use crate::sysfiles::image_mime;

/// Raw screenshot of a state (PNG or JPEG bytes); decode it in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

type ThumbCache = Mutex<HashMap<String, Option<Arc<Thumbnail>>>>;

fn thumb_cache() -> &'static ThumbCache {
    static CACHE: OnceLock<ThumbCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Loads (and caches by path + mtime) the state's screenshot as raw bytes.
pub fn thumbnail_for(engine: Engine, state: &EmulatorSaveState) -> Option<Arc<Thumbnail>> {
    let key = format!("{}:{}", state.path, state.modified_at);
    if let Some(hit) = thumb_cache().lock().unwrap().get(&key) {
        return hit.clone();
    }
    let value =
        engine.thumbnail(&state.path).filter(|b| !b.is_empty()).map(|bytes| Arc::new(Thumbnail { mime: image_mime(&bytes), bytes }));
    thumb_cache().lock().unwrap().insert(key, value.clone());
    value
}
