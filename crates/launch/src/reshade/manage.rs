//! Everything done to an installed ReShade: switch it on and off, update
//! the DLL or the shaders, add and remove shader sources, import a preset,
//! remove it all, and take over an install made by hand.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;

use super::catalog::Package;
use super::detect::{occupant, Api, Arch, Occupant};
use super::fetch::{is_known_compiler, repo_zip_url, sha256_file, Build, Cache};
use super::ini::Ini;
use super::install::{config_text, copy_preset, install_archive, now, ours, Journal, Ops, Step, COMPILER, CONFIG};
use super::manifest::{self, join, Kind, Manifest, ADOPTED};
use crate::gfxmods::anticheat::{Policy, Risk};
use crate::gfxmods::journal::journaled;

fn load(paths: &DataPaths, game_id: &str) -> Result<Manifest, String> {
    manifest::load(paths, game_id).ok_or_else(|| "ReShade is not installed for this game".to_string())
}

/// Renames the DLL: Windows loads any DLL in the game's folder, and under
/// Proton the DXVK `dxgi` is native already, so only the file name turns
/// ReShade on or off.
pub fn set_enabled(paths: &DataPaths, game_id: &str, on: bool) -> Result<Manifest, String> {
    let mut m = load(paths, game_id)?;
    if m.enabled == on {
        return Ok(m);
    }
    let from = m.dll_path();
    m.enabled = on;
    let to = m.dll_path();
    if !from.is_file() {
        return Err(format!("{} is missing", from.display()));
    }
    if to.exists() {
        return Err(format!("{} is in the way", to.display()));
    }
    std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    manifest::save(paths, game_id, &m)?;
    Ok(m)
}

/// Replaces only the DLL (and nothing the user set up: INI, preset and
/// shaders stay). The build stays the one installed.
pub fn update_dll(ops: &Ops, game_id: &str, version: &str) -> Result<Manifest, String> {
    let m = load(ops.paths, game_id)?;
    let addon = m.addon;
    replace_dll(ops, game_id, m, version, addon)
}

/// Switches to the build with add-on support (`on`) or back to the standard
/// one, at the installed version. `risk`: the game's, from the overview; the
/// add-on build is refused for a game with any known anti-cheat. Going back
/// is refused while an add-on Heronite installed is there (it would stop
/// loading).
pub fn set_addon(ops: &Ops, game_id: &str, on: bool, risk: &Risk) -> Result<Manifest, String> {
    let m = load(ops.paths, game_id)?;
    if m.addon == on {
        return Ok(m);
    }
    if m.version.trim().is_empty() {
        return Err("ReShade's version is not known here: update ReShade first".into());
    }
    if on && risk.blocks(Policy::Block) {
        return Err("the build with add-on support is not offered for games with anti-cheat".into());
    }
    if !on && m.files.iter().any(|f| f.kind == Kind::Addon) {
        return Err("remove the ReShade add-ons first".into());
    }
    let version = m.version.clone();
    replace_dll(ops, game_id, m, &version, on)
}

fn replace_dll(ops: &Ops, game_id: &str, mut m: Manifest, version: &str, addon: bool) -> Result<Manifest, String> {
    let arch = arch_of(&m)?;
    (ops.progress)(Step::Dll(version.to_string()));
    let dll = Cache::new(ops.paths).build_dll(ops.net, version, Build::of(addon), arch, ops.cancel)?;
    let dest = m.dll_path();
    if let Occupant::Other { path, .. } = occupant(&m.dir(), &m.proxy) {
        return Err(format!("{} is no longer ReShade", path.display()));
    }
    let sha = journaled(|j| j.copy(&dll, &dest))?;
    m.record(&m.proxy.clone(), sha, Kind::Dll, None);
    m.version = version.to_string();
    m.addon = addon;
    manifest::save(ops.paths, game_id, &m)?;
    Ok(m)
}

pub(super) fn arch_of(m: &Manifest) -> Result<Arch, String> {
    match m.arch.as_str() {
        "x86" => Ok(Arch::X86),
        "x64" => Ok(Arch::X64),
        a => Err(format!("unknown architecture {a}")),
    }
}

