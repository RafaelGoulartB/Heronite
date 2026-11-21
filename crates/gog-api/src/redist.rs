//! Redistributables (DirectX, Visual C++ runtimes, GOG's script
//! interpreter...). GOG's repository lists each one as a depot. Those whose
//! program lives under `__redist` are shared by every game and downloaded
//! into one folder, which keeps gogdl's `.gogdl-redist-manifest` (the
//! repository with `HGLInstalled`, the ids it holds); the others ship in the
//! game's folder with the game.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::api::{self, Client};
use crate::files::{Diff, FileEntry, Items};
use crate::v2;

/// The record of the shared folder.
pub const MANIFEST: &str = ".gogdl-redist-manifest";
/// The secure link key of redistributable files.
pub const PRODUCT: &str = "redist";

/// One redistributable of the repository.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoDepot {
    pub id: String,
    /// Its program, relative to the folder it is installed in.
    pub executable: String,
    pub manifest: String,
    pub compressed_size: u64,
    pub size: u64,
}

impl RepoDepot {
    fn parse(v: &Value) -> Option<Self> {
        Some(Self {
            id: v.get("dependencyId").and_then(api::id_text)?,
            executable: v.pointer("/executable/path").and_then(Value::as_str).unwrap_or_default().to_string(),
            manifest: v.get("manifest").and_then(Value::as_str)?.to_string(),
            compressed_size: v.get("compressedSize").and_then(Value::as_u64).unwrap_or(0),
            size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    /// Whether it goes into the shared folder rather than the game's.
    pub fn is_shared(&self) -> bool {
        self.executable.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase().starts_with("__redist")
    }
}

/// GOG's repository of redistributables.
#[derive(Debug, Clone, PartialEq)]
pub struct Repository {
    /// As gogdl kept it (with `build_id`).
    pub raw: Value,
    pub depots: Vec<RepoDepot>,
}

impl Repository {
    pub fn parse(doc: &Value) -> Self {
        let depots =
            doc.get("depots").and_then(Value::as_array).map(|l| l.iter().filter_map(RepoDepot::parse).collect()).unwrap_or_default();
        Self { raw: doc.clone(), depots }
    }

    /// The depots of `ids` that go in the shared folder (`shared`) or in the
    /// game's.
    pub fn depots_for(&self, ids: &[String], shared: bool) -> Vec<&RepoDepot> {
        self.depots.iter().filter(|d| ids.contains(&d.id) && d.is_shared() == shared).collect()
    }

    /// `(download, disk)` bytes of the game-folder redistributables of `ids`,
    /// which `gogdl info` adds to the game's `*` size.
    pub fn game_folder_size(&self, ids: &[String]) -> (u64, u64) {
        self.depots_for(ids, false).iter().fold((0, 0), |(a, b), d| (a + d.compressed_size, b + d.size))
    }
}

impl Client {
    /// The repository, as [`Repository`].
    pub fn repository(&self) -> Result<Repository> {
        Ok(Repository::parse(&self.dependencies_repository()?))
    }

    /// The files of redistributable depots (each manifest fetched in turn).
    pub fn dependency_files(&self, depots: &[&RepoDepot]) -> Result<Vec<FileEntry>> {
        let mut out = Vec::new();
        for d in depots {
            let (doc, _) = self.get_zlib_json(&api::dependency_depot_url(&d.manifest))?;
            out.extend(v2::parse_depot(&doc, PRODUCT).files);
        }
        Ok(out)
    }
}

/// The shared folder's record, if it has one.
pub fn read_installed(dir: &Path) -> Option<Value> {
    let doc: Value = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
    (doc.get("depots").is_some() && doc.get("build_id").is_some()).then_some(doc)
}

/// Ids `HGLInstalled` lists.
pub fn installed_ids(doc: &Value) -> Vec<String> {
    doc.get("HGLInstalled").and_then(Value::as_array).map(|l| l.iter().filter_map(api::id_text).collect()).unwrap_or_default()
}

/// Writes the shared folder's record: `repo` with `installed`.
pub fn write_installed(dir: &Path, repo: &Repository, installed: &[String]) -> Result<()> {
    let mut doc = repo.raw.clone();
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("HGLInstalled".into(), json!(installed));
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(MANIFEST);
    heronite_data::temps::write_atomic(&path, &serde_json::to_vec(&doc)?, None).with_context(|| format!("writing {}", path.display()))
}

/// What `gogdl redist --ids <ids> --path <dir>` did: the shared
/// redistributables of `ids` in `dir`, against what the folder's record
/// says it has. Returns the work and the ids to record once it is done
/// (the record then lists only those, as gogdl's did).
pub fn shared_plan(client: &Client, repo: &Repository, dir: &Path, ids: &[String]) -> Result<(Diff, Vec<String>)> {
    let new_depots = repo.depots_for(ids, true);
    let installed: Vec<String> = new_depots.iter().map(|d| d.id.clone()).collect();
    let old_files = match read_installed(dir) {
        Some(doc) => {
            let had = installed_ids(&doc);
            let old_repo = Repository::parse(&doc);
            let old: Vec<&RepoDepot> = old_repo.depots.iter().filter(|d| had.contains(&d.id)).collect();
            Some(client.dependency_files(&old)?)
        }
        None => None,
    };
    let new_files = client.dependency_files(&new_depots)?;
    let diff = v2::diff(Items { files: new_files, ..Default::default() }, old_files.as_deref());
    Ok((diff, installed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repository {
        Repository::parse(&json!({"build_id": "9", "depots": [
            {"dependencyId": "ISI", "executable": {"path": "__redist/ISI/scriptinterpreter.exe"}, "manifest": "m1", "compressedSize": 10, "size": 20},
            {"dependencyId": "MSVC2017", "executable": {"path": "__redist\\MSVC2017\\vc_redist.x86.exe"}, "manifest": "m2", "compressedSize": 30, "size": 40},
            {"dependencyId": "INGAME", "executable": {"path": "bin/helper.exe"}, "manifest": "m3", "compressedSize": 5, "size": 6}
        ]}))
    }

    #[test]
    fn shared_and_game_folder_redistributables() {
        let r = repo();
        let ids: Vec<String> = ["ISI", "MSVC2017", "INGAME", "NOPE"].iter().map(|s| s.to_string()).collect();
        assert_eq!(r.depots_for(&ids, true).iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["ISI", "MSVC2017"]);
        assert_eq!(r.depots_for(&ids, false).iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["INGAME"]);
        assert_eq!(r.game_folder_size(&ids), (5, 6));
    }

    #[test]
    fn the_record_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_installed(dir.path()).is_none());
        write_installed(dir.path(), &repo(), &["ISI".to_string()]).unwrap();
        let doc = read_installed(dir.path()).unwrap();
        assert_eq!(installed_ids(&doc), ["ISI"]);
        assert_eq!(doc["build_id"], "9");
    }
}
