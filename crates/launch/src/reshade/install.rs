//! Installing ReShade into a game's folder, like its installer: the DLL
//! under the proxy's name, `d3dcompiler_47` under Wine, the shader
//! packages, the preset and `ReShade.ini`. Every write is journaled: when a
//! step fails or is cancelled, what this operation wrote is undone.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;

use super::archive::{self, Part};
use super::catalog::{Compat, Package};
use super::detect::{occupant, Arch, Occupant, Target};
use super::fetch::{is_known_compiler, sha256, sha256_file, Cache};
use super::ini::Ini;
use super::manifest::{self, join, Kind, Manifest};
pub(crate) use crate::gfxmods::journal::Journal;
use crate::net::{Cancel, Net};

/// What the long operations need besides their arguments.
pub struct Ops<'a> {
    pub paths: &'a DataPaths,
    pub net: &'a dyn Net,
    pub cancel: Cancel<'a>,
    pub progress: &'a dyn Fn(Step),
}

/// What an operation is doing, for the progress line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Dll(String),
    Compiler,
    /// Name of a package or extra source.
    Shaders(String),
    /// Name of a ReShade add-on (a RenoDX mod).
    Addon(String),
    Config,
}

pub const CONFIG: &str = "ReShade.ini";
pub const COMPILER: &str = "d3dcompiler_47.dll";

#[derive(Debug, Clone)]
pub struct Request {
    pub game_id: String,
    pub target: Target,
    pub proxy: String,
    pub version: String,
    /// The packages to install (the required one included).
    pub packages: Vec<Package>,
    pub preset: Option<PathBuf>,
    /// Add `d3dcompiler_47` (Wine/Proton with Direct3D 9 to 12).
    pub compiler: bool,
    pub compat: Option<Compat>,
}

/// A folder the game can be written to (a test file, removed at once).
fn check_writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(".heronite-write-test");
    std::fs::write(&probe, b"").map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

pub(super) fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Installs ReShade for `req`. `previous`: the game's manifest, when there
/// is one (a reinstall, or replacing an adopted install): its files may be
/// overwritten.
pub fn install(ops: &Ops, req: &Request, previous: Option<&Manifest>) -> Result<Manifest, String> {
    let dir = &req.target.dir;
    check_writable(dir)?;
    let owned_dll = previous.is_some_and(|p| p.proxy.eq_ignore_ascii_case(&req.proxy) && Path::new(&p.target_dir) == dir);
    let old_off = match occupant(dir, &req.proxy) {
        Occupant::Other { path, product } => {
            return Err(format!(
                "{} belongs to {}; Heronite will not replace it",
                path.display(),
                product.as_deref().unwrap_or("another program")
            ))
        }
        Occupant::ReShade { .. } if !owned_dll => return Err(format!("ReShade is already in {} (installed by hand)", dir.display())),
        Occupant::ReShade { path, off: true, .. } => Some(path),
        _ => None,
    };
    let mut journal = Journal::default();
    let res = write_install(ops, req, previous, old_off.as_deref(), &mut journal);
    match res {
        Ok(m) => Ok(m),
        Err(e) => {
            journal.rollback();
            Err(e)
        }
    }
}