/// A user's shader source: a local folder or a repository address.
fn is_url(source: &str) -> bool {
    source.starts_with("https://") || source.starts_with("http://")
}

/// The folder name of an extra source under `Shaders/` and `Textures/`.
fn source_folder(source: &str) -> String {
    let name = source
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("Extra")
        .trim_end_matches(".git")
        .trim_end_matches(".zip");
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect();
    match clean.trim() {
        "" => "Extra".into(),
        n => n.to_string(),
    }
}

fn extra_roots(source: &str) -> (String, String) {
    let name = source_folder(source);
    (format!("reshade-shaders/Shaders/{name}"), format!("reshade-shaders/Textures/{name}"))
}

/// Installs catalog packages and extra sources into an installed ReShade.
/// `refresh`: download them again (the "Update shaders" button).
fn add_sources(ops: &Ops, m: &mut Manifest, packages: &[Package], extras: &[String], refresh: bool) -> Result<(), String> {
    let cache = Cache::new(ops.paths);
    let previous = m.clone();
    journaled(|j| {
        for p in packages {
            (ops.progress)(Step::Shaders(p.name.clone()));
            let zip = cache.package(ops.net, &p.download_url, refresh, ops.cancel)?;
            install_archive(
                &zip,
                &(p.install_path.clone(), p.texture_install_path.clone()),
                &p.deny_effect_files,
                &p.name,
                Some(&previous),
                m,
                j,
                ops.cancel,
            )?;
            if !m.packages.contains(&p.name) {
                m.packages.push(p.name.clone());
            }
        }
        for source in extras {
            (ops.progress)(Step::Shaders(source_folder(source)));
            if is_url(source) {
                let zip = cache.package(ops.net, &repo_zip_url(source)?, refresh, ops.cancel)?;
                install_archive(&zip, &extra_roots(source), &[], source, Some(&previous), m, j, ops.cancel)?;
            } else {
                copy_folder(Path::new(source), source, Some(&previous), m, j)?;
            }
            if !m.extra_sources.contains(source) {
                m.extra_sources.push(source.clone());
            }
        }
        Ok(())
    })
}

/// A local folder of shaders: its `Shaders`/`Textures` folders when it has
/// them, else the whole folder, under `Shaders/<name>`.
fn copy_folder(folder: &Path, source: &str, previous: Option<&Manifest>, m: &mut Manifest, j: &mut Journal) -> Result<(), String> {
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let (shaders_root, textures_root) = extra_roots(source);
    let has = |n: &str| folder.join(n).is_dir();
    let parts: Vec<(PathBuf, String, Kind)> = match has("Shaders") {
        true => {
            let mut v = vec![(folder.join("Shaders"), shaders_root, Kind::Shader)];
            if has("Textures") {
                v.push((folder.join("Textures"), textures_root, Kind::Texture));
            }
            v
        }
        false => vec![(folder.to_path_buf(), shaders_root, Kind::Shader)],
    };
    let dir = m.dir();
    for (from, root, kind) in parts {
        for file in walk(&from) {
            let rel_in = file.strip_prefix(&from).map_err(|e| e.to_string())?;
            let rel = format!("{root}/{}", rel_in.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
            let path = join(&dir, &rel);
            let current = m.clone();
            if path.exists() && !ours(&rel, &path, &[Some(&current), previous], j) {
                continue;
            }
            let sha = j.copy(&file, &path)?;
            m.record(&rel, sha, kind, Some(source));
        }
    }
    Ok(())
}

/// Files below `dir`, sorted.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => out.push(e.path()),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

pub fn add_packages(ops: &Ops, game_id: &str, packages: &[Package]) -> Result<Manifest, String> {
    let mut m = load(ops.paths, game_id)?;
    add_sources(ops, &mut m, packages, &[], false)?;
    manifest::save(ops.paths, game_id, &m)?;
    Ok(m)
}

/// Adds a folder or a repository address (`https://github.com/<user>/<repo>`
/// or the address of a zip).
pub fn add_extra(ops: &Ops, game_id: &str, source: &str) -> Result<Manifest, String> {
    let source = source.trim().trim_end_matches(['/', '\\']).to_string();
    if is_url(&source) {
        repo_zip_url(&source)?;
    }
    let mut m = load(ops.paths, game_id)?;
    add_sources(ops, &mut m, &[], std::slice::from_ref(&source), false)?;
    manifest::save(ops.paths, game_id, &m)?;
    Ok(m)
}

/// Downloads the packages and repositories again and replaces the files
/// that are still as installed. `catalog`: today's package list.
pub fn update_shaders(ops: &Ops, game_id: &str, catalog: &[Package]) -> Result<Manifest, String> {
    let mut m = load(ops.paths, game_id)?;
    let packages: Vec<Package> = catalog.iter().filter(|p| m.packages.contains(&p.name)).cloned().collect();
    let extras: Vec<String> = m.extra_sources.iter().filter(|s| s.as_str() != ADOPTED).cloned().collect();
    add_sources(ops, &mut m, &packages, &extras, true)?;
    manifest::save(ops.paths, game_id, &m)?;
    Ok(m)
}

/// Files left in place because the user changed them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    pub files: Vec<String>,
}

