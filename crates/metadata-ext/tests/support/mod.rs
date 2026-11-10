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

/// The host [`pc_extras`] asks.
pub const PC_EXTRAS_HOST: &str = "api.pcextras.example.org";

/// An extras source of PC games written for the tests: series and features,
/// found by the game's Steam id when it has one (`matchedBy: steam`), else
/// by its name. Its service answers `GET /find?steam=…` or `?title=…` with
/// `[{page, year}]` and `GET /game?page=…` with `[{page, series, features}]`.
const PC_EXTRAS_JS: &str = r#"
async function get(ctx, path, query) {
  const r = await ctx.http.get("https://api.pcextras.example.org/" + path, { query });
  if (r.status !== 200) throw new Error("answered " + r.status);
  return r.json();
}
export async function find(game, ctx) {
  if (game.ids.steam) {
    const rows = await get(ctx, "find", { steam: game.ids.steam });
    if (rows.length) return rows.map((r) => ({ id: r.page, title: r.page, year: r.year, matchedBy: { id: "steam" } }));
  }
  const rows = await get(ctx, "find", { title: game.title });
  return rows.map((r) => ({ id: r.page, title: r.page, year: r.year }));
}
export async function fetch(id, ctx) {
  const row = (await get(ctx, "game", { page: id }))[0];
  return row ? { title: row.page, series: row.series, features: row.features } : null;
}
"#;

/// Writes the [`PC_EXTRAS_JS`] extension (`pcextras`, for PC games) in
/// `parent/pcextras`.
pub fn pc_extras(parent: &Path) -> PathBuf {
    let body = format!("[applies]\nto = \"pc\"\n\n[network]\nhosts = [\"{PC_EXTRAS_HOST}\"]\nrate = \"10/s\"");
    folder(parent, "pcextras", &manifest("pcextras", "1.0.0", "extras", &["series", "features"], &body), PC_EXTRAS_JS)
}

/// Endpoints of `server` whose origins `https://<host>` go to
/// `<server>/<host>` for each of `hosts`.
pub fn endpoints(server: &Server, hosts: &[&str]) -> Endpoints {
    let mut e = server.endpoints();
    e.origins = hosts.iter().map(|h| (format!("https://{h}"), format!("{}/{h}", server.base))).collect();
    e
}

/// The repository's `extensions/metadata/<name>` folder.
pub fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/metadata").join(name)
}
