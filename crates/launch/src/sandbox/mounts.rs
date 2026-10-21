//! What a sandbox shows from the host besides the system folders (those, the
//! devices and the session sockets are in [`super::script`]). Pure: built
//! from the settings, the request and the probe.

use heronite_i18n::tr;

use super::{private_home, Bwrap, Owner, Request};
use crate::env::join;
use crate::plan::PlanContext;
use crate::probe::Probe;
use crate::settings::GameSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    ReadOnly,
    ReadWrite,
    /// Writes land in a layer thrown away at the end (Winetricks' downloads:
    /// a change would reach every prefix).
    Throwaway,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Bind {
    /// Where it is, inside and outside.
    pub path: String,
    pub access: Access,
    /// Must exist (created first): `--bind`, not `--bind-try`.
    pub required: bool,
    /// Where it comes from when not from `path`.
    pub source: Option<String>,
}

/// The host folders a sandbox shows, and where its home comes from.
#[derive(Debug, Clone, Default)]
pub struct Mounts {
    /// The user's home folder (`$HOME`), hidden behind `private_home`.
    pub home: String,
    pub private_home: String,
    pub(super) binds: Vec<Bind>,
    /// Folders created before `bwrap` starts (it needs the sources).
    pub make: Vec<String>,
}

/// Files and folders of the home the sandbox reads when they exist: Vulkan
/// layers, MangoHud's settings, the X11 and PulseAudio cookies.
const HOME_READ: [&str; 5] = [".local/share/vulkan", ".config/MangoHud", ".config/pulse/cookie", ".Xauthority", ".config/lsfg-vk"];

/// The shader caches DXVK and VKD3D-Proton write next to the game's program
/// unless told where: in the sandbox that folder is read-only, so they go in
/// its home (the variable, and the folder in the home). A path the user set
/// is kept.
pub(super) const SHADER_CACHES: [(&str, &str); 2] =
    [("DXVK_STATE_CACHE_PATH", ".cache/dxvk"), ("VKD3D_SHADER_CACHE_PATH", ".cache/vkd3d-proton")];

/// The system folders every sandbox shows read-only (see [`super::script`]).
pub(super) const SYSTEM: [&str; 4] = ["/usr", "/etc", "/opt", "/nix"];

impl Mounts {
    pub fn build(
        ctx: &PlanContext,
        probe: &dyn Probe,
        s: &GameSettings,
        owner: Owner,
        req: &Request,
        bwrap: &Bwrap,
    ) -> Result<Self, String> {
        let home = clean(&probe.home()).unwrap_or_default();
        let private_home = private_home(&ctx.paths, owner);
        let mut m = Mounts { home: home.clone(), private_home: private_home.clone(), make: vec![private_home], ..Default::default() };
        let mut add = |path: &str, access: Access, required: bool| -> Result<(), String> {
            let Some(path) = clean(path) else { return Ok(()) };
            if too_wide(&path, &home) {
                return Err(tr!(
                    "The sandbox cannot share {path}: it would show your whole home folder. Move the game or the prefix into a folder of its own.",
                    path
                ));
            }
            // Already there, read-only, for every sandbox.
            if access == Access::ReadOnly && SYSTEM.iter().any(|d| inside(&path, d)) {
                return Ok(());
            }
            m.binds.push(Bind { path, access, required, source: None });
            Ok(())
        };

        if let Owner::Prefix(prefix) = owner {
            add(prefix, Access::ReadWrite, true)?;
        }
        if let Some(folder) = req.folder {
            add(folder, if s.sandbox.game_folder { Access::ReadWrite } else { Access::ReadOnly }, false)?;
        }
        if s.sandbox.shared_folders {
            for f in &s.shared_folders {
                add(f, Access::ReadWrite, false)?;
            }
        }
        for r in &req.read {
            add(r, Access::ReadOnly, false)?;
        }
        // The programs alongside the game join its sandbox later.
        for c in &s.companions {
            add(&crate::emulators::node_dirname(&c.path), Access::ReadOnly, false)?;
        }
        add(&wine_root(&s.wine_version.bin, &s.wine_version.kind), Access::ReadOnly, false)?;
        add(&ctx.tools_dir(), Access::ReadOnly, false)?;
        if let Some(env) = req.env {
            for key in ["LSFGVK_DLL_PATH", "LSFG_DLL_PATH"] {
                if let Some(dll) = env.get(key).filter(|v| v.starts_with('/')) {
                    add(&crate::emulators::node_dirname(dll), Access::ReadOnly, false)?;
                }
            }
            if env.keys().any(|k| k.starts_with("LSFG")) {
                add(&join(&home, ".local/lib"), Access::ReadOnly, false)?;
            }
            // Proton without umu starts the game through its `steam.exe`,
            // which needs the Steam client's libraries (only those: not the
            // account files next to them).
            if let Some(steam) = env.get("STEAM_COMPAT_CLIENT_INSTALL_PATH").filter(|v| v.starts_with('/')) {
                for sdk in [".steam/sdk32", ".steam/sdk64"] {
                    add(&join(&home, sdk), Access::ReadOnly, false)?;
                }
                add(&join(steam, "legacycompat"), Access::ReadOnly, false)?;
            }
        }
        for f in HOME_READ {
            add(&join(&home, f), Access::ReadOnly, false)?;
        }
        // Winetricks finds what it already downloaded (the sandbox may have
        // no network). Inside, `XDG_CACHE_HOME` is unset: it looks in the
        // default place.
        let cache = probe.env_var("XDG_CACHE_HOME").filter(|d| d.starts_with('/')).unwrap_or_else(|| join(&home, ".cache"));
        let source = join(&cache, "winetricks");
        // `--overlay-src` must exist.
        if !home.is_empty() && bwrap.overlay() && probe.exists(&source) {
            let path = join(&home, ".cache/winetricks");
            m.binds.push(Bind { source: (source != path).then_some(source), path, access: Access::Throwaway, required: false });
        }
        if !home.is_empty() {
            m.make.extend(SHADER_CACHES.iter().map(|(_, dir)| join(&m.private_home, dir)));
        }
        m.binds = order(std::mem::take(&mut m.binds));
        m.make.extend(m.binds.iter().filter(|b| b.required).map(|b| b.path.clone()));
        Ok(m)
    }
}