/// Removes the files only `source` brought, when unchanged. Files another
/// source also brings stay (`ReShade.fxh`), and so do edited ones.
pub fn remove_source(paths: &DataPaths, game_id: &str, source: &str) -> Result<(Manifest, Kept), String> {
    let mut m = load(paths, game_id)?;
    let dir = m.dir();
    let mut kept = Kept::default();
    let mut files = Vec::new();
    for mut f in std::mem::take(&mut m.files) {
        let had = f.source.len();
        f.source.retain(|s| s != source);
        if f.source.is_empty() && had > 0 {
            let path = join(&dir, &f.path);
            match sha256_file(&path) {
                Ok(sha) if sha == f.sha256 => std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?,
                Ok(_) => kept.files.push(f.path.clone()),
                Err(_) => {}
            }
            continue;
        }
        files.push(f);
    }
    m.files = files;
    m.packages.retain(|p| p != source);
    m.extra_sources.retain(|p| p != source);
    remove_empty_dirs(&dir.join("reshade-shaders"));
    manifest::save(paths, game_id, &m)?;
    Ok((m, kept))
}

/// Deletes empty folders below (and including) `dir`.
fn remove_empty_dirs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_dirs(&e.path());
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// Copies another preset in and points `ReShade.ini` to it.
pub fn import_preset(paths: &DataPaths, game_id: &str, preset: &Path) -> Result<Manifest, String> {
    let mut m = load(paths, game_id)?;
    let dir = m.dir();
    journaled(|j| {
        let previous = m.clone();
        let name = copy_preset(preset, &dir, Some(&previous), &mut m, j)?;
        let existing = std::fs::read_to_string(dir.join(CONFIG)).ok();
        let text = config_text(existing.as_deref(), Some(&name), None);
        j.write(&dir.join(CONFIG), text.as_bytes())?;
        m.record(CONFIG, super::fetch::sha256(text.as_bytes()), Kind::Config, None);
        m.preset = Some(name);
        Ok(())
    })?;
    manifest::save(paths, game_id, &m)?;
    Ok(m)
}

/// What a removal did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Removal {
    /// Changed by the user (or not deletable), so left in place.
    pub kept: Vec<String>,
    /// Sent to the trash (settings, the preset copy, the log).
    pub trashed: Vec<String>,
    /// Meant for the trash, which refused them (a drive without one): they
    /// stay in the game's folder.
    pub not_trashed: Vec<String>,
}

