//! State screenshots as raw image bytes, cached by path and mtime.

use std::sync::{Arc, Mutex, OnceLock};

use super::{EmulatorSaveState, Engine};
use crate::sysfiles::image_mime;

/// Raw screenshot of a state (PNG or JPEG bytes); decode it in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Two full listings (`MAX_STATES` each): a bigger cache holds screenshots
/// nobody asked for, a smaller one would reload every state on each listing.
const CACHE_LIMIT: usize = 48;

/// Least recently used first. Each entry is `(path, modified_at, thumbnail)`;
/// a missing screenshot is cached too, so it is not looked up again.
#[derive(Default)]
struct ThumbCache(Vec<(String, String, Option<Arc<Thumbnail>>)>);

impl ThumbCache {
    fn get(&mut self, path: &str, modified_at: &str) -> Option<Option<Arc<Thumbnail>>> {
        let at = self.0.iter().position(|(p, m, _)| p == path && m == modified_at)?;
        let entry = self.0.remove(at);
        let value = entry.2.clone();
        self.0.push(entry);
        Some(value)
    }

    fn insert(&mut self, path: &str, modified_at: &str, value: Option<Arc<Thumbnail>>) {
        // A rewritten state never matches its old mtime again.
        self.0.retain(|(p, _, _)| p != path);
        if self.0.len() >= CACHE_LIMIT {
            self.0.remove(0);
        }
        self.0.push((path.to_owned(), modified_at.to_owned(), value));
    }
}

fn thumb_cache() -> &'static Mutex<ThumbCache> {
    static CACHE: OnceLock<Mutex<ThumbCache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Loads (and caches by path + mtime) the state's screenshot as raw bytes.
pub fn thumbnail_for(engine: Engine, state: &EmulatorSaveState) -> Option<Arc<Thumbnail>> {
    if let Some(hit) = thumb_cache().lock().unwrap().get(&state.path, &state.modified_at) {
        return hit;
    }
    let value =
        engine.thumbnail(&state.path).filter(|b| !b.is_empty()).map(|bytes| Arc::new(Thumbnail { mime: image_mime(&bytes), bytes }));
    thumb_cache().lock().unwrap().insert(&state.path, &state.modified_at, value.clone());
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumb() -> Option<Arc<Thumbnail>> {
        Some(Arc::new(Thumbnail { mime: "image/png", bytes: vec![1] }))
    }

    #[test]
    fn rewritten_state_replaces_its_old_entry() {
        let mut cache = ThumbCache::default();
        cache.insert("a.state", "1", thumb());
        cache.insert("a.state", "2", thumb());
        assert!(cache.get("a.state", "1").is_none());
        assert!(cache.get("a.state", "2").is_some());
        assert_eq!(cache.0.len(), 1);
    }

    #[test]
    fn evicts_the_least_recently_used() {
        let mut cache = ThumbCache::default();
        for i in 0..CACHE_LIMIT {
            cache.insert(&format!("{i}.state"), "1", thumb());
        }
        assert!(cache.get("0.state", "1").is_some());
        cache.insert("new.state", "1", None);
        assert_eq!(cache.0.len(), CACHE_LIMIT);
        assert!(cache.get("1.state", "1").is_none());
        assert!(cache.get("0.state", "1").is_some());
        assert_eq!(cache.get("new.state", "1"), Some(None));
    }
}
