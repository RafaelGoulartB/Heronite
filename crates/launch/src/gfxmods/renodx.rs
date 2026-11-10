//! RenoDX (<https://github.com/clshortfuse/renodx>, MIT): HDR mods that
//! rewrite a game's shaders. Each is a ReShade add-on, one file
//! (`renodx-<game>.addon64`, or `.addon32` for 32-bit games) placed beside
//! ReShade's DLL; ReShade must be the build with add-on support. The mod's
//! settings are in ReShade's menu.
//!
//! The file is recorded in the game's ReShade manifest (kind `addon`,
//! source `RenoDX: <id>`), so removing ReShade removes it too, and an
//! edited or foreign file of the same name is never touched. One RenoDX mod
//! per game: installing another replaces it.
//!
//! Downloads come only from GitHub (release assets and `*.github.io`
//! pages), where RenoDX's snapshot builds are published, and must be a DLL
//! of the game's architecture.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;

use super::anticheat::{Policy, Risk};
use super::catalog::Found;
use super::download::download;
use super::hash::{sha256, sha256_file};
use super::journal::journaled;
use crate::diagnose::pe;
use crate::reshade::detect::Arch;
use crate::reshade::install::{Ops, Step};
use crate::reshade::manifest::{self, Kind, Manifest};

const SOURCE: &str = "RenoDX: ";

/// The manifest source of a RenoDX mod.
pub fn source(id: &str) -> String {
    format!("{SOURCE}{id}")
}

/// The RenoDX mod installed for a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The catalog id it was installed from.
    pub id: String,
    /// The add-on's file, relative to ReShade's folder.
    pub file: String,
}

pub fn installed(m: &Manifest) -> Option<Installed> {
    m.files.iter().filter(|f| f.kind == Kind::Addon).find_map(|f| {
        let id = f.source.iter().find_map(|s| s.strip_prefix(SOURCE))?;
        Some(Installed { id: id.to_string(), file: f.path.clone() })
    })
}

/// Whether RenoDX files may come from `url`: HTTPS on github.com or a
/// `*.github.io` page. The host ends at the first `/`, `?` or `#`, and one
/// with credentials, a port or a backslash is refused, so the check sees the
/// host the download connects to (`https://evil.com#@x.github.io/` is not
/// GitHub).
pub fn allowed_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains(['@', '\\', ':']) || host.chars().any(char::is_whitespace) {
        return false;
    }
    let host = host.to_ascii_lowercase();
    host == "github.com" || (host.ends_with(".github.io") && host.len() > ".github.io".len())
}

/// The add-on's address for `arch` (`None`: no build for it).
pub fn url_for(found: &Found, arch: Arch) -> Option<&str> {
    match arch {
        Arch::X64 => found.mod_.addon64.as_deref(),
        Arch::X86 => found.mod_.addon32.as_deref(),
    }
}

/// Why RenoDX cannot go into this install (`None`: it can).
pub fn refusal(m: &Manifest, found: &Found, risk: &Risk) -> Option<String> {
    if risk.blocks(Policy::Block) {
        return Some("RenoDX is not offered for games with anti-cheat".into());
    }
    if !m.addon {
        return Some("RenoDX needs ReShade with add-on support".into());
    }
    let arch = if m.arch == "x86" { Arch::X86 } else { Arch::X64 };
    match url_for(found, arch) {
        None => Some(format!("this RenoDX mod has no {}-bit build", arch.bits())),
        Some(u) if !allowed_url(u) => Some(format!("RenoDX downloads only from GitHub: {u}")),
        Some(_) => None,
    }
}

fn cache_file(paths: &DataPaths, url: &str) -> PathBuf {
    let ext = if url.ends_with(".addon32") { "addon32" } else { "addon64" };
    paths.tools_dir().join("gfxmods").join("renodx").join(format!("{}.{ext}", sha256(url.as_bytes())))
}

/// The add-on of `url`, downloaded once (`refresh`: again, for updates).
fn fetch(ops: &Ops, url: &str, arch: Arch, refresh: bool) -> Result<PathBuf, String> {
    let dest = cache_file(ops.paths, url);
    if !dest.is_file() || refresh {
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        download(ops.net, url, &dest, ops.cancel)?;
    }
    let info = pe::info(&dest).map_err(|e| format!("{}: {e}", dest.display()));
    match info.map(|i| Arch::of_machine(i.machine)) {
        Ok(Some(a)) if a == arch => Ok(dest),
        res => {
            let _ = std::fs::remove_file(&dest);
            Err(match res {
                Err(e) => format!("the RenoDX download is not a DLL ({e})"),
                _ => format!("the RenoDX download is not a {}-bit add-on", arch.bits()),
            })
        }
    }
}

