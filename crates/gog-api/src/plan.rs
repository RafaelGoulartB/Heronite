//! What an install, update or repair has to do, worked out the way gogdl's
//! `download`, `update` and `repair` did: the build, the files to write and
//! to delete, the download addresses, and what to record once it is done.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use heronite_i18n::tr;
use serde_json::Value;

use crate::api::Client;
use crate::chunk::SecureLink;
use crate::files::{Diff, FileEntry, Source};
use crate::redist::{self, Repository};
use crate::{info, linux, stored, v1, v2};

/// A repair of a game with no stored build: nothing says what its files
/// should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoRecord;

impl std::fmt::Display for NoRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("there is no record of this game's files")
    }
}

impl std::error::Error for NoRecord {}

/// What to work on.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub game_id: &'a str,
    /// `windows`, `osx` or `linux`.
    pub os: &'a str,
    /// A language code (`en-US`).
    pub language: &'a str,
    /// `None` leaves DLCs out; `Some(ids)` takes those the account owns
    /// (every owned one when empty).
    pub dlcs: Option<&'a [String]>,
    pub branch: Option<&'a str>,
    pub build_id: Option<&'a str>,
    /// Of a private branch.
    pub password: Option<&'a str>,
    /// The installed game's folder (updates and repairs of Linux builds).
    pub game_dir: Option<&'a Path>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Everything of the build (an earlier record is ignored).
    Install,
    /// What changed since the recorded build.
    Update,
    /// Every file of the recorded build, to check on disk.
    Repair,
}

/// The work, ready for the download engine.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub diff: Diff,
    /// The folder below the chosen path an install goes in.
    pub folder_name: String,
    /// 1 or 2 for the content system, 0 for a Linux installer.
    pub generation: u8,
    /// The secure link of each product the files come from (content system).
    pub links: HashMap<String, SecureLink>,
    /// The opened installers (Linux).
    pub installers: Vec<linux::Installer>,
    /// The build record to save once the files are in place (content system).
    pub stored: Option<Value>,
    /// The build id an install records (`legacy_build_id` for v1).
    pub build_id: String,
    pub version_name: String,
    pub version_etag: Option<String>,
    /// The DLCs taken.
    pub dlcs: Vec<v2::Dlc>,
}

