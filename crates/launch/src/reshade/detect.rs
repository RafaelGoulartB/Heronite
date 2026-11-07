//! Which executable gets ReShade, its architecture and render API, the DLL
//! name ReShade takes (the proxy) and what is already in its folder. The
//! rules are those of ReShade's installer (`setup/MainWindow.xaml.cs`).

use std::path::{Path, PathBuf};

use crate::diagnose::pe::{self, MACHINE_X64, MACHINE_X86};

use super::catalog::Compat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    /// Needs the `d3d8to9` wrapper: not supported.
    D3d8,
    D3d9,
    /// DirectX 10, 11 and 12.
    Dxgi,
    OpenGl,
    /// ReShade's Vulkan layer: not supported.
    Vulkan,
    DDraw,
}

impl Api {
    /// `RenderApi` of `Compatibility.ini`.
    pub fn from_compat(v: &str) -> Option<Self> {
        Some(match v.trim().to_ascii_uppercase().as_str() {
            "D3D8" => Api::D3d8,
            "D3D9" => Api::D3d9,
            "D3D10" | "D3D11" | "D3D12" | "DXGI" => Api::Dxgi,
            "OPENGL" => Api::OpenGl,
            "VULKAN" => Api::Vulkan,
            "DDRAW" => Api::DDraw,
            _ => return None,
        })
    }

    /// The DLL ReShade replaces (`None`: not supported here).
    pub fn proxy(self) -> Option<&'static str> {
        match self {
            Api::D3d9 => Some("d3d9.dll"),
            Api::Dxgi => Some("dxgi.dll"),
            Api::OpenGl => Some("opengl32.dll"),
            Api::DDraw => Some("ddraw.dll"),
            Api::D3d8 | Api::Vulkan => None,
        }
    }

    /// Other DLL names that work for the API (picked when the usual one is
    /// taken, like `d3d11.dll` next to a mod's `dxgi.dll`).
    pub fn alternatives(self) -> &'static [&'static str] {
        match self {
            Api::Dxgi => &["dxgi.dll", "d3d11.dll"],
            Api::D3d9 => &["d3d9.dll"],
            Api::OpenGl => &["opengl32.dll"],
            Api::DDraw => &["ddraw.dll"],
            Api::D3d8 | Api::Vulkan => &[],
        }
    }

    /// The `heroniteReshadeApi` value and the manifest's `api`.
    pub fn key(self) -> &'static str {
        match self {
            Api::D3d8 => "d3d8",
            Api::D3d9 => "d3d9",
            Api::Dxgi => "dxgi",
            Api::OpenGl => "opengl32",
            Api::Vulkan => "vulkan",
            Api::DDraw => "ddraw",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Api::D3d8, Api::D3d9, Api::Dxgi, Api::OpenGl, Api::Vulkan, Api::DDraw].into_iter().find(|a| a.key() == key)
    }

    /// DirectX 9 to 12 compile their effects with `d3dcompiler_47`.
    pub fn uses_d3dcompiler(self) -> bool {
        matches!(self, Api::D3d9 | Api::Dxgi)
    }
}

/// The render API from the DLLs an executable imports. A program can import
/// several: DXGI beats Direct3D 9, Vulkan beats DXGI, OpenGL loses to all.
pub fn api_from_imports(imports: &[String]) -> Option<Api> {
    let has = |f: &dyn Fn(&str) -> bool| imports.iter().any(|i| f(i));
    let d3d8 = has(&|i| i.starts_with("d3d8"));
    let d3d9 = has(&|i| i.starts_with("d3d9"));
    let dxgi = has(&|i| i.starts_with("dxgi") || i.starts_with("d3d1") || i.contains("gfsdk"));
    let vulkan = has(&|i| i.starts_with("vulkan-1"));
    let ddraw = has(&|i| i.starts_with("ddraw"));
    let opengl = has(&|i| i.starts_with("opengl32"));
    if vulkan {
        Some(Api::Vulkan)
    } else if dxgi {
        Some(Api::Dxgi)
    } else if d3d9 {
        Some(Api::D3d9)
    } else if d3d8 {
        Some(Api::D3d8)
    } else if ddraw {
        Some(Api::DDraw)
    } else if opengl {
        Some(Api::OpenGl)
    } else {
        None
    }
}

/// The DirectX version shown for DXGI games (`None`: unknown).
fn directx_version(imports: &[String]) -> Option<u8> {
    [("d3d12", 12), ("d3d11", 11), ("d3d10", 10)].into_iter().find(|(n, _)| imports.iter().any(|i| i.starts_with(n))).map(|(_, v)| v)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86,
    X64,
}