fn file_name(url: &str) -> Result<String, String> {
    let name = url.rsplit('/').next().unwrap_or("");
    let ok = !name.is_empty()
        && (name.ends_with(".addon64") || name.ends_with(".addon32"))
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    ok.then(|| name.to_string()).ok_or_else(|| format!("not a ReShade add-on: {url}"))
}

/// Installs `found` beside the game's ReShade (`refresh`: download it again,
/// to update). Another RenoDX mod Heronite installed is replaced.
pub fn install(ops: &Ops, game_id: &str, found: &Found, risk: &Risk, refresh: bool) -> Result<Manifest, String> {
    let mut m = manifest::load(ops.paths, game_id).ok_or("ReShade is not installed for this game")?;
    if let Some(why) = refusal(&m, found, risk) {
        return Err(why);
    }
    let arch = if m.arch == "x86" { Arch::X86 } else { Arch::X64 };
    let url = url_for(found, arch).unwrap_or_default().to_string();
    let name = file_name(&url)?;
    let ext = match arch {
        Arch::X64 => ".addon64",
        Arch::X86 => ".addon32",
    };
    if !name.ends_with(ext) {
        return Err(format!("{name} is not a {}-bit add-on", arch.bits()));
    }
    (ops.progress)(Step::Addon(found.name.clone()));
    let src = fetch(ops, &url, arch, refresh)?;
    let dir = m.dir();
    let dest = dir.join(&name);
    let previous = installed(&m);
    let ours = |rel: &str, path: &Path| m.file(rel).is_some_and(|f| sha256_file(path).is_ok_and(|s| s == f.sha256));
    if dest.exists() && !ours(&name, &dest) {
        return Err(format!("{} is already there (not installed by Heronite)", dest.display()));
    }
    let old = previous.as_ref().filter(|p| p.file != name).map(|p| dir.join(&p.file)).filter(|p| p.exists());
    if let Some(old) = &old {
        if !ours(&previous.as_ref().unwrap().file, old) {
            return Err(format!("{} was changed since Heronite installed it", old.display()));
        }
    }
    let sha = journaled(|j| {
        if let Some(old) = &old {
            j.remove(old)?;
        }
        j.copy(&src, &dest)
    })?;
    if let Some(p) = previous {
        m.files.retain(|f| f.path != p.file);
    }
    m.record(&name, sha, Kind::Addon, Some(&source(&found.id)));
    manifest::save(ops.paths, game_id, &m)?;
    Ok(m)
}

