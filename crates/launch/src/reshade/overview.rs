//! Everything the game settings show about ReShade, read in one blocking
//! call off the UI thread: the executable and its API, what is installed,
//! what is in the way, the anti-cheat risk, the catalog and the versions.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;

use super::anticheat::{self, Risk};
use super::catalog::{self, Compat, Package};
use super::detect::{self, Api, Occupant, Target};
use super::fetch::Cache;
use super::manifest::{self, Manifest, ADOPTED};
use super::preset::{self, Needs};
use super::ReShade;
use crate::gfxmods::catalog::{self as mods, Found};
use crate::gfxmods::renodx;
use crate::net::Net;

/// The game, as the settings know it.
pub struct Facts<'a> {
    pub game_id: &'a str,
    pub title: &'a str,
    pub steam_app_id: Option<&'a str>,
    pub install_path: Option<&'a str>,
    pub executable: Option<&'a str>,
    /// `targetExe` of the game's settings.
    pub target_exe: &'a str,
    pub settings: &'a ReShade,
    pub env_options: &'a [(String, String)],
    /// Run by Wine or Proton (`false`: Windows runs it).
    pub wine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// No Windows executable to install into.
    NoExecutable,
    /// The executable imports no graphics API (a launcher): pick another.
    Launcher,
    /// Vulkan or Direct3D 8.
    Unsupported(Api),
    /// `Banned=1`: the game bans or blocks ReShade.
    Banned,
    /// Another program's DLL has the name ReShade needs.
    Conflict {
        path: PathBuf,
        product: Option<String>,
    },
    /// A ReShade Heronite did not install.
    Manual {
        proxy: String,
        version: Option<String>,
        off: bool,
    },
    NotInstalled,
    Installed,
}