/// The folder of a Wine or Proton build: Proton's own folder, or the one
/// above Wine's `bin`.
fn wine_root(bin: &str, kind: &str) -> String {
    let dir = crate::emulators::node_dirname(bin);
    if kind != "proton" && dir.ends_with("/bin") {
        return crate::emulators::node_dirname(&dir);
    }
    dir
}

/// Absolute, without `.`/`..` or a trailing `/`; `None` for anything else.
fn clean(p: &str) -> Option<String> {
    let p = p.trim();
    if !p.starts_with('/') || p.split('/').any(|c| c == "." || c == "..") {
        return None;
    }
    let p = p.trim_end_matches('/');
    Some(if p.is_empty() { "/".into() } else { p.to_string() })
}

fn inside(path: &str, dir: &str) -> bool {
    path == dir || dir == "/" || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

/// `/`, the home folder or a folder holding it.
fn too_wide(path: &str, home: &str) -> bool {
    path == "/" || (!home.is_empty() && inside(home, path))
}

/// Without repeats and without anything already shown by a writable folder
/// around it (made read-only, a setup program could not write into its own
/// prefix), outer folders first so inner ones go on top.
fn order(binds: Vec<Bind>) -> Vec<Bind> {
    let mut out: Vec<Bind> = Vec::new();
    for b in binds {
        if out.iter().any(|o| o.access == Access::ReadWrite && inside(&b.path, &o.path)) {
            continue;
        }
        if b.access == Access::ReadWrite {
            // A writable folder wins over what was asked inside it, or at
            // the same place.
            out.retain(|o| !(inside(&o.path, &b.path) && o.access != Access::ReadWrite));
        }
        if out.iter().any(|o| o.path == b.path) {
            continue;
        }
        out.push(b);
    }
    out.sort_by_key(|b| b.path.matches('/').count());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(path: &str, access: Access) -> Bind {
        Bind { path: path.into(), access, required: false, source: None }
    }

    #[test]
    fn paths_are_cleaned_and_wide_ones_refused() {
        assert_eq!(clean("/a/b/"), Some("/a/b".into()));
        assert_eq!(clean("relative"), None);
        assert_eq!(clean("/a/../etc"), None);
        assert!(too_wide("/", "/home/u"));
        assert!(too_wide("/home", "/home/u"));
        assert!(too_wide("/home/u", "/home/u"));
        assert!(!too_wide("/home/u/Games", "/home/u"));
        assert!(!too_wide("/home/user2", "/home/u"));
    }

    #[test]
    fn writable_folders_cover_what_is_inside_them() {
        let out = order(vec![
            bind("/h/Games/pfx/drive_c/setup", Access::ReadOnly),
            bind("/h/Games/pfx", Access::ReadWrite),
            bind("/h/Games", Access::ReadOnly),
            bind("/h/Games/pfx/drive_c/tool", Access::ReadOnly),
            bind("/h/Games", Access::ReadOnly),
        ]);
        let paths: Vec<(&str, Access)> = out.iter().map(|b| (b.path.as_str(), b.access)).collect();
        assert_eq!(paths, [("/h/Games", Access::ReadOnly), ("/h/Games/pfx", Access::ReadWrite)]);
    }

    /// The game's folder comes first, read-only: shared with Proton later,
    /// it is writable.
    #[test]
    fn a_writable_folder_wins_over_the_same_one_read_only() {
        let out = order(vec![bind("/h/Games/Game", Access::ReadOnly), bind("/h/Games/Game", Access::ReadWrite)]);
        let paths: Vec<(&str, Access)> = out.iter().map(|b| (b.path.as_str(), b.access)).collect();
        assert_eq!(paths, [("/h/Games/Game", Access::ReadWrite)]);
    }

    #[test]
    fn wine_roots() {
        assert_eq!(wine_root("/p/GE-Proton9/proton", "proton"), "/p/GE-Proton9");
        assert_eq!(wine_root("/opt/wine/bin/wine", "wine"), "/opt/wine");
        assert_eq!(wine_root("/usr/bin/wine", "wine"), "/usr");
    }
}