/// Removes the game's RenoDX mod (an edited file stays, and is named).
pub fn remove(paths: &DataPaths, game_id: &str) -> Result<(Manifest, Vec<String>), String> {
    let m = manifest::load(paths, game_id).ok_or("ReShade is not installed for this game")?;
    let Some(i) = installed(&m) else { return Ok((m, Vec::new())) };
    let (m, kept) = crate::reshade::manage::remove_source(paths, game_id, &source(&i.id))?;
    Ok((m, kept.files))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnose::pe::tests::pe_with;
    use crate::diagnose::pe::{MACHINE_X64, MACHINE_X86};
    use crate::gfxmods::anticheat::{Finding, Source};
    use crate::gfxmods::catalog;
    use crate::reshade::install::install as install_reshade;
    use crate::reshade::install::tests::{game, net, ops, request, Game};

    const UE: &str = "https://marat569.github.io/renodx/renodx-ue-extended.addon64";
    const STALKER: &str = "https://clshortfuse.github.io/renodx/renodx-stalker2.addon64";

    fn found(id: &str) -> Found {
        let c = catalog::parse(catalog::tests::SAMPLE).unwrap();
        match id {
            "generic-unreal" => {
                let dir = tempfile::tempdir().unwrap();
                let exe = dir.path().join("G-Win64-Shipping.exe");
                c.renodx_for(None, Some(&exe), "x").unwrap()
            }
            _ => c
                .games
                .iter()
                .find(|e| e.id == id)
                .map(|e| Found { id: e.id.clone(), name: e.id.clone(), mod_: e.renodx.clone().unwrap(), how: catalog::How::Title })
                .unwrap(),
        }
    }

    fn setup(addon: bool) -> (Game, crate::net::FakeNet) {
        setup_with(addon, pe_with(MACHINE_X64, &[], Some("RenoDX"), 0))
    }

    fn setup_with(addon: bool, ue: Vec<u8>) -> (Game, crate::net::FakeNet) {
        let g = game();
        let net = net().with_file(UE, ue).with_file(STALKER, pe_with(MACHINE_X64, &[], Some("RenoDX"), 0));
        let net = net.with_file(&crate::reshade::fetch::addon_setup_url("6.8.0"), crate::reshade::fetch::tests::fake_setup());
        install_reshade(&ops(&g.paths, &net), &request(&g, None), None).unwrap();
        if addon {
            crate::reshade::manage::set_addon(&ops(&g.paths, &net), "sp", true, &Risk::default()).unwrap();
        }
        (g, net)
    }

    #[test]
    fn only_github_hosts() {
        assert!(allowed_url(UE) && allowed_url("https://github.com/a/renodx/releases/download/snapshot/renodx-x.addon64"));
        assert!(!allowed_url("http://github.com/a.addon64"));
        assert!(!allowed_url("https://www.nexusmods.com/x/mods/1"));
        assert!(!allowed_url("https://github.io/x.addon64"));
        assert!(!allowed_url("https://evil.com/github.io/x.addon64"));
        assert!(!allowed_url("https://evil.com#@x.github.io/f.addon64"));
        assert!(!allowed_url("https://evil.com?@x.github.io/f.addon64"));
        assert!(!allowed_url("https://x.github.io@evil.com/f.addon64"));
        assert!(!allowed_url("https://evil.com\\@x.github.io/f.addon64"));
        assert!(!allowed_url("https://x.github.io:8443/f.addon64"));
        assert!(file_name("https://x.github.io/../renodx-a.addon64").is_ok());
        assert!(file_name("https://x.github.io/renodx-a.dll").is_err());
    }

    #[test]
    fn needs_the_addon_build_and_no_anti_cheat() {
        let (g, net) = setup(false);
        let m = manifest::load(&g.paths, "sp").unwrap();
        let f = found("generic-unreal");
        assert_eq!(refusal(&m, &f, &Risk::default()).as_deref(), Some("RenoDX needs ReShade with add-on support"));
        assert!(install(&ops(&g.paths, &net), "sp", &f, &Risk::default(), false).is_err());
        let ac = Risk { banned: false, findings: vec![Finding { name: "Easy Anti-Cheat".into(), source: Source::List }] };
        assert!(refusal(&m, &f, &ac).unwrap().contains("anti-cheat"));
        let mut x86 = m.clone();
        x86.addon = true;
        x86.arch = "x86".into();
        assert!(refusal(&x86, &f, &Risk::default()).unwrap().contains("32-bit"));
    }

    #[test]
    fn installs_replaces_and_removes_with_reshade() {
        let (g, net) = setup(true);
        let dir = g.dir.path();
        let o = ops(&g.paths, &net);
        let m = install(&o, "sp", &found("generic-unreal"), &Risk::default(), false).unwrap();
        assert!(dir.join("renodx-ue-extended.addon64").is_file());
        assert_eq!(installed(&m), Some(Installed { id: "generic-unreal".into(), file: "renodx-ue-extended.addon64".into() }));
        // Another mod replaces it.
        let m = install(&o, "sp", &found("stalker2"), &Risk::default(), false).unwrap();
        assert!(!dir.join("renodx-ue-extended.addon64").exists() && dir.join("renodx-stalker2.addon64").is_file());
        assert_eq!(installed(&m).unwrap().id, "stalker2");
        assert_eq!(m.files.iter().filter(|f| f.kind == Kind::Addon).count(), 1);
        // Back to the standard build is refused while it is there.
        assert!(crate::reshade::manage::set_addon(&o, "sp", false, &Risk::default()).is_err());
        let (m, kept) = remove(&g.paths, "sp").unwrap();
        assert!(kept.is_empty() && installed(&m).is_none() && !dir.join("renodx-stalker2.addon64").exists());
        assert!(!crate::reshade::manage::set_addon(&o, "sp", false, &Risk::default()).unwrap().addon);
    }

    #[test]
    fn a_foreign_or_edited_file_is_never_overwritten() {
        let (g, net) = setup(true);
        let dir = g.dir.path();
        let o = ops(&g.paths, &net);
        std::fs::write(dir.join("renodx-ue-extended.addon64"), b"the user's own").unwrap();
        assert!(install(&o, "sp", &found("generic-unreal"), &Risk::default(), false).unwrap_err().contains("not installed by Heronite"));
        std::fs::remove_file(dir.join("renodx-ue-extended.addon64")).unwrap();
        install(&o, "sp", &found("generic-unreal"), &Risk::default(), false).unwrap();
        std::fs::write(dir.join("renodx-ue-extended.addon64"), b"edited").unwrap();
        assert!(install(&o, "sp", &found("stalker2"), &Risk::default(), false).unwrap_err().contains("changed"));
        let (_, kept) = remove(&g.paths, "sp").unwrap();
        assert_eq!(kept, ["renodx-ue-extended.addon64"]);
        assert!(dir.join("renodx-ue-extended.addon64").exists());
    }

    #[test]
    fn a_download_of_the_wrong_kind_is_refused() {
        let (g, net) = setup_with(true, pe_with(MACHINE_X86, &[], None, 0));
        let err = install(&ops(&g.paths, &net), "sp", &found("generic-unreal"), &Risk::default(), false).unwrap_err();
        assert!(err.contains("64-bit"), "{err}");
        assert!(!g.dir.path().join("renodx-ue-extended.addon64").exists());
        assert!(!cache_file(&g.paths, UE).exists());
    }
}