/// A package or extra source in the installed list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    /// Package name, folder or address ([`ADOPTED`] for adopted files).
    pub name: String,
    pub effects: usize,
    pub bytes: u64,
    /// Effects of it the current preset uses.
    pub used: Vec<String>,
    /// The required package: never removed.
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct Overview {
    pub state: State,
    pub exe: Option<PathBuf>,
    /// The game's executable when it is a launcher and `exe` was picked
    /// next to it.
    pub launcher: Option<PathBuf>,
    pub target: Option<Target>,
    /// What `Compatibility.ini` says about the executable.
    pub compat: Option<Compat>,
    /// Executables that import a graphics API, for "Change…".
    pub candidates: Vec<PathBuf>,
    /// The DLL name ReShade takes (or has, once installed).
    pub proxy: Option<String>,
    pub manifest: Option<Manifest>,
    pub risk: Risk,
    pub packages: Vec<Package>,
    pub latest: Option<String>,
    /// No network: lists and versions come from the cache.
    pub offline: bool,
    /// The installed preset's effects: what they need, what is missing.
    pub preset_needs: Option<Needs>,
    pub sources: Vec<SourceRow>,
    /// Other games whose ReShade is in the same folder.
    pub sharing: Vec<String>,
    /// DLL names the user's `WINEDLLOVERRIDES` already sets.
    pub user_overrides: Vec<String>,
    /// The RenoDX mod Heronite's catalog has for the game.
    pub renodx: Option<Found>,
    /// The RenoDX mod installed, when there is one.
    pub renodx_installed: Option<renodx::Installed>,
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What the files on disk say about the game and ReShade.
struct Inspection {
    manifest: Option<Manifest>,
    exe: Option<PathBuf>,
    launcher: Option<PathBuf>,
    target: Option<Target>,
    compat: Option<Compat>,
    candidates: Vec<PathBuf>,
    risk: Risk,
    proxy: Option<String>,
    state: State,
}

/// Reads the game's programs and folders (no network). `compat_text`:
/// ReShade's `Compatibility.ini`, when there is one.
fn inspect(paths: &DataPaths, f: &Facts, compat_text: Option<&str>) -> Inspection {
    let manifest = manifest::load(paths, f.game_id);
    let game_exe = detect::game_exe(f.install_path, f.executable, f.target_exe);
    let chosen = Some(PathBuf::from(&f.settings.target)).filter(|p| !f.settings.target.is_empty() && p.is_file());
    let picked = chosen.is_some();
    let mut exe = chosen.or(game_exe);
    let compat = |e: &Path| -> Option<Compat> { compat_text.and_then(|t| catalog::compat(t, &file_name(e))) };
    let mut candidates = Vec::new();
    // No executable known (a Steam game): the biggest program of its folder
    // that draws.
    if exe.is_none() {
        if let Some(install) = f.install_path.map(Path::new).filter(|p| p.is_dir()) {
            candidates = detect::candidates(install);
            exe = candidates.first().cloned();
        }
    }
    if let Some(dir) = exe.as_deref().and_then(Path::parent).filter(|_| candidates.is_empty()) {
        candidates = detect::candidates(dir);
        if let Some(install) = f.install_path.map(Path::new).filter(|p| *p != dir) {
            for c in detect::candidates(install) {
                if !candidates.contains(&c) {
                    candidates.push(c);
                }
            }
        }
    }
    let mut compat_of_exe = exe.as_deref().and_then(compat);
    let mut target = exe.as_deref().and_then(|e| detect::target(e, compat_of_exe.as_ref()).ok());
    // A launcher draws nothing: the biggest program next to it that imports
    // a graphics API is the game (the user can pick another).
    let mut launcher = None;
    if !picked && target.as_ref().is_some_and(|t| t.api.is_none()) {
        if let Some(c) = candidates.first() {
            launcher = exe.replace(c.clone());
            compat_of_exe = compat(c);
            target = detect::target(c, compat_of_exe.as_ref()).ok();
        }
    }
    let banned = compat_of_exe.as_ref().is_some_and(|c| c.banned);
    let listed = heronite_data::anticheat::lookup(paths, f.steam_app_id, f.title);
    let mut dirs: Vec<&Path> = Vec::new();
    if let Some(t) = &target {
        dirs.push(&t.dir);
    }
    if let Some(p) = f.install_path {
        dirs.push(Path::new(p));
    }
    let risk = anticheat::assess(banned, listed.as_ref(), &dirs);
    let api = match f.settings.api.as_str() {
        "auto" | "" => target.as_ref().and_then(|t| t.api),
        "d3d11" => Some(Api::Dxgi),
        k => Api::from_key(k),
    };
    let proxy = match &manifest {
        Some(m) => Some(m.proxy.clone()),
        None => f.settings.proxy_choice().or_else(|| api.and_then(Api::proxy).map(str::to_string)),
    };
    let state = match (&manifest, &target) {
        (Some(_), _) => State::Installed,
        (None, None) => State::NoExecutable,
        (None, Some(t)) => match (banned, detect::find_reshade(&t.dir), api) {
            (true, _, _) => State::Banned,
            (_, Some((proxy, Occupant::ReShade { version, off, .. })), _) => State::Manual { proxy, version, off },
            (_, _, None) => State::Launcher,
            (_, _, Some(a @ (Api::Vulkan | Api::D3d8))) => State::Unsupported(a),
            _ => match proxy.as_deref().map(|p| detect::occupant(&t.dir, p)) {
                Some(Occupant::Other { path, product }) => State::Conflict { path, product },
                _ => State::NotInstalled,
            },
        },
    };
    Inspection { manifest, exe, launcher, target, compat: compat_of_exe, candidates, risk, proxy, state }
}

/// Whether ReShade fits the game with no known problem, from what is on
/// disk only (fast enough for a menu, off the UI thread): installed by
/// Heronite, or ready to install or adopt with no anti-cheat known, nothing
/// banning it and nothing in its place.
pub fn fits(paths: &DataPaths, f: &Facts) -> bool {
    if !f.wine && !cfg!(windows) {
        return false;
    }
    let compat_text = Cache::new(paths).cached_catalog(false);
    let i = inspect(paths, f, compat_text.as_deref());
    match i.state {
        State::Installed => true,
        State::NotInstalled | State::Manual { .. } => !i.risk.banned && i.risk.findings.is_empty(),
        _ => false,
    }
}

/// `refresh`: ask for the lists and the newest version even within a day.
pub fn overview(paths: &DataPaths, net: &dyn Net, f: &Facts, refresh: bool) -> Overview {
    let cache = Cache::new(paths);
    let offline = !net.is_online();
    let packages = cache.catalog(net, true, refresh).map(|t| catalog::packages(&t)).unwrap_or_default();
    let compat_text = cache.catalog(net, false, refresh);
    let latest = cache.latest_version(net, refresh);
    let Inspection { manifest, exe, launcher, target, compat, candidates, risk, proxy, state } = inspect(paths, f, compat_text.as_deref());
    let preset_needs = manifest.as_ref().and_then(|m| {
        let text = std::fs::read_to_string(m.dir().join(m.preset.as_deref()?)).ok()?;
        let effects = preset::active_effects(&text);
        Some(preset::needs(&effects, &packages, |e| m.has_effect(e)))
    });
    let active: Vec<String> = manifest
        .as_ref()
        .and_then(|m| std::fs::read_to_string(m.dir().join(m.preset.as_deref()?)).ok())
        .map(|t| preset::active_effects(&t))
        .unwrap_or_default();
    let sources = manifest.as_ref().map(|m| source_rows(m, &packages, &active)).unwrap_or_default();
    let sharing = manifest
        .as_ref()
        .map(|m| super::manage::sharing(paths, f.game_id, &m.dir()).into_iter().map(|(id, _)| id).collect())
        .unwrap_or_default();
    // Only what the launch adds: a game's own d3dcompiler_47 keeps the
    // user's override.
    let compiler = manifest.as_ref().is_some_and(|m| m.compiler.is_some()).then_some(super::install::COMPILER);
    let dlls: Vec<&str> = proxy.iter().map(String::as_str).chain(compiler).collect();
    let user_overrides = match f.wine {
        true => super::user_overrides(f.env_options, &dlls),
        false => Vec::new(),
    };
    let renodx = mods::load(paths, net, refresh).renodx_for(f.steam_app_id, exe.as_deref(), f.title);
    let renodx_installed = manifest.as_ref().and_then(renodx::installed);
    Overview {
        state,
        exe,
        launcher,
        target,
        compat,
        candidates,
        proxy,
        manifest,
        risk,
        packages,
        latest,
        offline,
        preset_needs,
        sources,
        sharing,
        user_overrides,
        renodx,
        renodx_installed,
    }
}

fn source_rows(m: &Manifest, packages: &[Package], active: &[String]) -> Vec<SourceRow> {
    let dir = m.dir();
    m.sources()
        .into_iter()
        .map(|name| {
            let files: Vec<_> = m.files.iter().filter(|f| f.source.contains(&name)).collect();
            let effects = m.effects_of(&name);
            SourceRow {
                bytes: files.iter().filter_map(|f| std::fs::metadata(manifest::join(&dir, &f.path)).ok()).map(|md| md.len()).sum(),
                used: active.iter().filter(|a| effects.iter().any(|e| e.eq_ignore_ascii_case(a))).cloned().collect(),
                effects: effects.len(),
                required: name != ADOPTED && packages.iter().any(|p| p.name == name && p.required),
                name,
            }
        })
        .collect()
}
