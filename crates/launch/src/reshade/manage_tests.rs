use super::*;
use crate::diagnose::pe::tests::pe_with;
use crate::diagnose::pe::MACHINE_X64;
use crate::reshade::fetch;
use crate::reshade::install::install;
use crate::reshade::install::tests::{game, net, ops, package, request, Game, SWEETFX};

fn installed(preset: Option<&str>) -> (Game, crate::net::FakeNet) {
    let g = game();
    let net = net();
    let p = preset.map(|text| {
        let path = g._data.path().join("Machete.ini");
        std::fs::write(&path, text).unwrap();
        path
    });
    install(&ops(&g.paths, &net), &request(&g, p), None).unwrap();
    (g, net)
}

fn trash(into: &Path) -> impl Fn(&Path) -> Result<(), String> + '_ {
    move |p: &Path| std::fs::rename(p, into.join(p.file_name().unwrap())).map_err(|e| e.to_string())
}

#[test]
fn switching_renames_the_dll() {
    let (g, _) = installed(None);
    let dir = g.dir.path();
    let m = set_enabled(&g.paths, "sp", false).unwrap();
    assert!(!m.enabled && !dir.join("dxgi.dll").exists() && dir.join("dxgi.dll.heronite-off").exists());
    set_enabled(&g.paths, "sp", true).unwrap();
    assert!(dir.join("dxgi.dll").exists() && !dir.join("dxgi.dll.heronite-off").exists());
}

#[test]
fn updating_replaces_only_the_dll() {
    let (g, net) = installed(Some("Techniques=Tonemap@Tonemap.fx\n"));
    let dir = g.dir.path();
    std::fs::write(dir.join("Machete.ini"), "Techniques=Tonemap@Tonemap.fx\nEdited=1\n").unwrap();
    let ini = std::fs::read_to_string(dir.join("ReShade.ini")).unwrap() + "[OVERLAY]\nWindow=x\n";
    std::fs::write(dir.join("ReShade.ini"), &ini).unwrap();
    set_enabled(&g.paths, "sp", false).unwrap();
    let m = update_dll(&ops(&g.paths, &net), "sp", "6.9.0").unwrap();
    assert_eq!(m.version, "6.9.0");
    assert!(dir.join("dxgi.dll.heronite-off").exists() && !dir.join("dxgi.dll").exists());
    assert_eq!(std::fs::read_to_string(dir.join("ReShade.ini")).unwrap(), ini);
    assert!(std::fs::read_to_string(dir.join("Machete.ini")).unwrap().contains("Edited=1"));
}

#[test]
fn removing_deletes_heronites_files_and_trashes_the_settings() {
    let (g, _) = installed(Some("Techniques=Tonemap@Tonemap.fx\n"));
    let dir = g.dir.path();
    std::fs::write(dir.join("reshade-shaders/Shaders/SweetFX/Curves.fx"), "my edit").unwrap();
    std::fs::write(dir.join("ReShade.log"), "log").unwrap();
    let bin = tempfile::tempdir().unwrap();
    let r = remove(&g.paths, "sp", &trash(bin.path())).unwrap();
    assert!(!dir.join("dxgi.dll").exists());
    assert!(!dir.join("reshade-shaders/Shaders/ReShade.fxh").exists());
    assert_eq!(r.kept, ["reshade-shaders/Shaders/SweetFX/Curves.fx"]);
    assert!(dir.join("reshade-shaders/Shaders/SweetFX/Curves.fx").exists());
    assert!(!dir.join("reshade-shaders/Textures").exists());
    for f in ["ReShade.ini", "Machete.ini", "ReShade.log"] {
        assert!(!dir.join(f).exists() && bin.path().join(f).exists(), "{f}");
    }
    assert!(dir.join("FL_2027.exe").exists());
    assert!(manifest::load(&g.paths, "sp").is_none());
}

#[test]
fn removing_a_package_keeps_shared_and_edited_files() {
    let (g, _) = installed(None);
    let dir = g.dir.path();
    std::fs::write(dir.join("reshade-shaders/Shaders/SweetFX/Curves.fx"), "my edit").unwrap();
    let (m, kept) = remove_source(&g.paths, "sp", "SweetFX").unwrap();
    assert_eq!(kept.files, ["reshade-shaders/Shaders/SweetFX/Curves.fx"]);
    assert!(!dir.join("reshade-shaders/Shaders/SweetFX/Tonemap.fx").exists());
    assert!(dir.join("reshade-shaders/Shaders/ReShade.fxh").exists());
    assert_eq!(m.file("reshade-shaders/Shaders/ReShade.fxh").unwrap().source, ["Standard effects"]);
    assert_eq!(m.packages, ["Standard effects"]);
}

#[test]
fn packages_and_folders_are_added_later() {
    let g = game();
    let net = net();
    let mut req = request(&g, None);
    req.packages.truncate(1);
    install(&ops(&g.paths, &net), &req, None).unwrap();
    let m = add_packages(&ops(&g.paths, &net), "sp", &[package("SweetFX", SWEETFX, &["Tonemap.fx"], false)]).unwrap();
    assert!(m.has_effect("Tonemap.fx"));
    let folder = g._data.path().join("NiceGuy-Shaders");
    std::fs::create_dir_all(folder.join("Shaders")).unwrap();
    std::fs::write(folder.join("Shaders/SlowSharp.fx"), "slow").unwrap();
    std::fs::create_dir_all(folder.join("Textures")).unwrap();
    std::fs::write(folder.join("Textures/noise.png"), "png").unwrap();
    let source = folder.to_string_lossy().into_owned();
    let m = add_extra(&ops(&g.paths, &net), "sp", &source).unwrap();
    let dir = g.dir.path();
    assert_eq!(std::fs::read(dir.join("reshade-shaders/Shaders/NiceGuy-Shaders/SlowSharp.fx")).unwrap(), b"slow");
    assert!(dir.join("reshade-shaders/Textures/NiceGuy-Shaders/noise.png").exists());
    assert_eq!(m.extra_sources, std::slice::from_ref(&source));
    assert_eq!(m.effects_of(&source), ["SlowSharp.fx"]);
    let (m, _) = remove_source(&g.paths, "sp", &source).unwrap();
    assert!(!dir.join("reshade-shaders/Shaders/NiceGuy-Shaders").exists());
    assert!(m.extra_sources.is_empty());
}

