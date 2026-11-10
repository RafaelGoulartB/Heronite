//! Helpers of the extension tests: extension folders written on the fly and
//! the fixture server of `heronite-metadata`'s tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[allow(unused_imports)]
pub use heronite_metadata::{Endpoints, Http};

#[path = "../../../metadata/tests/common/mod.rs"]
pub mod common;

#[allow(unused_imports)]
pub use common::{fixture, Req, Resp, Server};

/// A manifest of `id` with `body` after the required keys (sections).
pub fn manifest(id: &str, version: &str, kind: &str, provides: &[&str], body: &str) -> String {
    let fields: Vec<String> = provides.iter().map(|f| format!("\"{f}\"")).collect();
    format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = 1\nauthor = \"tests\"\nkind = \"{kind}\"\ndescription = \"A test source.\"\n\n[provides]\nfields = [{}]\n\n{body}\n",
        fields.join(", ")
    )
}

/// Writes an extension folder in `parent/<name>`.
pub fn folder(parent: &Path, name: &str, manifest: &str, script: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("source.toml"), manifest).unwrap();
    std::fs::write(dir.join("source.js"), script).unwrap();
    dir
}

/// Endpoints of `server` whose origins `https://<host>` go to
/// `<server>/<host>` for each of `hosts`.
pub fn endpoints(server: &Server, hosts: &[&str]) -> Endpoints {
    let mut e = server.endpoints();
    e.origins = hosts.iter().map(|h| (format!("https://{h}"), format!("{}/{h}", server.base))).collect();
    e
}

/// The repository's `extensions/<name>` folder.
pub fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions").join(name)
}