impl Arch {
    pub fn key(self) -> &'static str {
        match self {
            Arch::X86 => "x86",
            Arch::X64 => "x64",
        }
    }

    pub fn bits(self) -> u8 {
        match self {
            Arch::X86 => 32,
            Arch::X64 => 64,
        }
    }

    pub fn of_machine(machine: u16) -> Option<Self> {
        match machine {
            MACHINE_X86 => Some(Arch::X86),
            MACHINE_X64 => Some(Arch::X64),
            _ => None,
        }
    }

    /// `ReShade32.dll` / `ReShade64.dll`.
    pub fn dll(self) -> &'static str {
        match self {
            Arch::X86 => "ReShade32.dll",
            Arch::X64 => "ReShade64.dll",
        }
    }
}

/// Where and how ReShade goes for one executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub exe: PathBuf,
    /// The executable's folder, or `InstallTarget` of `Compatibility.ini`.
    pub dir: PathBuf,
    pub arch: Arch,
    /// `None`: the executable imports no graphics DLL (a launcher).
    pub api: Option<Api>,
    /// 9 to 12 for Direct3D games, when known.
    pub directx: Option<u8>,
    /// The API came from `Compatibility.ini`, not from the imports.
    pub api_from_list: bool,
    /// Under 1 MB of stack: ReShade's installer warns it can crash.
    pub small_stack: bool,
}

/// Programs ReShade's installer warns about below this stack size.
const SMALL_STACK: u64 = 1024 * 1024;

/// Reads `exe`. Not a Windows program (a Linux game): an error.
pub fn target(exe: &Path, compat: Option<&Compat>) -> Result<Target, String> {
    let info = pe::info(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let arch = Arch::of_machine(info.machine).ok_or_else(|| format!("{}: unsupported machine {:#x}", exe.display(), info.machine))?;
    let folder = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let dir = match compat.and_then(|c| c.install_target.as_deref()) {
        Some(sub) => sub.split(['\\', '/']).filter(|p| !p.is_empty() && *p != "." && *p != "..").fold(folder, |d, p| d.join(p)),
        None => folder,
    };
    let listed = compat.and_then(|c| c.api);
    let api = listed.or_else(|| api_from_imports(&info.imports));
    let directx = match api {
        Some(Api::D3d9) => Some(9),
        Some(Api::Dxgi) => directx_version(&info.imports),
        _ => None,
    };
    Ok(Target {
        exe: exe.to_path_buf(),
        dir,
        arch,
        api,
        directx,
        api_from_list: listed.is_some(),
        small_stack: info.stack_reserve > 0 && info.stack_reserve < SMALL_STACK,
    })
}

/// How deep below the game's folder its programs are looked for: Unreal's
/// `<Project>/Binaries/Win64` is three folders down.
const EXE_DEPTH: usize = 3;
/// Folders read at most while looking (a game's data can hold thousands).
const MAX_FOLDERS: usize = 400;

/// Folders whose programs are not the game: redistributables, anti-cheats,
/// installers, crash reporters and Unreal's engine tools.
fn not_the_game(folder: &str) -> bool {
    const NAMES: [&str; 14] = [
        "engine",
        "_commonredist",
        "commonredist",
        "redist",
        "redistributables",
        "directx",
        "vcredist",
        "dotnet",
        "easyanticheat",
        "battleye",
        "__installer",
        "installer",
        "support",
        "prerequisites",
    ];
    let f = folder.to_ascii_lowercase();
    NAMES.contains(&f.as_str()) || f.starts_with("crashreport") || f.starts_with("crashpad")
}

/// Windows programs of `dir` and its subfolders (down to [`EXE_DEPTH`],
/// past what is [`not_the_game`]) that import a graphics API, biggest
/// first: the choices when the game's executable is a launcher, or when it
/// is not known (a Steam game, where the real program may sit in
/// `<Project>/Binaries/Win64` or `platform/steam`).
pub fn candidates(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(u64, PathBuf)> = Vec::new();
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = [(dir.to_path_buf(), 0)].into();
    let mut read = 0;
    while let Some((folder, depth)) = queue.pop_front() {
        read += 1;
        if read > MAX_FOLDERS {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut subfolders = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if depth < EXE_DEPTH && !not_the_game(&e.file_name().to_string_lossy()) {
                    subfolders.push(path);
                }
                continue;
            }
            if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
                continue;
            }
            // Followed: a game's executable may be a link.
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let graphics = pe::imports(&path).ok().and_then(|i| api_from_imports(&i)).is_some();
            if graphics && meta.is_file() {
                found.push((meta.len(), path));
            }
        }
        subfolders.sort();
        queue.extend(subfolders.into_iter().map(|p| (p, depth + 1)));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Every DLL name ReShade can take, in the order folders are searched.
pub const PROXIES: [&str; 5] = ["dxgi.dll", "d3d9.dll", "d3d11.dll", "opengl32.dll", "ddraw.dll"];
/// What a switched-off ReShade DLL is renamed to: `<proxy>` + this.
pub const OFF_SUFFIX: &str = ".heronite-off";

/// What sits where ReShade would go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Occupant {
    Free,
    /// A ReShade DLL (`off`: renamed with [`OFF_SUFFIX`]).
    ReShade {
        path: PathBuf,
        off: bool,
        version: Option<String>,
    },
    /// Another DLL: DXVK, a mod, an upscaler. Never overwritten.
    Other {
        path: PathBuf,
        product: Option<String>,
    },
}