fn write_install(
    ops: &Ops,
    req: &Request,
    previous: Option<&Manifest>,
    old_off: Option<&Path>,
    j: &mut Journal,
) -> Result<Manifest, String> {
    let dir = &req.target.dir;
    let cache = Cache::new(ops.paths);
    let mut m = Manifest {
        version: req.version.clone(),
        arch: req.target.arch.key().into(),
        api: req.target.api.map(|a| a.key()).unwrap_or_default().into(),
        proxy: req.proxy.clone(),
        target_dir: dir.to_string_lossy().into_owned(),
        adopted: false,
        addon: false,
        compiler: None,
        enabled: true,
        packages: Vec::new(),
        extra_sources: Vec::new(),
        preset: None,
        files: Vec::new(),
        installed_at: now(),
    };
    (ops.progress)(Step::Dll(req.version.clone()));
    let dll = cache.dll(ops.net, &req.version, req.target.arch, ops.cancel)?;
    if let Some(off) = old_off {
        j.remove(off)?;
    }
    let sha = j.copy(&dll, &dir.join(&req.proxy))?;
    m.record(&req.proxy, sha, Kind::Dll, None);
    if req.compiler {
        (ops.progress)(Step::Compiler);
        m.compiler = add_compiler(ops, &cache, req.target.arch, dir, j, &mut m)?;
    }
    for p in &req.packages {
        if (ops.cancel)() {
            return Err(crate::net::CANCELLED.into());
        }
        (ops.progress)(Step::Shaders(p.name.clone()));
        let zip = cache.package(ops.net, &p.download_url, false, ops.cancel)?;
        let roots = (p.install_path.clone(), p.texture_install_path.clone());
        install_archive(&zip, &roots, &p.deny_effect_files, &p.name, previous, &mut m, j, ops.cancel)?;
        m.packages.push(p.name.clone());
    }
    (ops.progress)(Step::Config);
    if let Some(preset) = &req.preset {
        m.preset = Some(copy_preset(preset, dir, previous, &mut m, j)?);
    }
    let existing = std::fs::read_to_string(dir.join(CONFIG)).ok();
    let text = config_text(existing.as_deref(), m.preset.as_deref(), req.compat.as_ref());
    j.write(&dir.join(CONFIG), text.as_bytes())?;
    m.record(CONFIG, sha256(text.as_bytes()), Kind::Config, None);
    manifest::save(ops.paths, &req.game_id, &m)?;
    Ok(m)
}

/// Puts the checked `d3dcompiler_47.dll` next to the game. A different one
/// already there (the game's own) is left alone. `Some("fxc2")`: Heronite's
/// copy is there.
fn add_compiler(ops: &Ops, cache: &Cache, arch: Arch, dir: &Path, j: &mut Journal, m: &mut Manifest) -> Result<Option<String>, String> {
    let dest = dir.join(COMPILER);
    if dest.is_file() {
        let sha = sha256_file(&dest)?;
        if !is_known_compiler(&sha) {
            log::info!("{} is the game's own; ReShade uses it", dest.display());
            return Ok(None);
        }
        m.record(COMPILER, sha, Kind::Compiler, None);
        return Ok(Some("fxc2".into()));
    }
    let src = cache.compiler(ops.net, arch, ops.cancel)?;
    let sha = j.copy(&src, &dest)?;
    m.record(COMPILER, sha, Kind::Compiler, None);
    Ok(Some("fxc2".into()))
}

/// Whether a file may be replaced: written by this operation, or recorded
/// in a manifest and unchanged since.
pub(super) fn ours(rel: &str, path: &Path, manifests: &[Option<&Manifest>], j: &Journal) -> bool {
    if j.touched(path) {
        return true;
    }
    let Some(sha) = manifests.iter().flatten().find_map(|m| m.file(rel)).map(|f| f.sha256.clone()) else { return false };
    sha256_file(path).is_ok_and(|now| now == sha)
}

