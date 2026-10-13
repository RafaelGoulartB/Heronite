//! Download size of a game (`nile install --info --json <id>`).

use anyhow::{bail, Result};
use heronite_data::DataPaths;
use heronite_epic::cache_store::CacheStore;
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::cmd::Nile;

/// Cached for 6 hours, like the other stores' install info.
const LIFESPAN_MINUTES: u64 = 60 * 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallInfo {
    /// Bytes to download for a fresh install.
    pub download_size: u64,
    /// Bytes on disk once installed: nile stores the files as they are
    /// downloaded, so it equals `download_size`.
    pub disk_size: u64,
}

impl InstallInfo {
    fn new(download_size: u64) -> Self {
        Self { download_size, disk_size: download_size }
    }
}

fn store(paths: &DataPaths) -> CacheStore<'_> {
    CacheStore::new(paths, "nile_install_info", Some(LIFESPAN_MINUTES))
}

/// The cached info or `nile install --info`.
pub fn install_info(paths: &DataPaths, nile: &Nile, app_name: &str) -> Result<InstallInfo> {
    let cache = store(paths);
    if let Some(size) = cache.get(app_name).as_ref().and_then(download_size) {
        log::debug!("Using cached install info for {app_name}");
        return Ok(InstallInfo::new(size));
    }
    let out = nile.run(&["install", "--info", "--json", app_name])?;
    let Some(size) = parse_size(&out.stdout) else {
        let why = out.error_lines();
        if why.is_empty() {
            bail!(tr!("Failed to get install info for {app_name}", app_name));
        }
        bail!(tr!("Failed to get install info for {app_name}: {error}", app_name, error = why));
    };
    cache.set(app_name, json!({ "download_size": size }))?;
    Ok(InstallInfo::new(size))
}

fn download_size(doc: &Value) -> Option<u64> {
    doc.get("download_size").and_then(Value::as_u64)
}

/// `{"download_size": <bytes>}` on one line of stdout.
fn parse_size(stdout: &str) -> Option<u64> {
    stdout.lines().map(str::trim).filter(|l| l.starts_with('{')).find_map(|l| download_size(&serde_json::from_str::<Value>(l).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::Env;

    #[test]
    fn size_is_read_from_the_json_line() {
        assert_eq!(parse_size("{\"download_size\": 1234}\n"), Some(1234));
        assert_eq!(parse_size("noise\n{\"download_size\": 7}"), Some(7));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("{\"other\": 1}"), None);
    }

    #[test]
    fn info_is_fetched_once_and_cached() {
        let env = Env::new();
        let nile = env.nile("echo 'INFO [CLI]:\t Found: Game' >&2; echo '{\"download_size\": 5000000}'");
        let info = install_info(&env.paths, &nile, "amzn1.adg.product.AAA").unwrap();
        assert_eq!(info, InstallInfo { download_size: 5_000_000, disk_size: 5_000_000 });
        let again = install_info(&env.paths, &nile, "amzn1.adg.product.AAA").unwrap();
        assert_eq!(again.download_size, 5_000_000);
        assert_eq!(env.calls(), ["install --info --json amzn1.adg.product.AAA"]);
    }

    #[test]
    fn failure_carries_nile_errors() {
        let env = Env::new();
        let nile = env.nile("echo 'ERROR [CLI]:\t Couldn'\"'\"'t find what you are looking for' >&2");
        let err = install_info(&env.paths, &nile, "x").unwrap_err().to_string();
        assert!(err.contains("find what you are looking for"), "{err}");
        let nile = env.nile("true");
        assert!(install_info(&env.paths, &nile, "x").unwrap_err().to_string().contains("Failed to get install info for x"));
    }
}
