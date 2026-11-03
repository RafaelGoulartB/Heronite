//! Builds of the first content system, which older games still have: no
//! compression and no chunks, each file is a byte range of the product's
//! `main.bin`.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::api::{self, Client};
use crate::files::{self, Checksum, FileEntry, Items, Root, Source};
use crate::language::{self, Language};
use crate::v2::{Dlc, Sizes};

/// One depot of the build.
#[derive(Debug, Clone, PartialEq)]
pub struct Depot {
    /// v1 language names (`English`, `Neutral` for every language).
    pub languages: Vec<String>,
    pub game_ids: Vec<String>,
    pub size: u64,
    pub manifest: String,
}

impl Depot {
    fn parse(v: &Value) -> Option<Self> {
        if v.get("redist").is_some() {
            return None;
        }
        let strings = |k: &str| v.get(k).and_then(Value::as_array).map(|l| l.iter().filter_map(api::id_text).collect()).unwrap_or_default();
        let size = v.get("size").and_then(|s| s.as_u64().or_else(|| s.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0);
        Some(Self {
            languages: strings("languages"),
            game_ids: strings("gameIDs"),
            size,
            manifest: v.get("manifest")?.as_str()?.to_string(),
        })
    }

    /// gogdl: `Neutral`, the install's language, or no language at all.
    pub fn has_language(&self, lang: Option<&Language>) -> bool {
        self.languages.is_empty() || self.languages.iter().any(|l| l == "Neutral" || lang.is_some_and(|t| t.matches(l)))
    }
}

/// The meta of a v1 build.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub raw: Value,
    pub root_game_id: String,
    pub install_directory: String,
    /// Part of the address of its depot manifests and `main.bin`.
    pub timestamp: String,
    pub depots: Vec<Depot>,
    /// Redistributable ids.
    pub dependencies: Vec<String>,
    /// `(gameID, English name)` of the game and its DLCs.
    pub products: Vec<(String, String)>,
}

impl Meta {
    pub fn parse(doc: &Value) -> Result<Self> {
        let product = doc.get("product").ok_or_else(|| anyhow!("the build has no product"))?;
        let text = |k: &str| product.get(k).and_then(api::id_text);
        let all_depots = product.get("depots").and_then(Value::as_array).cloned().unwrap_or_default();
        let products = product
            .get("gameIDs")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|g| {
                        let name = g.get("name").and_then(|n| n.get("en").and_then(Value::as_str).or(n.as_str())).unwrap_or_default();
                        Some((g.get("gameID").and_then(api::id_text)?, name.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            raw: doc.clone(),
            root_game_id: text("rootGameID").ok_or_else(|| anyhow!("the build has no product id"))?,
            install_directory: text("installDirectory").unwrap_or_default(),
            timestamp: text("timestamp").unwrap_or_default(),
            depots: all_depots.iter().filter_map(Depot::parse).collect(),
            dependencies: all_depots.iter().filter_map(|d| d.get("redist").and_then(api::id_text)).collect(),
            products,
        })
    }

    /// The DLCs the account owns; with `requested`, only those.
    pub fn owned_dlcs(&self, owned: &[String], requested: Option<&[String]>) -> Vec<Dlc> {
        self.products
            .iter()
            .filter(|(id, _)| *id != self.root_game_id && owned.contains(id))
            .filter(|(id, _)| requested.is_none_or(|r| r.contains(id)))
            .map(|(id, name)| Dlc { id: id.clone(), title: name.clone() })
            .collect()
    }

    fn product_depots(&self, dlcs: &[Dlc], dlc_only: bool) -> Vec<&Depot> {
        self.depots
            .iter()
            .filter(|d| d.game_ids.iter().any(|g| dlcs.iter().any(|x| &x.id == g) || (!dlc_only && *g == self.root_game_id)))
            .collect()
    }

    pub fn selected_depots(&self, lang: &str, dlcs: &[Dlc], dlc_only: bool) -> Vec<&Depot> {
        let lang = language::parse(lang);
        self.product_depots(dlcs, dlc_only).into_iter().filter(|d| d.has_language(lang.as_ref())).collect()
    }

    pub fn languages(&self, dlcs: &[Dlc]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in self.product_depots(dlcs, false) {
            for code in d.languages.iter().filter(|l| *l != "Neutral").filter_map(|l| language::parse(l)).map(|l| l.code.to_string()) {
                if !out.contains(&code) {
                    out.push(code);
                }
            }
        }
        out
    }

    /// Sizes per product and language; v1 depots are not compressed.
    pub fn sizes(&self, dlcs: &[Dlc]) -> BTreeMap<String, Sizes> {
        let mut out: BTreeMap<String, Sizes> = BTreeMap::new();
        for d in self.product_depots(dlcs, false) {
            for g in &d.game_ids {
                let product = out.entry(g.clone()).or_default();
                product.entry("*".into()).or_default();
                for l in &d.languages {
                    let key = if l == "Neutral" { "*".to_string() } else { language::code_of(l) };
                    let s = product.entry(key).or_default();
                    s.0 += d.size;
                    s.1 += d.size;
                }
            }
        }
        out
    }

    /// The document gogdl stored: the meta with the platform, the language
    /// and the DLCs.
    pub fn stored(&self, platform: &str, lang: &str, dlcs: &[Dlc]) -> Value {
        let mut doc = self.raw.clone();
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("HGLPlatform".into(), json!(platform));
            obj.insert("HGLInstallLanguage".into(), json!(language::code_of(lang)));
            obj.insert("HGLdlcs".into(), Value::Array(dlcs.iter().map(Dlc::to_json).collect()));
        }
        doc
    }