/// Removes ReShade from the game. `ReShade.ini`, the preset copy and
/// `ReShade.log` hold the user's settings and go to `discard` (the trash)
/// first: a trash that refuses them leaves them in place and does not stop
/// the removal. Then the DLL goes (if that fails, nothing else changed and
/// ReShade stays installed), then the other files Heronite wrote (when
/// unchanged), and the manifest last, so a half-removed ReShade is never
/// put back before a launch. Files another game's ReShade in the same folder
/// also lists stay.
pub fn remove(paths: &DataPaths, game_id: &str, discard: &dyn Fn(&Path) -> Result<(), String>) -> Result<Removal, String> {
    let m = load(paths, game_id)?;
    let dir = m.dir();
    let shared: Vec<Manifest> = sharing(paths, game_id, &dir).into_iter().map(|(_, m)| m).collect();
    let ours: Vec<_> = m.files.iter().filter(|f| !shared.iter().any(|o| o.file(&f.path).is_some())).collect();
    let mut out = Removal::default();
    let mut settings: Vec<(String, PathBuf)> =
        ours.iter().filter(|f| matches!(f.kind, Kind::Config | Kind::Preset)).map(|f| (f.path.clone(), join(&dir, &f.path))).collect();
    if shared.is_empty() {
        settings.push(("ReShade.log".into(), dir.join("ReShade.log")));
    }
    for (name, path) in settings.into_iter().filter(|(_, p)| p.is_file()) {
        match discard(&path) {
            Ok(()) => out.trashed.push(name),
            Err(e) => {
                log::warn!("ReShade: {name} stays in the game's folder: {e}");
                out.not_trashed.push(name);
            }
        }
    }
    let dll = m.dll_path();
    if let Some(f) = ours.iter().find(|f| f.kind == Kind::Dll) {
        match sha256_file(&dll) {
            Ok(sha) if sha == f.sha256 => std::fs::remove_file(&dll).map_err(|e| format!("{}: {e}", dll.display()))?,
            Ok(_) => out.kept.push(f.path.clone()),
            Err(_) => {}
        }
    }
    for f in ours.iter().filter(|f| !matches!(f.kind, Kind::Dll | Kind::Config | Kind::Preset)) {
        let path = join(&dir, &f.path);
        match sha256_file(&path) {
            Ok(sha) if sha == f.sha256 => {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("ReShade: could not delete {}: {e}", path.display());
                    out.kept.push(f.path.clone());
                }
            }
            Ok(_) => out.kept.push(f.path.clone()),
            Err(_) => {}
        }
    }
    remove_empty_dirs(&dir.join("reshade-shaders"));
    manifest::delete(paths, game_id);
    Ok(out)
}

/// Whether ReShade loads at the next start: its DLL is in place, or the
/// place is free and the cached copy [`recover`] puts back is there.
pub fn will_load(paths: &DataPaths, m: &Manifest) -> bool {
    match occupant(&m.dir(), &m.proxy) {
        Occupant::ReShade { off, .. } => !off,
        Occupant::Free => arch_of(m).ok().and_then(|a| Cache::new(paths).cached_build_dll(&m.version, Build::of(m.addon), a)).is_some(),
        Occupant::Other { .. } => false,
    }
}

/// Other games whose ReShade is in `dir`.
pub fn sharing(paths: &DataPaths, game_id: &str, dir: &Path) -> Vec<(String, Manifest)> {
    let Ok(entries) = std::fs::read_dir(paths.root.join("reshade")) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let id = e.path().file_stem()?.to_string_lossy().into_owned();
            (id != game_id).then_some(id)
        })
        .filter_map(|id| manifest::load(paths, &id).map(|m| (id, m)))
        .filter(|(_, m)| Path::new(&m.target_dir) == dir)
        .collect()
}

fn api_of_proxy(proxy: &str) -> Option<Api> {
    match proxy.to_ascii_lowercase().as_str() {
        "dxgi.dll" | "d3d11.dll" | "d3d10.dll" | "d3d12.dll" => Some(Api::Dxgi),
        "d3d9.dll" => Some(Api::D3d9),
        "opengl32.dll" => Some(Api::OpenGl),
        "ddraw.dll" => Some(Api::DDraw),
        _ => None,
    }
}