/// The occupant of `<dir>/<proxy>` (or of its switched-off name).
pub fn occupant(dir: &Path, proxy: &str) -> Occupant {
    for (name, off) in [(proxy.to_string(), false), (format!("{proxy}{OFF_SUFFIX}"), true)] {
        let path = dir.join(&name);
        if !path.is_file() {
            continue;
        }
        let info = pe::info(&path).ok();
        let product = info.as_ref().and_then(|i| i.product_name.clone());
        if product.as_deref().is_some_and(is_reshade) {
            return Occupant::ReShade { version: file_version(&path), path, off };
        }
        if !off {
            return Occupant::Other { path, product };
        }
    }
    Occupant::Free
}

pub fn is_reshade(product: &str) -> bool {
    product.trim().eq_ignore_ascii_case("ReShade")
}

/// The first ReShade DLL of `dir` under any proxy name: a manual install.
pub fn find_reshade(dir: &Path) -> Option<(String, Occupant)> {
    PROXIES.iter().find_map(|p| match occupant(dir, p) {
        o @ Occupant::ReShade { .. } => Some((p.to_string(), o)),
        _ => None,
    })
}

/// `ProductVersion` of a DLL, as `6.8.0` (ReShade writes `6.8.0.2105`;
/// the build number means nothing to users).
pub fn file_version(path: &Path) -> Option<String> {
    let v = pe::info(path).ok()?.product_version?;
    let parts: Vec<&str> = v.split(['.', ',']).map(str::trim).take(3).collect();
    (!parts.is_empty() && parts.iter().all(|p| p.parse::<u32>().is_ok())).then(|| parts.join("."))
}

