//! ReShade presets: which effect files a preset turns on. `Techniques`
//! lists the active techniques (`Name@File.fx`, comma-separated);
//! `TechniqueSorting` only orders every technique, active or not.

use super::catalog::Package;
use super::ini::{list, Ini};

/// The `.fx` files of the active techniques, in order, without repeats.
/// Old presets name techniques without their file: those are skipped.
pub fn active_effects(text: &str) -> Vec<String> {
    let ini = Ini::parse(text);
    let mut out: Vec<String> = Vec::new();
    for t in list(ini.get("", "Techniques").unwrap_or("")) {
        if let Some((_, file)) = t.split_once('@') {
            let file = file.trim();
            if !file.is_empty() && !out.iter().any(|f| f.eq_ignore_ascii_case(file)) {
                out.push(file.to_string());
            }
        }
    }
    out
}

/// How a preset's effects map to the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Needs {
    /// Packages that bring at least one effect, in catalog order.
    pub packages: Vec<String>,
    /// Effects no package nor installed file brings.
    pub missing: Vec<String>,
}

/// `present`: whether an effect file is already there (an extra source).
pub fn needs(effects: &[String], packages: &[Package], present: impl Fn(&str) -> bool) -> Needs {
    let mut n = Needs::default();
    for p in packages {
        if effects.iter().any(|e| p.has_effect(e)) {
            n.packages.push(p.name.clone());
        }
    }
    n.missing = effects.iter().filter(|e| !packages.iter().any(|p| p.has_effect(e)) && !present(e)).cloned().collect();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACHETE: &str = "PreprocessorDefinitions=\nTechniques=Colourfulness@Colourfulness.fx, Tonemap@Tonemap.fx,Curves@Curves.fx,Levels@Levels.fx,SlowSharp@SlowSharp.fx,Tonemap@Tonemap.fx\nTechniqueSorting=Colourfulness@Colourfulness.fx,SMAA@SMAA.fx,FXAA@FXAA.fx\nKeyPCGI_One@RadiantGI.fx=46,1,0,0\n\n[Curves.fx]\nContrast=0.1\n";

    fn package(name: &str, effects: &[&str]) -> Package {
        Package {
            name: name.into(),
            description: String::new(),
            install_path: String::new(),
            texture_install_path: String::new(),
            download_url: String::new(),
            repository_url: String::new(),
            effect_files: effects.iter().map(|e| e.to_string()).collect(),
            deny_effect_files: Vec::new(),
            enabled: false,
            required: false,
        }
    }

    #[test]
    fn only_active_techniques_count() {
        assert_eq!(active_effects(MACHETE), ["Colourfulness.fx", "Tonemap.fx", "Curves.fx", "Levels.fx", "SlowSharp.fx"]);
        assert!(active_effects("Techniques=Old,Names\n").is_empty());
        assert!(active_effects("[GENERAL]\nTechniques=A@A.fx\n").is_empty());
    }

    #[test]
    fn maps_effects_to_packages_and_lists_the_rest() {
        let catalog = [
            package("Standard effects", &["Deband.fx"]),
            package("SweetFX", &["Tonemap.fx", "Curves.fx", "Levels.fx"]),
            package("Legacy effects", &["Colourfulness.fx"]),
        ];
        let effects = active_effects(MACHETE);
        let n = needs(&effects, &catalog, |_| false);
        assert_eq!(n.packages, ["SweetFX", "Legacy effects"]);
        assert_eq!(n.missing, ["SlowSharp.fx"]);
        let found = needs(&effects, &catalog, |e| e == "SlowSharp.fx");
        assert!(found.missing.is_empty());
    }
}