    /// The secure link path of a product's files.
    pub fn link_path(&self, platform: &str) -> String {
        format!("/{platform}/{}/", self.timestamp)
    }
}

/// The address of a v1 depot manifest.
pub fn depot_url(game_id: &str, platform: &str, timestamp: &str, manifest: &str) -> String {
    format!("{}/content-system/v1/manifests/{game_id}/{platform}/{timestamp}/{manifest}", api::CDN)
}

/// Reads a v1 depot manifest (`{"depot": {"files": [..]}}`) of `product`.
pub fn parse_depot(doc: &Value, product: &str) -> Items {
    let mut out = Items::default();
    for record in doc.pointer("/depot/files").and_then(Value::as_array).into_iter().flatten() {
        let path = files::clean_path(record.get("path").and_then(Value::as_str).unwrap_or_default());
        if record.get("directory").is_some() {
            out.dirs.push(path.trim_end_matches('/').to_string());
            continue;
        }
        let size = record.get("size").and_then(Value::as_u64).unwrap_or(0);
        let source = match record.get("offset").and_then(Value::as_u64) {
            _ if size == 0 => Source::Empty,
            Some(offset) => Source::Range { product: product.to_string(), offset },
            None => {
                log::warn!("v1 file {path} of {product} has no place in main.bin; skipped");
                continue;
            }
        };
        let support = record.get("support").and_then(Value::as_bool) == Some(true);
        out.files.push(FileEntry {
            path,
            root: if support { Root::Support } else { Root::Game },
            size,
            source,
            checksum: record.get("hash").and_then(Value::as_str).map(|h| Checksum::Md5(h.to_lowercase())),
            executable: record.get("executable").and_then(Value::as_bool) == Some(true),
        });
    }
    out
}

impl Client {
    /// The files of `depots` of a v1 build for `platform`.
    pub fn v1_items(&self, meta: &Meta, platform: &str, depots: &[&Depot]) -> Result<Items> {
        let mut all = Items::default();
        for d in depots {
            let game_id = d.game_ids.first().map(String::as_str).unwrap_or(&meta.root_game_id);
            let doc = self.get_json(&depot_url(game_id, platform, &meta.timestamp, &d.manifest))?;
            let items = parse_depot(&doc, game_id);
            all.files.extend(items.files);
            all.dirs.extend(items.dirs);
        }
        Ok(all)
    }
}

/// What an update of a v1 build changes (by md5); without `old`, everything.
pub fn diff(new: Items, old: Option<&[FileEntry]>) -> files::Diff {
    let mut out = files::Diff { dirs: new.dirs, ..Default::default() };
    let Some(old) = old else {
        out.files = new.files;
        return out;
    };
    let new_keys = files::by_key(&new.files);
    let old_keys = files::by_key(old);
    out.deleted = old.iter().filter(|f| !new_keys.contains_key(&f.key())).map(|f| (f.root, f.path.clone())).collect();
    out.files =
        new.files.into_iter().filter(|f| old_keys.get(&f.key()).is_none_or(|o| o.checksum != f.checksum || o.size != f.size)).collect();
    out
}

