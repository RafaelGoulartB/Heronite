//! Image resolution for the UI. Temporary: picks the first candidate that is
//! available locally (file, or Heronite's `images-cache/<sha256(url)>`) and
//! lets GPUI decode it. To be replaced by `heronite-images` (resized,
//! cached thumbnails, pre-blurred stage).

use std::path::{Path, PathBuf};

use heronite_data::ImageRef;
use sha2::{Digest, Sha256};

pub fn cache_path(images_cache: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    images_cache.join(hex)
}

pub fn first_local(chain: &[ImageRef], images_cache: &Path) -> Option<PathBuf> {
    chain.iter().find_map(|r| match r {
        ImageRef::File(p) => p.is_file().then(|| p.clone()),
        ImageRef::Url(u) => Some(cache_path(images_cache, u)).filter(|p| p.is_file()),
        ImageRef::Fallback => None,
    })
}
