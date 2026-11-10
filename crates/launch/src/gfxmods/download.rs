//! Downloads of mod files and lists: HTTPS only, into a temporary name so
//! a cut download never looks complete, and lists refreshed at most daily.

use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::net::{Cancel, Net};

/// How long a downloaded list stays fresh.
pub(crate) const TTL: Duration = Duration::from_secs(24 * 3600);

/// Whether `path` was written less than [`TTL`] ago.
pub(crate) fn fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < TTL)
}

pub(crate) fn https(url: &str) -> Result<(), String> {
    match url.starts_with("https://") {
        true => Ok(()),
        false => Err(format!("only HTTPS downloads are allowed: {url}")),
    }
}

/// Downloads `url` into a temporary name, then renames it to `dest`.
pub(crate) fn download(net: &dyn Net, url: &str, dest: &Path, cancel: Cancel) -> Result<(), String> {
    https(url)?;
    let tmp = dest.with_extension("part");
    let res = net.download(url, &tmp, cancel).and_then(|_| std::fs::rename(&tmp, dest).map_err(|e| e.to_string()));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_and_fresh_files() {
        assert!(https("https://x/y").is_ok());
        assert!(https("http://x/y").is_err());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("list.ini");
        assert!(!fresh(&file));
        std::fs::write(&file, b"").unwrap();
        assert!(fresh(&file));
    }
}
