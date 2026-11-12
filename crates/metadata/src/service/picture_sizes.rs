//! The sizes of the pictures a dialog shows (the Metadata dialog's covers
//! and backgrounds), read from each picture's first bytes
//! ([`crate::picture_size`]) and remembered while the app runs: an address
//! keeps its picture, and a local copy is one file for good.

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::Mutex;

use super::MetadataService;
use crate::picture_size::{fetch_size, file_size};

/// Sizes read at once.
const AT_ONCE: usize = 6;
/// Sizes remembered at most; past it the memory starts over.
const CAP: usize = 5_000;

/// The sizes read, by address; `None`: not a picture whose size can be read.
#[derive(Default)]
pub(super) struct Sizes(Mutex<HashMap<String, Option<(u32, u32)>>>);

impl MetadataService {
    /// The size of each picture of `urls` (`http(s)`, `localart://`, a
    /// `file:` address or a path), read from its first bytes, a few at once,
    /// and remembered. `None` for one that cannot be read: not a picture, a
    /// missing file, or no answer (asked again next time). Blocking.
    pub fn picture_sizes(&self, urls: &[String]) -> Vec<(String, Option<(u32, u32)>)> {
        let mut out: Vec<(String, Option<(u32, u32)>)> = Vec::new();
        let mut unknown: Vec<&String> = Vec::new();
        {
            let known = self.sizes.0.lock();
            for url in urls {
                match known.get(url) {
                    Some(size) => out.push((url.clone(), *size)),
                    None if !unknown.contains(&url) => unknown.push(url),
                    None => {}
                }
            }
        }
        for batch in unknown.chunks(AT_ONCE) {
            let read: Vec<(String, Option<Option<(u32, u32)>>)> = std::thread::scope(|s| {
                let reads: Vec<_> = batch.iter().map(|url| (*url, s.spawn(move || self.read_size(url)))).collect();
                reads.into_iter().map(|(url, r)| (url.clone(), r.join().ok().flatten())).collect()
            });
            let mut known = self.sizes.0.lock();
            for (url, size) in read {
                if let Some(size) = size {
                    if known.len() >= CAP {
                        known.clear();
                    }
                    known.insert(url.clone(), size);
                }
                out.push((url, size.flatten()));
            }
        }
        out
    }

    /// One picture's size: `None` when it got no answer (not remembered).
    fn read_size(&self, url: &str) -> Option<Option<(u32, u32)>> {
        if url.starts_with("http://") || url.starts_with("https://") {
            return match fetch_size(&self.http, url) {
                Ok(size) => Some(size),
                Err(e) => {
                    log::info!("Picture size of {} not read: {e:#}", url.split(['?', '#']).next().unwrap_or(url));
                    None
                }
            };
        }
        let path = if url.starts_with("localart:") {
            self.paths.resolve_localart(url, None)
        } else {
            let rest = url.strip_prefix("file://").or_else(|| url.strip_prefix("file:")).unwrap_or(url);
            Some(PathBuf::from(heronite_data::paths::percent_decode(rest)))
        };
        Some(path.and_then(|p| file_size(&p)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::paths::DataPaths;

    #[test]
    fn local_pictures_and_unknown_ones() {
        let dir = tempfile::tempdir().unwrap();
        let svc = MetadataService::new(DataPaths::at(dir.path()));
        let art = svc.paths.metadata_art_dir().join("sideload_a");
        std::fs::create_dir_all(&art).unwrap();
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend(b"IHDR");
        png.extend(1400u32.to_be_bytes());
        png.extend(1240u32.to_be_bytes());
        std::fs::write(art.join("cover-square.png"), &png).unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"not a picture").unwrap();
        let urls = vec![
            "localart://metadata-art/sideload_a/cover-square.png".to_string(),
            dir.path().join("notes.txt").to_string_lossy().into_owned(),
            "localart://metadata-art/sideload_a/missing.png".to_string(),
        ];
        let sizes = svc.picture_sizes(&urls);
        assert_eq!(sizes, [(urls[0].clone(), Some((1400, 1240))), (urls[1].clone(), None), (urls[2].clone(), None)]);
        // Remembered: the file gone, the size stays.
        std::fs::remove_file(art.join("cover-square.png")).unwrap();
        assert_eq!(svc.picture_sizes(&urls[..1]), [(urls[0].clone(), Some((1400, 1240)))]);
    }
}