/// Takes over a ReShade installed by hand in `dir` under `proxy`, without
/// downloading anything: the files there become Heronite's, so switching,
/// updating and removing work.
pub fn adopt(paths: &DataPaths, game_id: &str, dir: &Path, proxy: &str) -> Result<Manifest, String> {
    let Occupant::ReShade { path, off, version } = occupant(dir, proxy) else {
        return Err(format!("no ReShade DLL named {proxy} in {}", dir.display()));
    };
    let info = crate::diagnose::pe::info(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let arch = Arch::of_machine(info.machine).ok_or("unknown architecture")?;
    let mut m = Manifest {
        version: version.unwrap_or_default(),
        arch: arch.key().into(),
        api: api_of_proxy(proxy).map(|a| a.key()).unwrap_or_default().into(),
        proxy: proxy.to_string(),
        target_dir: dir.to_string_lossy().into_owned(),
        adopted: true,
        addon: false,
        compiler: None,
        enabled: !off,
        packages: Vec::new(),
        extra_sources: Vec::new(),
        preset: None,
        files: Vec::new(),
        installed_at: now(),
    };
    m.record(proxy, sha256_file(&path)?, Kind::Dll, None);
    let compiler = dir.join(COMPILER);
    if let Ok(sha) = sha256_file(&compiler) {
        if is_known_compiler(&sha) {
            m.record(COMPILER, sha, Kind::Compiler, None);
            m.compiler = Some("fxc2".into());
        }
    }
    let config = dir.join(CONFIG);
    if let Ok(text) = std::fs::read_to_string(&config) {
        m.record(CONFIG, super::fetch::sha256(text.as_bytes()), Kind::Config, None);
        let preset = Ini::parse(&text).get("GENERAL", "PresetPath").map(str::to_string).unwrap_or_default();
        let preset_path = join(dir, &preset);
        let in_dir = preset_path.parent() == Some(dir) && preset_path.is_file();
        if let (true, Some(name)) = (in_dir, preset_path.file_name().map(|n| n.to_string_lossy().into_owned())) {
            m.record(&name, sha256_file(&preset_path)?, Kind::Preset, None);
            m.preset = Some(name);
        }
    }
    let shaders = dir.join("reshade-shaders");
    for file in walk(&shaders) {
        let rel_in = file.strip_prefix(dir).map_err(|e| e.to_string())?;
        let rel = rel_in.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        let kind = if rel.to_ascii_lowercase().starts_with("reshade-shaders/textures/") { Kind::Texture } else { Kind::Shader };
        m.record(&rel, sha256_file(&file)?, kind, Some(ADOPTED));
    }
    if m.files.iter().any(|f| f.source.iter().any(|s| s == ADOPTED)) {
        m.extra_sources.push(ADOPTED.into());
    }
    manifest::save(paths, game_id, &m)?;
    Ok(m)
}

/// What [`recover`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// The DLL was put back from the cache.
    Restored { version: String },
    /// Another program's DLL took its place: ReShade is switched off.
    Blocked { path: PathBuf },
    /// Gone and not cached (or not copyable): the game starts without it.
    Lost { version: String, error: Option<String> },
}

/// Before a launch: puts the DLL back from the cache when it went missing
/// (a store update or file check removes unknown files). Another program's
/// DLL in its place switches ReShade off instead.
pub fn recover(paths: &DataPaths, game_id: &str) -> Option<Recovery> {
    let mut m = manifest::load(paths, game_id)?;
    if !m.enabled || !m.dir().is_dir() {
        return None;
    }
    match occupant(&m.dir(), &m.proxy) {
        Occupant::ReShade { off: false, .. } => None,
        Occupant::ReShade { off: true, .. } => {
            m.enabled = false;
            manifest::save(paths, game_id, &m).ok()?;
            None
        }
        Occupant::Other { path, .. } => {
            m.enabled = false;
            manifest::save(paths, game_id, &m).ok()?;
            Some(Recovery::Blocked { path })
        }
        Occupant::Free => {
            let version = m.version.clone();
            let Some(dll) = arch_of(&m).ok().and_then(|a| Cache::new(paths).cached_build_dll(&m.version, Build::of(m.addon), a)) else {
                return Some(Recovery::Lost { version, error: None });
            };
            let dest = m.dir().join(&m.proxy);
            if let Err(e) = std::fs::copy(&dll, &dest) {
                return Some(Recovery::Lost { version, error: Some(e.to_string()) });
            }
            if let Ok(sha) = sha256_file(&dest) {
                m.record(&m.proxy.clone(), sha, Kind::Dll, None);
                let _ = manifest::save(paths, game_id, &m);
            }
            Some(Recovery::Restored { version })
        }
    }
}

#[cfg(test)]
#[path = "manage_tests.rs"]
mod tests;