/// The files of a stored v1 build, fetched again.
pub fn stored_files(client: &Client, stored: &Value) -> Result<Vec<FileEntry>> {
    let meta = Meta::parse(stored)?;
    let platform = stored.get("HGLPlatform").and_then(Value::as_str).unwrap_or("windows").to_string();
    let lang = stored.get("HGLInstallLanguage").and_then(Value::as_str).unwrap_or("en-US").to_string();
    let dlcs = Dlc::list_from(stored.get("HGLdlcs"));
    Ok(client.v1_items(&meta, &platform, &meta.selected_depots(&lang, &dlcs, false))?.files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta::parse(&json!({"version": 1, "product": {
            "rootGameID": "1", "installDirectory": "Old Game", "timestamp": "123456",
            "gameIDs": [{"gameID": "1", "name": {"en": "Old Game"}}, {"gameID": "2", "name": {"en": "Old DLC"}}],
            "depots": [
                {"languages": ["Neutral"], "gameIDs": ["1"], "size": "1000", "manifest": "a.json"},
                {"languages": ["English"], "gameIDs": ["1"], "size": "100", "manifest": "b.json"},
                {"languages": ["French"], "gameIDs": ["1"], "size": "90", "manifest": "c.json"},
                {"languages": ["Neutral"], "gameIDs": ["2"], "size": "10", "manifest": "d.json"},
                {"redist": "DirectX", "size": 50, "targetDir": "__redist"}
            ]
        }}))
        .unwrap()
    }

    #[test]
    fn depots_languages_and_sizes() {
        let m = meta();
        assert_eq!(m.dependencies, ["DirectX"]);
        let dlcs = m.owned_dlcs(&["2".to_string()], None);
        assert_eq!(dlcs[0].title, "Old DLC");
        let names = |l: Vec<&Depot>| l.iter().map(|d| d.manifest.clone()).collect::<Vec<_>>();
        assert_eq!(names(m.selected_depots("en-US", &[], false)), ["a.json", "b.json"]);
        assert_eq!(names(m.selected_depots("fr-FR", &dlcs, false)), ["a.json", "c.json", "d.json"]);
        assert_eq!(m.languages(&dlcs), ["en-US", "fr-FR"]);
        let sizes = m.sizes(&dlcs);
        assert_eq!(sizes["1"]["*"], (1000, 1000));
        assert_eq!(sizes["1"]["en-US"], (100, 100));
        assert_eq!(sizes["2"]["*"], (10, 10));
        assert_eq!(m.stored("windows", "English", &dlcs)["HGLPlatform"], "windows");
        assert_eq!(m.link_path("windows"), "/windows/123456/");
        assert_eq!(
            depot_url("1", "windows", "123456", "a.json"),
            "https://gog-cdn-fastly.gog.com/content-system/v1/manifests/1/windows/123456/a.json"
        );
    }

    #[test]
    fn files_are_ranges_of_main_bin() {
        let doc = json!({"depot": {"files": [
            {"path": "/Game.exe", "size": 10, "hash": "AB", "offset": 500, "executable": true},
            {"path": "/support/x.ico", "size": 3, "hash": "cd", "offset": 0, "support": true},
            {"path": "/empty", "size": 0, "hash": "d41d8cd98f00b204e9800998ecf8427e"},
            {"directory": true, "path": "/Saves"}
        ]}});
        let items = parse_depot(&doc, "1");
        assert_eq!(items.files[0].source, Source::Range { product: "1".into(), offset: 500 });
        assert_eq!(items.files[0].checksum, Some(Checksum::Md5("ab".into())));
        assert!(items.files[0].executable);
        assert_eq!(items.files[1].root, Root::Support);
        assert_eq!(items.files[2].source, Source::Empty);
        assert_eq!(items.dirs, ["Saves"]);
        let updated = diff(parse_depot(&doc, "1"), Some(&items.files[..2]));
        assert_eq!(updated.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["empty"]);
    }
}
