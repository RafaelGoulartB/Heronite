//! Builds of the Galaxy 2 content system: the build's meta (products,
//! depots per language, redistributables, install folder) and the depot
//! manifests that list the files and their zlib chunks.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::api::{self, Client};
use crate::files::{self, Checksum, ChunkPart, ChunkRef, Diff, FileEntry, Items, Link, Root, Source};
use crate::language::{self, Language};

/// `(download, disk)` bytes per language key (`*` = every language).
pub type Sizes = BTreeMap<String, (u64, u64)>;

/// A DLC the account owns, as gogdl recorded it (`{"title", "id"}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dlc {
    pub id: String,
    pub title: String,
}

impl Dlc {
    pub fn to_json(&self) -> Value {
        json!({"title": self.title, "id": self.id})
    }

    /// The DLCs of gogdl's `HGLdlcs`.
    pub fn list_from(v: Option<&Value>) -> Vec<Dlc> {
        v.and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|d| {
                        Some(Dlc {
                            id: d.get("id").and_then(api::id_text)?,
                            title: d.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// One depot of the meta.
#[derive(Debug, Clone, PartialEq)]
pub struct Depot {
    pub product_id: String,
    pub languages: Vec<String>,
    /// `32` / `64`; empty for both.
    pub bitness: Vec<String>,
    pub compressed_size: u64,
    pub size: u64,
    pub manifest: String,
}

impl Depot {
    fn parse(v: &Value) -> Option<Self> {
        let strings = |k: &str| v.get(k).and_then(Value::as_array).map(|l| l.iter().filter_map(api::id_text).collect()).unwrap_or_default();
        Some(Self {
            product_id: v.get("productId").and_then(api::id_text)?,
            languages: strings("languages"),
            bitness: strings("osBitness"),
            compressed_size: v.get("compressedSize").and_then(Value::as_u64).unwrap_or(0),
            size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
            manifest: v.get("manifest").and_then(Value::as_str)?.to_string(),
        })
    }

    /// gogdl: a depot of `*` or of the install's language.
    pub fn has_language(&self, lang: Option<&Language>) -> bool {
        self.languages.iter().any(|l| l == "*" || lang.is_some_and(|t| t.matches(l)))
    }

    /// Depots for one bitness only are those of a 64-bit system.
    pub fn fits_system(&self) -> bool {
        self.bitness.is_empty() || self.bitness.iter().any(|b| b == "64")
    }
}

/// The meta of a v2 build.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    /// The document as GOG sent it (what gogdl stored, with its own keys).
    pub raw: Value,
    pub base_product_id: String,
    pub install_directory: String,
    pub depots: Vec<Depot>,
    /// Redistributable ids (`dependencies`).
    pub dependencies: Vec<String>,
    /// `(productId, name)` of the game and its DLCs.
    pub products: Vec<(String, String)>,
    /// The game's own client, for its cloud saves.
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

impl Meta {
    pub fn parse(doc: &Value) -> Result<Self> {
        let text = |k: &str| doc.get(k).and_then(api::id_text);
        let products = doc
            .get("products")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|p| {
                        Some((
                            p.get("productId").and_then(api::id_text)?,
                            p.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            raw: doc.clone(),
            base_product_id: text("baseProductId").ok_or_else(|| anyhow!("the build has no product id"))?,
            install_directory: text("installDirectory").unwrap_or_default(),
            depots: doc.get("depots").and_then(Value::as_array).map(|l| l.iter().filter_map(Depot::parse).collect()).unwrap_or_default(),
            dependencies: doc
                .get("dependencies")
                .and_then(Value::as_array)
                .map(|l| l.iter().filter_map(api::id_text).collect())
                .unwrap_or_default(),
            products,
            client_id: text("clientId"),
            client_secret: text("clientSecret"),
        })
    }

    /// The DLCs of the build the account owns (`owned` ids); with
    /// `requested`, only those.
    pub fn owned_dlcs(&self, owned: &[String], requested: Option<&[String]>) -> Vec<Dlc> {
        self.products
            .iter()
            .filter(|(id, _)| *id != self.base_product_id && owned.contains(id))
            .filter(|(id, _)| requested.is_none_or(|r| r.contains(id)))
            .map(|(id, name)| Dlc { id: id.clone(), title: name.clone() })
            .collect()
    }

    /// Depots of the game (unless `dlc_only`) and of `dlcs`, any language.
    pub fn product_depots(&self, dlcs: &[Dlc], dlc_only: bool) -> Vec<&Depot> {
        self.depots
            .iter()
            .filter(|d| dlcs.iter().any(|x| x.id == d.product_id) || (!dlc_only && d.product_id == self.base_product_id))
            .collect()
    }

    /// The depots an install of `lang` downloads.
    pub fn selected_depots(&self, lang: &str, dlcs: &[Dlc], dlc_only: bool) -> Vec<&Depot> {
        let lang = language::parse(lang);
        self.product_depots(dlcs, dlc_only).into_iter().filter(|d| d.has_language(lang.as_ref()) && d.fits_system()).collect()
    }

    /// Codes of the languages of the depots of the game and `dlcs`.
    pub fn languages(&self, dlcs: &[Dlc]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in self.product_depots(dlcs, false) {
            for l in d.languages.iter().filter(|l| *l != "*") {
                if let Some(code) = language::parse(l).map(|l| l.code.to_string()) {
                    if !out.contains(&code) {
                        out.push(code);
                    }
                }
            }
        }
        out
    }

    /// Sizes per product and language, like `gogdl info`.
    pub fn sizes(&self, dlcs: &[Dlc]) -> BTreeMap<String, Sizes> {
        let mut out: BTreeMap<String, Sizes> = BTreeMap::new();
        for d in self.product_depots(dlcs, false) {
            let product = out.entry(d.product_id.clone()).or_default();
            product.entry("*".into()).or_default();
            for l in &d.languages {
                let s = product.entry(language::code_of(l)).or_default();
                s.0 += d.compressed_size;
                s.1 += d.size;
            }
        }
        out
    }

    /// The document gogdl stored for the install: the meta with the
    /// language and the DLCs.
    pub fn stored(&self, lang: &str, dlcs: &[Dlc]) -> Value {
        let mut doc = self.raw.clone();
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("HGLInstallLanguage".into(), json!(language::code_of(lang)));
            obj.insert("HGLdlcs".into(), Value::Array(dlcs.iter().map(Dlc::to_json).collect()));
        }
        doc
    }
}

/// Reads a depot manifest (`{"depot": {"items": [..]}}`) of `product`;
/// files flagged `support` go in the support folder under the product id.
pub fn parse_depot(doc: &Value, product: &str) -> Items {
    let depot = doc.get("depot").unwrap_or(&Value::Null);
    let container = small_files_container(depot);
    let mut out = Items::default();
    for item in depot.get("items").and_then(Value::as_array).into_iter().flatten() {
        let path = files::clean_path(item.get("path").and_then(Value::as_str).unwrap_or_default());
        match item.get("type").and_then(Value::as_str) {
            Some("DepotFile") => {
                if let Some(f) = depot_file(item, &path, product, container.as_deref()) {
                    out.files.push(f);
                }
            }
            Some("DepotLink") => {
                out.links.push(Link { path, target: item.get("target").and_then(Value::as_str).unwrap_or_default().to_string() })
            }
            _ => out.dirs.push(path.trim_end_matches('/').to_string()),
        }
    }
    out
}

fn chunk_ref(c: &Value) -> Option<ChunkRef> {
    Some(ChunkRef {
        md5: c.get("md5").and_then(Value::as_str)?.to_string(),
        compressed_md5: c.get("compressedMd5").and_then(Value::as_str)?.to_string(),
        size: c.get("size").and_then(Value::as_u64)?,
        compressed_size: c.get("compressedSize").and_then(Value::as_u64)?,
    })
}

/// The chunks of the depot's small files container, if it has one.
fn small_files_container(depot: &Value) -> Option<Vec<ChunkRef>> {
    let list = depot.get("smallFilesContainer")?.get("chunks")?.as_array()?;
    list.iter().map(chunk_ref).collect()
}

/// The parts of `container` that hold `size` bytes from `offset`.
fn container_parts(container: &[ChunkRef], offset: u64, size: u64) -> Option<Vec<ChunkPart>> {
    let mut parts = Vec::new();
    let (mut start, end) = (offset, offset + size);
    let mut chunk_start = 0;
    for c in container {
        let chunk_end = chunk_start + c.size;
        if start < chunk_end && start < end {
            let take = end.min(chunk_end) - start;
            parts.push(ChunkPart { chunk: c.clone(), offset: start - chunk_start, size: take, old_offset: None });
            start += take;
        }
        chunk_start = chunk_end;
    }
    (start == end).then_some(parts)
}

fn depot_file(item: &Value, path: &str, product: &str, container: Option<&[ChunkRef]>) -> Option<FileEntry> {
    let flags: Vec<&str> =
        item.get("flags").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let chunks: Vec<ChunkRef> =
        item.get("chunks").and_then(Value::as_array).map(|l| l.iter().filter_map(chunk_ref).collect()).unwrap_or_default();
    let mut parts: Vec<ChunkPart> = chunks.into_iter().map(|c| ChunkPart { size: c.size, chunk: c, offset: 0, old_offset: None }).collect();
    if parts.is_empty() {
        if let (Some(sfc), Some(r)) = (container, item.get("sfcRef")) {
            let (offset, size) = (r.get("offset").and_then(Value::as_u64)?, r.get("size").and_then(Value::as_u64)?);
            parts = container_parts(sfc, offset, size)?;
        }
    }
    let size: u64 = parts.iter().map(|p| p.size).sum();
    let md5 = item.get("md5").and_then(Value::as_str).map(str::to_lowercase);
    let sha256 = item.get("sha256").and_then(Value::as_str).map(str::to_lowercase);
    // One whole chunk is the whole file: its md5 is the file's.
    let single = (parts.len() == 1 && parts[0].offset == 0).then(|| parts[0].chunk.md5.to_lowercase());
    let checksum = md5.map(Checksum::Md5).or(sha256.map(Checksum::Sha256)).or(single.map(Checksum::Md5));
    let support = flags.contains(&"support");
    Some(FileEntry {
        path: if support { format!("{product}/{path}") } else { path.to_string() },
        root: if support { Root::Support } else { Root::Game },
        size,
        source: if parts.is_empty() { Source::Empty } else { Source::Chunks { product: product.to_string(), parts } },
        checksum,
        executable: flags.contains(&"executable"),
    })
}

impl Client {
    /// The items of `depots` (each depot manifest fetched in turn).
    pub fn depot_items(&self, depots: &[&Depot]) -> Result<Items> {
        let mut all = Items::default();
        for d in depots {
            let (doc, _) = self.get_zlib_json(&api::depot_url(&d.manifest))?;
            let items = parse_depot(&doc, &d.product_id);
            all.files.extend(items.files);
            all.links.extend(items.links);
            all.dirs.extend(items.dirs);
        }
        Ok(all)
    }
}

/// What an install or update of a v2 build changes: everything new without
/// an `old` build; else files gone, new, or with other bytes. A changed file
/// reuses the chunks it already has (`old_offset`). Links are always made.
/// The old files may be of a v1 build: those compare by md5.
pub fn diff(new: Items, old: Option<&[FileEntry]>) -> Diff {
    let mut out = Diff { links: new.links, dirs: new.dirs, ..Default::default() };
    let Some(old) = old else {
        out.files = new.files;
        return out;
    };
    let new_keys = files::by_key(&new.files);
    let old_keys = files::by_key(old);
    for f in old {
        if !new_keys.contains_key(&f.key()) {
            out.deleted.push((f.root, f.path.clone()));
        }
    }
    for mut f in new.files {
        let Some(prev) = old_keys.get(&f.key()) else {
            out.files.push(f);
            continue;
        };
        if !changed(&f, prev) {
            continue;
        }
        reuse_chunks(&mut f, prev);
        out.files.push(f);
    }
    out
}

/// gogdl's comparison: by whole checksum when both have one, else by the
/// list of chunks.
fn changed(new: &FileEntry, old: &FileEntry) -> bool {
    match (&new.checksum, &old.checksum) {
        (Some(a), Some(b)) if std::mem::discriminant(a) == std::mem::discriminant(b) => a != b || new.size != old.size,
        _ => new.piece_sums() != old.piece_sums() || new.size != old.size,
    }
}

/// Points the chunks `new` shares with `old` at their place in the old file.
fn reuse_chunks(new: &mut FileEntry, old: &FileEntry) {
    // Bytes read from the old file must be at the same path and root.
    if new.root != old.root {
        return;
    }
    let mut offsets: std::collections::HashMap<&str, u64> = std::collections::HashMap::new();
    if let Source::Chunks { parts, .. } = &old.source {
        let mut at = 0;
        for p in parts {
            if p.offset == 0 && p.size == p.chunk.size {
                offsets.entry(p.chunk.md5.as_str()).or_insert(at);
            }
            at += p.size;
        }
    }
    if let Source::Chunks { parts, .. } = &mut new.source {
        for p in parts.iter_mut().filter(|p| p.offset == 0 && p.size == p.chunk.size) {
            p.old_offset = offsets.get(p.chunk.md5.as_str()).copied();
        }
    }
}

/// The files of a stored v2 build (`old` of an update), fetched again from
/// its depots.
pub fn stored_files(client: &Client, stored: &Value) -> Result<Vec<FileEntry>> {
    let meta = Meta::parse(stored)?;
    let lang = stored.get("HGLInstallLanguage").and_then(Value::as_str).unwrap_or("en-US").to_string();
    let dlcs = Dlc::list_from(stored.get("HGLdlcs"));
    Ok(client.depot_items(&meta.selected_depots(&lang, &dlcs, false))?.files)
}

/// The meta keys a stored build always has, for a quick check of a file
/// read from disk.
pub fn looks_stored(doc: &Map<String, Value>) -> bool {
    doc.contains_key("baseProductId") && doc.contains_key("depots")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::testing::file;

    fn meta() -> Meta {
        Meta::parse(&json!({
            "version": 2, "baseProductId": "100", "installDirectory": "The Game", "buildId": "5",
            "clientId": "cid", "clientSecret": "sec", "dependencies": ["MSVC2017", "DirectX"],
            "products": [{"productId": "100", "name": "The Game"}, {"productId": "200", "name": "Expansion"}, {"productId": "300", "name": "Not owned"}],
            "depots": [
                {"productId": "100", "languages": ["*"], "size": 1000, "compressedSize": 600, "manifest": "m1"},
                {"productId": "100", "languages": ["en-US"], "size": 100, "compressedSize": 60, "manifest": "m2"},
                {"productId": "100", "languages": ["de"], "size": 90, "compressedSize": 50, "manifest": "m3"},
                {"productId": "100", "languages": ["*"], "osBitness": ["32"], "size": 5, "compressedSize": 5, "manifest": "m32"},
                {"productId": "200", "languages": ["*"], "size": 10, "compressedSize": 7, "manifest": "m4"},
                {"productId": "300", "languages": ["*"], "size": 1, "compressedSize": 1, "manifest": "m5"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn depots_follow_language_bitness_and_dlcs() {
        let m = meta();
        let owned: Vec<String> = vec!["100".into(), "200".into()];
        let dlcs = m.owned_dlcs(&owned, None);
        assert_eq!(dlcs, [Dlc { id: "200".into(), title: "Expansion".into() }]);
        assert!(m.owned_dlcs(&owned, Some(&[])).is_empty());
        let names = |l: Vec<&Depot>| l.iter().map(|d| d.manifest.clone()).collect::<Vec<_>>();
        assert_eq!(names(m.selected_depots("en-US", &[], false)), ["m1", "m2"]);
        assert_eq!(names(m.selected_depots("German", &dlcs, false)), ["m1", "m3", "m4"]);
        assert_eq!(names(m.selected_depots("en-US", &dlcs, true)), ["m4"]);
        assert_eq!(m.languages(&dlcs), ["en-US", "de-DE"]);
        let sizes = m.sizes(&dlcs);
        assert_eq!(sizes["100"]["*"], (605, 1005));
        assert_eq!(sizes["100"]["en-US"], (60, 100));
        assert_eq!(sizes["100"]["de-DE"], (50, 90));
        assert_eq!(sizes["200"]["*"], (7, 10));
        assert!(!sizes.contains_key("300"));
        let stored = m.stored("en", &dlcs);
        assert_eq!(stored["HGLInstallLanguage"], "en-US");
        assert_eq!(stored["HGLdlcs"], json!([{"title": "Expansion", "id": "200"}]));
        assert_eq!(Dlc::list_from(stored.get("HGLdlcs")), dlcs);
        assert!(looks_stored(stored.as_object().unwrap()));
        assert_eq!((m.client_id.as_deref(), m.dependencies.len()), (Some("cid"), 2));
    }

    #[test]
    fn depot_items_are_read() {
        let doc = json!({"depot": {"items": [
            {"type": "DepotFile", "path": "bin\\game.exe", "flags": ["executable"], "md5": "ABC",
             "chunks": [{"md5": "c1", "compressedMd5": "z1", "size": 10, "compressedSize": 4}, {"md5": "c2", "compressedMd5": "z2", "size": 5, "compressedSize": 3}]},
            {"type": "DepotFile", "path": "setup.exe", "flags": ["support"], "chunks": [{"md5": "c3", "compressedMd5": "z3", "size": 7, "compressedSize": 7}]},
            {"type": "DepotFile", "path": "empty.txt", "chunks": []},
            {"type": "DepotDirectory", "path": "saves\\"},
            {"type": "DepotLink", "path": "lib/libfoo.so", "target": "libfoo.so.1"}
        ]}});
        let items = parse_depot(&doc, "100");
        let exe = &items.files[0];
        assert_eq!((exe.path.as_str(), exe.size, exe.executable, exe.root), ("bin/game.exe", 15, true, Root::Game));
        assert_eq!(exe.checksum, Some(Checksum::Md5("abc".into())));
        assert_eq!(exe.download_size(), 7);
        let setup = &items.files[1];
        assert_eq!((setup.path.as_str(), setup.root), ("100/setup.exe", Root::Support));
        assert_eq!(setup.checksum, Some(Checksum::Md5("c3".into())));
        assert_eq!(items.files[2].source, Source::Empty);
        assert_eq!(items.dirs, ["saves"]);
        assert_eq!(items.links[0].target, "libfoo.so.1");
    }

    #[test]
    fn small_files_come_out_of_the_container() {
        let doc = json!({"depot": {
            "smallFilesContainer": {"chunks": [{"md5": "a", "compressedMd5": "za", "size": 10, "compressedSize": 5}, {"md5": "b", "compressedMd5": "zb", "size": 10, "compressedSize": 5}]},
            "items": [{"type": "DepotFile", "path": "x", "chunks": [], "sfcRef": {"offset": 8, "size": 4}, "md5": "m"}]
        }});
        let items = parse_depot(&doc, "1");
        let Source::Chunks { parts, .. } = &items.files[0].source else { panic!() };
        assert_eq!(parts.iter().map(|p| (p.chunk.md5.as_str(), p.offset, p.size)).collect::<Vec<_>>(), [("a", 8, 2), ("b", 0, 2)]);
        assert_eq!(items.files[0].size, 4);
    }

    #[test]
    fn updates_download_only_what_changed_and_reuse_old_chunks() {
        let old = vec![file("same", &[("s", 5)]), file("Changed", &[("a", 5), ("b", 5)]), file("gone", &[("g", 1)])];
        let new = Items {
            files: vec![file("same", &[("s", 5)]), file("changed", &[("a", 5), ("c", 5)]), file("added", &[("n", 2)])],
            ..Default::default()
        };
        let d = diff(new, Some(&old));
        assert_eq!(d.deleted, [(Root::Game, "gone".to_string())]);
        assert_eq!(d.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["changed", "added"]);
        let Source::Chunks { parts, .. } = &d.files[0].source else { panic!() };
        assert_eq!(parts.iter().map(|p| p.old_offset).collect::<Vec<_>>(), [Some(0), None]);
        let fresh = diff(Items { files: old.clone(), ..Default::default() }, None);
        assert_eq!(fresh.files.len(), 3);
    }
}