fn products_of(files: &[FileEntry]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in files {
        let p = match &f.source {
            Source::Chunks { product, .. } | Source::Range { product, .. } => product,
            _ => continue,
        };
        if !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

/// Works out `request` in `mode`; `config_dir` is gogdl's (`gogdlConfig`).
pub fn prepare(client: &Client, config_dir: &Path, request: &Request, mode: Mode) -> Result<Prepared> {
    if request.os == "linux" {
        return prepare_linux(client, request, mode);
    }
    let record = match mode {
        Mode::Install => None,
        Mode::Update | Mode::Repair => stored::read(config_dir, request.game_id),
    };
    if mode == Mode::Repair && record.is_none() {
        return Err(anyhow!(NoRecord));
    }
    let repo = client.repository();
    let (new_doc, build, etag) = match (mode, &record) {
        (Mode::Repair, Some(doc)) => (doc.clone(), None, None),
        _ => {
            let builds = client.builds(request.game_id, request.os, request.password)?;
            let build = builds.pick(request.branch, request.build_id).cloned().ok_or_else(|| anyhow!("no build"))?;
            let (doc, etag) = client.meta(&build)?;
            (doc, Some(build), etag)
        }
    };
    let generation = stored::generation(&new_doc);
    let owned = if request.dlcs.is_some() && mode != Mode::Repair { client.owned()? } else { Vec::new() };
    let requested = request.dlcs.filter(|l| !l.is_empty());

    let (mut items, stored_doc, dlcs, folder, deps, link_path) = if generation == 1 {
        let m = v1::Meta::parse(&new_doc)?;
        let dlcs = if mode == Mode::Repair {
            v2::Dlc::list_from(new_doc.get("HGLdlcs"))
        } else if request.dlcs.is_some() {
            m.owned_dlcs(&owned, requested)
        } else {
            Vec::new()
        };
        let lang = if mode == Mode::Repair {
            new_doc.get("HGLInstallLanguage").and_then(Value::as_str).unwrap_or(request.language).to_string()
        } else {
            request.language.to_string()
        };
        let platform =
            new_doc.get("HGLPlatform").and_then(Value::as_str).filter(|_| mode == Mode::Repair).unwrap_or(request.os).to_string();
        let items = client.v1_items(&m, &platform, &m.selected_depots(&lang, &dlcs, false))?;
        let stored_doc = if mode == Mode::Repair { new_doc.clone() } else { m.stored(&platform, &lang, &dlcs) };
        (items, stored_doc, dlcs, m.install_directory.clone(), m.dependencies.clone(), m.link_path(&platform))
    } else {
        let m = v2::Meta::parse(&new_doc)?;
        let dlcs = if mode == Mode::Repair {
            v2::Dlc::list_from(new_doc.get("HGLdlcs"))
        } else if request.dlcs.is_some() {
            m.owned_dlcs(&owned, requested)
        } else {
            Vec::new()
        };
        let lang = if mode == Mode::Repair {
            new_doc.get("HGLInstallLanguage").and_then(Value::as_str).unwrap_or(request.language).to_string()
        } else {
            request.language.to_string()
        };
        let items = client.depot_items(&m.selected_depots(&lang, &dlcs, false))?;
        let stored_doc = if mode == Mode::Repair { new_doc.clone() } else { m.stored(&lang, &dlcs) };
        (items, stored_doc, dlcs, m.install_directory.clone(), m.dependencies.clone(), "/".to_string())
    };

    // Redistributables that live in the game's folder come with the game.
    let repo: Option<Repository> = match repo {
        Ok(r) => Some(r),
        Err(err) if deps.is_empty() => {
            log::debug!("the GOG redist repository: {err:#}");
            None
        }
        Err(err) => return Err(err),
    };
    if let Some(repo) = &repo {
        items.files.extend(client.dependency_files(&repo.depots_for(&deps, false))?);
    }

    let old_files = match (mode, &record) {
        (Mode::Update, Some(doc)) => {
            let mut files = stored::files(client, doc)?;
            if let Some(repo) = &repo {
                let old_deps: Vec<String> = match stored::generation(doc) {
                    1 => v1::Meta::parse(doc).map(|m| m.dependencies).unwrap_or_default(),
                    _ => v2::Meta::parse(doc).map(|m| m.dependencies).unwrap_or_default(),
                };
                files.extend(client.dependency_files(&repo.depots_for(&old_deps, false))?);
            }
            Some(files)
        }
        _ => None,
    };
    let diff = if generation == 1 { v1::diff(items, old_files.as_deref()) } else { v2::diff(items, old_files.as_deref()) };

    let mut links = HashMap::new();
    for product in products_of(&diff.files) {
        let link = if product == redist::PRODUCT {
            client.dependency_link()?
        } else {
            client.secure_link(&product, &link_path, generation, None)?
        };
        links.insert(product, link);
    }
    let (build_id, version_name) = match &build {
        Some(b) => (b.recorded_id().to_string(), b.version_name.clone()),
        None => (stored_build_id(&new_doc), String::new()),
    };
    Ok(Prepared {
        diff,
        folder_name: folder,
        generation,
        links,
        installers: Vec::new(),
        stored: Some(stored_doc),
        build_id,
        version_name,
        version_etag: etag,
        dlcs,
    })
}

/// The build id in a stored record (`buildId`, or v1's `product.timestamp`
/// is no id: empty then).
fn stored_build_id(doc: &Value) -> String {
    doc.get("buildId").and_then(crate::api::id_text).unwrap_or_default()
}

fn prepare_linux(client: &Client, request: &Request, mode: Mode) -> Result<Prepared> {
    let product = client.product_downloads(request.game_id)?;
    let installers = linux::linux_installers(&product);
    let chosen = linux::pick_installer(&installers, request.language)
        .ok_or_else(|| anyhow!(tr!("This game cannot be installed for {platform}", platform = request.os)))?;
    let mut opened = vec![client.open_installer(chosen, request.game_id)?];
    let mut dlcs = Vec::new();
    if let Some(wanted) = request.dlcs {
        let owned = client.owned()?;
        for dlc in product.get("expanded_dlcs").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = dlc.get("id").and_then(crate::api::id_text) else { continue };
            if !owned.contains(&id) || (!wanted.is_empty() && !wanted.contains(&id)) {
                continue;
            }
            let list = linux::linux_installers(dlc);
            match linux::pick_installer(&list, request.language) {
                Some(inst) => {
                    opened.push(client.open_installer(inst, &id)?);
                    dlcs.push(v2::Dlc { id, title: dlc.get("title").and_then(Value::as_str).unwrap_or_default().to_string() });
                }
                None => log::warn!("GOG DLC {id} has no Linux installer"),
            }
        }
    }
    let old = match (mode, request.game_dir) {
        (Mode::Update, Some(dir)) => linux::read_record(dir),
        _ => None,
    };
    let (mut diff, symlinks) = linux::diff(&opened, old.as_ref());
    for (product_id, name) in symlinks {
        let Some(inst) = opened.iter().find(|i| i.product == product_id) else { continue };
        let Some(entry) = inst.entries.iter().find(|e| e.name == name) else { continue };
        diff.links.push(client.link_of(inst, entry)?);
    }
    Ok(Prepared {
        diff,
        folder_name: info::folder_from_windows_build(client, request.game_id),
        generation: 0,
        links: HashMap::new(),
        installers: opened,
        stored: None,
        build_id: String::new(),
        version_name: chosen.get("version").and_then(Value::as_str).unwrap_or_default().to_string(),
        version_etag: None,
        dlcs,
    })
}

/// The files of `diff` that are missing or damaged under `game_dir` and
/// `support_dir` (a repair downloads only those). Reads every file.
pub fn broken(diff: &Diff, game_dir: &Path, support_dir: &Path) -> Result<Diff> {
    let mut out = Diff { links: Vec::new(), dirs: diff.dirs.clone(), ..Default::default() };
    for f in &diff.files {
        let root = match f.root {
            crate::files::Root::Game => game_dir,
            crate::files::Root::Support => support_dir,
        };
        if !crate::files::matches_on_disk(&root.join(&f.path), f)? {
            out.files.push(f.clone());
        }
    }
    for l in &diff.links {
        if std::fs::symlink_metadata(game_dir.join(&l.path)).is_err() {
            out.links.push(l.clone());
        }
    }
    Ok(out)
}