#[test]
fn a_manual_install_is_adopted_as_it_is() {
    let g = game();
    let dir = g.dir.path();
    let dll = pe_with(MACHINE_X64, &[], Some("ReShade"), 0);
    std::fs::write(dir.join("dxgi.dll"), &dll).unwrap();
    std::fs::write(dir.join("ReShade.ini"), "[GENERAL]\nPresetPath=.\\MachetePES21.ini\n").unwrap();
    std::fs::write(dir.join("MachetePES21.ini"), "Techniques=Tonemap@Tonemap.fx\n").unwrap();
    std::fs::create_dir_all(dir.join("reshade-shaders/Shaders")).unwrap();
    std::fs::write(dir.join("reshade-shaders/Shaders/Tonemap.fx"), "t").unwrap();
    std::fs::create_dir_all(dir.join("reshade-shaders/Textures")).unwrap();
    std::fs::write(dir.join("reshade-shaders/Textures/lut.png"), "p").unwrap();
    let m = adopt(&g.paths, "sp", dir, "dxgi.dll").unwrap();
    assert!(m.adopted && m.enabled);
    assert_eq!((m.api.as_str(), m.arch.as_str(), m.preset.as_deref()), ("dxgi", "x64", Some("MachetePES21.ini")));
    assert_eq!(m.extra_sources, [ADOPTED]);
    assert_eq!(m.file("reshade-shaders/Textures/lut.png").unwrap().kind, Kind::Texture);
    assert_eq!(std::fs::read(dir.join("dxgi.dll")).unwrap(), dll);
    set_enabled(&g.paths, "sp", false).unwrap();
    assert!(dir.join("dxgi.dll.heronite-off").exists());
    // Replacing the adopted install keeps the settings and the preset.
    set_enabled(&g.paths, "sp", true).unwrap();
    let net = net();
    let previous = manifest::load(&g.paths, "sp").unwrap();
    let m = install(&ops(&g.paths, &net), &request(&g, None), Some(&previous)).unwrap();
    assert!(!m.adopted);
    assert!(fetch::validate_dll(&dir.join("dxgi.dll"), Arch::X64).is_ok());
    assert!(std::fs::read_to_string(dir.join("ReShade.ini")).unwrap().contains("PresetPath=.\\MachetePES21.ini"));
}

#[test]
fn a_missing_dll_is_put_back_before_launching() {
    let (g, _) = installed(None);
    let dir = g.dir.path();
    std::fs::remove_file(dir.join("dxgi.dll")).unwrap();
    assert_eq!(recover(&g.paths, "sp"), Some(Recovery::Restored { version: "6.8.0".into() }));
    assert!(dir.join("dxgi.dll").exists());
    assert_eq!(recover(&g.paths, "sp"), None);
    std::fs::write(dir.join("dxgi.dll"), pe_with(MACHINE_X64, &[], Some("Special K"), 0)).unwrap();
    assert!(matches!(recover(&g.paths, "sp"), Some(Recovery::Blocked { .. })));
    assert!(!manifest::load(&g.paths, "sp").unwrap().enabled);
}

#[test]
fn games_sharing_a_folder_keep_each_others_files() {
    let (g, _) = installed(None);
    let m = manifest::load(&g.paths, "sp").unwrap();
    manifest::save(&g.paths, "other", &m).unwrap();
    assert_eq!(sharing(&g.paths, "sp", &m.dir()).len(), 1);
    let bin = tempfile::tempdir().unwrap();
    remove(&g.paths, "sp", &trash(bin.path())).unwrap();
    assert!(g.dir.path().join("dxgi.dll").exists());
}

#[test]
fn the_launch_names_the_dll_only_under_wine_and_when_it_is_on() {
    use crate::reshade::{apply, ReShade};
    let (g, _) = installed(None);
    let on = ReShade { enabled: true, api: "auto".into(), ..Default::default() };
    let mut env = crate::env::Env::new();
    env.insert("WINEDLLOVERRIDES".into(), "winemenubuilder.exe=d".into());
    let line = apply(&mut env, &g.paths, "sp", &on, true).unwrap();
    assert!(line.starts_with("ReShade 6.8.0 (dxgi, x64) at "), "{line}");
    assert_eq!(env["WINEDLLOVERRIDES"], "winemenubuilder.exe=d;dxgi=n,b");
    let mut windows = crate::env::Env::new();
    assert!(apply(&mut windows, &g.paths, "sp", &on, false).is_some());
    assert!(windows.is_empty());
    let mut off = crate::env::Env::new();
    assert!(apply(&mut off, &g.paths, "sp", &ReShade::default(), true).is_none());
    set_enabled(&g.paths, "sp", false).unwrap();
    assert!(apply(&mut off, &g.paths, "sp", &on, true).is_none());
    assert!(off.is_empty());
    assert!(apply(&mut off, &g.paths, "other", &on, true).is_none());
}