/// Copies a package's files to `roots` (`InstallPath`, `TextureInstallPath`
/// relative to the manifest's folder). A different file already there that
/// Heronite did not write is kept.
#[allow(clippy::too_many_arguments)]
pub(super) fn install_archive(
    zip: &Path,
    roots: &(String, String),
    deny: &[String],
    source: &str,
    previous: Option<&Manifest>,
    m: &mut Manifest,
    j: &mut Journal,
    cancel: Cancel,
) -> Result<(), String> {
    let dir = m.dir();
    let base = |root: &str| root.split(['\\', '/']).filter(|p| !p.is_empty() && *p != "." && *p != "..").collect::<Vec<_>>().join("/");
    let (shaders, textures) = (base(&roots.0), base(&roots.1));
    let mut entries: Vec<(String, Vec<u8>, Kind)> = Vec::new();
    archive::package_files(zip, deny, cancel, |part, rel, reader| {
        let (root, kind) = match part {
            Part::Shaders => (&shaders, Kind::Shader),
            Part::Textures => (&textures, Kind::Texture),
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        entries.push((format!("{root}/{}", rel.join("/")), bytes, kind));
        Ok(())
    })?;
    for (rel, bytes, kind) in entries {
        let path = join(&dir, &rel);
        let sha = sha256(&bytes);
        let current = m.clone();
        if path.exists() && !ours(&rel, &path, &[Some(&current), previous], j) {
            if sha256_file(&path).is_ok_and(|s| s == sha) {
                m.record(&rel, sha, kind, Some(source));
            } else {
                log::info!("ReShade: kept {} (not Heronite's)", path.display());
            }
            continue;
        }
        j.write(&path, &bytes)?;
        m.record(&rel, sha, kind, Some(source));
    }
    Ok(())
}

/// Copies the preset into the game's folder (the original is never
/// changed: ReShade saves into its copy). A preset already in that folder
/// is used where it is. Gives the copy's file name.
pub(super) fn copy_preset(
    preset: &Path,
    dir: &Path,
    previous: Option<&Manifest>,
    m: &mut Manifest,
    j: &mut Journal,
) -> Result<String, String> {
    let name = preset.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| format!("{} is not a file", preset.display()))?;
    let bytes = std::fs::read(preset).map_err(|e| format!("{}: {e}", preset.display()))?;
    let in_place = preset.parent().and_then(|p| p.canonicalize().ok()) == dir.canonicalize().ok();
    if in_place {
        return Ok(name);
    }
    // A different preset of the same name that is not Heronite's copy stays.
    let (stem, ext) = name.rsplit_once('.').map_or((name.as_str(), "ini"), |(s, e)| (s, e));
    let mut chosen = name.clone();
    for n in 2.. {
        let path = dir.join(&chosen);
        let current = m.clone();
        let free = !path.exists() || ours(&chosen, &path, &[Some(&current), previous], j) || std::fs::read(&path).is_ok_and(|b| b == bytes);
        if free {
            break;
        }
        chosen = format!("{stem} ({n}).{ext}");
    }
    j.write(&dir.join(&chosen), &bytes)?;
    m.record(&chosen, sha256(&bytes), Kind::Preset, None);
    Ok(chosen)
}

/// `ReShade.ini` with what Heronite needs added (and the preset set when
/// one was imported). Everything else in an existing file stays.
pub fn config_text(existing: Option<&str>, preset: Option<&str>, compat: Option<&Compat>) -> String {
    let mut ini = Ini::parse(existing.unwrap_or(""));
    ini.set_missing("GENERAL", "EffectSearchPaths", ".\\reshade-shaders\\Shaders\\**");
    ini.set_missing("GENERAL", "TextureSearchPaths", ".\\reshade-shaders\\Textures\\**");
    match preset {
        Some(p) => {
            ini.set("GENERAL", "PresetPath", &format!(".\\{p}"));
            // Only the preset's effects compile: the game opens faster.
            ini.set_missing("GENERAL", "SkipLoadingDisabledEffects", "1");
        }
        None => {
            ini.set_missing("GENERAL", "PresetPath", ".\\ReShadePreset.ini");
            ini.set_missing("GENERAL", "SkipLoadingDisabledEffects", "0");
        }
    }
    if let Some(c) = compat {
        if c.depth_reversed.is_some() || c.depth_upside_down.is_some() {
            let defs = format!(
                "RESHADE_DEPTH_LINEARIZATION_FAR_PLANE=1000.0,RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN={},RESHADE_DEPTH_INPUT_IS_REVERSED={},RESHADE_DEPTH_INPUT_IS_LOGARITHMIC=0",
                c.depth_upside_down.as_deref().unwrap_or("0"),
                c.depth_reversed.as_deref().unwrap_or("1"),
            );
            if ini.get("GENERAL", "PreprocessorDefinitions").is_none_or(str::is_empty) {
                ini.set("GENERAL", "PreprocessorDefinitions", &defs);
            }
        }
        for (key, value) in [
            ("DepthCopyBeforeClears", &c.depth_copy_before_clears),
            ("DepthCopyAtClearIndex", &c.depth_copy_at_clear_index),
            ("UseAspectRatioHeuristics", &c.use_aspect_ratio_heuristics),
        ] {
            if let Some(v) = value {
                ini.set_missing("DEPTH", key, v);
            }
        }
    }
    // Home opens ReShade's menu (its installer's default).
    ini.set_missing("INPUT", "KeyOverlay", "36,0,0,0");
    ini.to_text()
}

#[cfg(test)]
#[path = "install_tests.rs"]
pub(crate) mod tests;
