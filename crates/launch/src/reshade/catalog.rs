//! ReShade's own lists, read like its installer does
//! (`crosire/reshade`, `setup/MainWindow.xaml.cs`):
//!
//! * `EffectPackages.ini`: the shader packages offered at install, one
//!   numbered section each.
//! * `Compatibility.ini`: per executable name, the render API, where to
//!   install, the depth buffer settings, and `Banned=1` for games that ban
//!   or block ReShade.

use super::detect::Api;
use super::ini::{list, Ini};

pub const PACKAGES_URL: &str = "https://raw.githubusercontent.com/crosire/reshade-shaders/list/EffectPackages.ini";
pub const COMPATIBILITY_URL: &str = "https://raw.githubusercontent.com/crosire/reshade-shaders/list/Compatibility.ini";

/// A shader package of `EffectPackages.ini`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// `PackageName`: also how the manifest names it.
    pub name: String,
    pub description: String,
    /// `InstallPath` / `TextureInstallPath`, relative to the game's folder
    /// with ReShade's backslashes (`.\reshade-shaders\Shaders`).
    pub install_path: String,
    pub texture_install_path: String,
    /// A zip of the repository.
    pub download_url: String,
    pub repository_url: String,
    /// File names of the effects it brings.
    pub effect_files: Vec<String>,
    /// Effect files of the zip that are left out.
    pub deny_effect_files: Vec<String>,
    /// Ticked by default.
    pub enabled: bool,
    /// Always installed ("Standard effects").
    pub required: bool,
}

impl Package {
    pub fn has_effect(&self, file: &str) -> bool {
        self.effect_files.iter().any(|f| f.eq_ignore_ascii_case(file))
    }
}

pub fn packages(text: &str) -> Vec<Package> {
    let ini = Ini::parse(text);
    ini.sections()
        .into_iter()
        .filter_map(|s| {
            let get = |k: &str| ini.get(s, k).unwrap_or("").to_string();
            let name = get("PackageName");
            let download_url = get("DownloadUrl");
            if name.is_empty() || download_url.is_empty() {
                return None;
            }
            Some(Package {
                name,
                description: get("PackageDescription"),
                install_path: Some(get("InstallPath")).filter(|p| !p.is_empty()).unwrap_or_else(|| ".\\reshade-shaders\\Shaders".into()),
                texture_install_path: Some(get("TextureInstallPath"))
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| ".\\reshade-shaders\\Textures".into()),
                download_url,
                repository_url: get("RepositoryUrl"),
                effect_files: list(&get("EffectFiles")),
                deny_effect_files: list(&get("DenyEffectFiles")),
                enabled: get("Enabled") == "1",
                required: get("Required") == "1",
            })
        })
        .collect()
}

/// What `Compatibility.ini` says about one executable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compat {
    pub banned: bool,
    pub api: Option<Api>,
    /// A folder relative to the executable's where ReShade goes.
    pub install_target: Option<String>,
    /// Raw values of the depth settings (`None`: not given).
    pub depth_reversed: Option<String>,
    pub depth_upside_down: Option<String>,
    pub depth_copy_before_clears: Option<String>,
    pub depth_copy_at_clear_index: Option<String>,
    pub use_aspect_ratio_heuristics: Option<String>,
}

/// The entry of `exe_name` (a file name; compared without case).
pub fn compat(text: &str, exe_name: &str) -> Option<Compat> {
    let ini = Ini::parse(text);
    let section = ini.sections().into_iter().find(|s| s.eq_ignore_ascii_case(exe_name))?;
    // Both spellings are in the file (`RenderApi` and `RenderAPI`); the
    // lookup ignores case.
    let get = |k: &str| ini.get(section, k).map(str::to_string).filter(|v| !v.is_empty());
    Some(Compat {
        banned: get("Banned").as_deref() == Some("1"),
        api: get("RenderApi").as_deref().and_then(Api::from_compat),
        install_target: get("InstallTarget"),
        depth_reversed: get("DepthReversed"),
        depth_upside_down: get("DepthUpsideDown"),
        depth_copy_before_clears: get("DepthCopyBeforeClears"),
        depth_copy_at_clear_index: get("DepthCopyAtClearIndex"),
        use_aspect_ratio_heuristics: get("UseAspectRatioHeuristics"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGES: &str = "[00]\nEnabled=1\nRequired=1\nPackageName=Standard effects\nInstallPath=.\\reshade-shaders\\Shaders\nTextureInstallPath=.\\reshade-shaders\\Textures\nDownloadUrl=https://github.com/crosire/reshade-shaders/archive/slim.zip\nEffectFiles=Daltonize.fx,Deband.fx\n\n[01]\nEnabled=1\nPackageName=SweetFX by CeeJay.dk\nDownloadUrl=https://github.com/CeeJayDK/SweetFX/archive/master.zip\nEffectFiles=Curves.fx,Tonemap.fx\nDenyEffectFiles=Template.fx\n\n[02]\nPackageName=OtisFX\nInstallPath=.\\reshade-shaders\\Shaders\\OtisFX\nDownloadUrl=https://x/o.zip\n\n[03]\nPackageName=No download\n";

    #[test]
    fn reads_the_packages() {
        let p = packages(PACKAGES);
        assert_eq!(p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Standard effects", "SweetFX by CeeJay.dk", "OtisFX"]);
        assert!(p[0].required && p[0].enabled && !p[2].enabled && !p[1].required);
        assert_eq!(p[1].deny_effect_files, ["Template.fx"]);
        assert!(p[1].has_effect("tonemap.fx"));
        assert_eq!(p[1].install_path, ".\\reshade-shaders\\Shaders");
        assert_eq!(p[2].install_path, ".\\reshade-shaders\\Shaders\\OtisFX");
        assert_eq!(p[2].texture_install_path, ".\\reshade-shaders\\Textures");
    }

    #[test]
    fn reads_compatibility_by_executable_name() {
        let text = "\u{feff}# Apex\n[r5apex.exe]\nBanned=1\nRenderApi=D3D11\n\n[Game.exe]\nRenderAPI=OpenGL\nDepthReversed=0\nInstallTarget=bin\\x64\n[Old.exe]\nRenderApi=D3D8\n";
        let apex = compat(text, "R5Apex.exe").unwrap();
        assert!(apex.banned);
        assert_eq!(apex.api, Some(Api::Dxgi));
        let game = compat(text, "game.exe").unwrap();
        assert_eq!(
            (game.api, game.depth_reversed.as_deref(), game.install_target.as_deref()),
            (Some(Api::OpenGl), Some("0"), Some("bin\\x64"))
        );
        assert!(!game.banned);
        assert_eq!(compat(text, "old.exe").unwrap().api, Some(Api::D3d8));
        assert!(compat(text, "other.exe").is_none());
    }
}