/// The game's Windows executable: its target executable, else the store's
/// or the sideload entry's (relative to the install folder). `None` when
/// the file is missing or not a Windows program.
pub fn game_exe(install_path: Option<&str>, executable: Option<&str>, target_exe: &str) -> Option<PathBuf> {
    let exe = Some(target_exe.trim()).filter(|t| !t.is_empty()).or(executable.filter(|e| !e.trim().is_empty()))?;
    let path = match Path::new(exe).is_absolute() {
        true => PathBuf::from(exe),
        false => exe.split(['\\', '/']).filter(|p| !p.is_empty()).fold(PathBuf::from(install_path?), |d, p| d.join(p)),
    };
    (crate::wine_tools::is_windows_program(&path.to_string_lossy()) && path.is_file()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnose::pe::tests::pe_with;

    fn imports(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn api_rules_and_tie_breaks() {
        assert_eq!(api_from_imports(&imports(&["kernel32.dll", "d3d9.dll"])), Some(Api::D3d9));
        assert_eq!(api_from_imports(&imports(&["d3d8.dll"])), Some(Api::D3d8));
        assert_eq!(api_from_imports(&imports(&["d3d11.dll"])), Some(Api::Dxgi));
        assert_eq!(api_from_imports(&imports(&["gfsdk_aftermath_lib.x64.dll"])), Some(Api::Dxgi));
        assert_eq!(api_from_imports(&imports(&["ddraw.dll"])), Some(Api::DDraw));
        assert_eq!(api_from_imports(&imports(&["opengl32.dll"])), Some(Api::OpenGl));
        assert_eq!(api_from_imports(&imports(&["vulkan-1.dll"])), Some(Api::Vulkan));
        assert_eq!(api_from_imports(&imports(&["d3d9.dll", "dxgi.dll"])), Some(Api::Dxgi));
        assert_eq!(api_from_imports(&imports(&["dxgi.dll", "vulkan-1.dll"])), Some(Api::Vulkan));
        assert_eq!(api_from_imports(&imports(&["opengl32.dll", "d3d9.dll"])), Some(Api::D3d9));
        assert_eq!(api_from_imports(&imports(&["kernel32.dll"])), None);
        assert_eq!(directx_version(&imports(&["dxgi.dll", "d3d11.dll"])), Some(11));
    }

    #[test]
    fn the_list_wins_over_the_imports_and_moves_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Game.exe");
        std::fs::write(&exe, pe_with(MACHINE_X86, &["d3d9.dll"], None, 0x8000)).unwrap();
        let t = target(&exe, None).unwrap();
        assert_eq!((t.arch, t.api, t.directx, t.api_from_list, t.small_stack), (Arch::X86, Some(Api::D3d9), Some(9), false, true));
        assert_eq!(t.dir, dir.path());
        let compat = Compat { api: Some(Api::OpenGl), install_target: Some("bin\\x64".into()), ..Default::default() };
        let t = target(&exe, Some(&compat)).unwrap();
        assert_eq!((t.api, t.api_from_list), (Some(Api::OpenGl), true));
        assert_eq!(t.dir, dir.path().join("bin").join("x64"));
        std::fs::write(dir.path().join("run.sh"), "#!/bin/sh\n").unwrap();
        assert!(target(&dir.path().join("run.sh"), None).is_err());
    }

    #[test]
    fn launchers_point_to_the_graphics_executables() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("launcher.exe"), pe_with(MACHINE_X64, &["kernel32.dll"], None, 0x10_0000)).unwrap();
        let mut big = pe_with(MACHINE_X64, &["d3d11.dll"], None, 0x10_0000);
        big.extend(vec![0u8; 4096]);
        std::fs::write(dir.path().join("FL_2027.exe"), big).unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("small.exe"), pe_with(MACHINE_X64, &["d3d9.dll"], None, 0x10_0000)).unwrap();
        assert_eq!(candidates(dir.path()), [dir.path().join("FL_2027.exe"), bin.join("small.exe")]);
    }

    #[test]
    fn unreal_games_keep_theirs_under_the_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Game.exe"), pe_with(MACHINE_X64, &["kernel32.dll"], None, 0x10_0000)).unwrap();
        let win64 = dir.path().join("Game").join("Binaries").join("Win64");
        std::fs::create_dir_all(&win64).unwrap();
        std::fs::write(win64.join("Game-Win64-Shipping.exe"), pe_with(MACHINE_X64, &["d3d12.dll"], None, 0x10_0000)).unwrap();
        // The engine's own programs (crash reporter...) are not the game.
        let engine = dir.path().join("Engine").join("Binaries").join("Win64");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join("CrashReportClient.exe"), pe_with(MACHINE_X64, &["d3d11.dll"], None, 0x10_0000)).unwrap();
        assert_eq!(candidates(dir.path()), [win64.join("Game-Win64-Shipping.exe")]);
    }

    #[test]
    fn a_program_deeper_in_the_folder_is_found() {
        // Phantom Blade Zero: steam/PBZ.exe starts platform/steam/PBZ.exe.
        let dir = tempfile::tempdir().unwrap();
        let launcher = dir.path().join("steam");
        let game = dir.path().join("platform").join("steam");
        std::fs::create_dir_all(&launcher).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(launcher.join("PBZ.exe"), pe_with(MACHINE_X64, &["kernel32.dll"], None, 0x10_0000)).unwrap();
        std::fs::write(game.join("PBZ.exe"), pe_with(MACHINE_X64, &["dxgi.dll"], None, 0x10_0000)).unwrap();
        let redist = dir.path().join("_CommonRedist").join("DirectX");
        std::fs::create_dir_all(&redist).unwrap();
        std::fs::write(redist.join("DXSETUP.exe"), pe_with(MACHINE_X86, &["d3d9.dll"], None, 0x10_0000)).unwrap();
        // Too deep: a game's data, not its program.
        let deep = dir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("tool.exe"), pe_with(MACHINE_X64, &["d3d11.dll"], None, 0x10_0000)).unwrap();
        assert_eq!(candidates(dir.path()), [game.join("PBZ.exe")]);
    }

    #[test]
    fn what_sits_in_the_proxys_place() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(occupant(dir.path(), "dxgi.dll"), Occupant::Free);
        std::fs::write(dir.path().join("dxgi.dll"), pe_with(MACHINE_X64, &[], Some("DXVK"), 0)).unwrap();
        assert!(matches!(occupant(dir.path(), "dxgi.dll"), Occupant::Other { product: Some(p), .. } if p == "DXVK"));
        assert!(find_reshade(dir.path()).is_none());
        std::fs::write(dir.path().join("d3d9.dll.heronite-off"), pe_with(MACHINE_X64, &[], Some("ReShade"), 0)).unwrap();
        let (proxy, found) = find_reshade(dir.path()).unwrap();
        assert_eq!(proxy, "d3d9.dll");
        assert!(matches!(found, Occupant::ReShade { off: true, .. }));
    }

    #[test]
    fn the_games_executable() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("Game");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("game.exe"), b"MZ").unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        assert_eq!(game_exe(Some(&root), Some("Game\\game.exe"), ""), Some(sub.join("game.exe")));
        let abs = sub.join("game.exe").to_string_lossy().into_owned();
        assert_eq!(game_exe(None, Some("other.exe"), &abs), Some(sub.join("game.exe")));
        assert_eq!(game_exe(Some(&root), Some("missing.exe"), ""), None);
    }
}
