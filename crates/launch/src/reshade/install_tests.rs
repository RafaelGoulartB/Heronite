use super::*;
use crate::diagnose::pe::tests::pe_with;
use crate::diagnose::pe::MACHINE_X64;
use crate::net::FakeNet;
use crate::reshade::archive::tests::zip_bytes;
use crate::reshade::detect::{self, Api};
use crate::reshade::fetch;
use crate::reshade::fetch::{setup_url, tests::fake_setup};

pub(crate) const STANDARD: &str = "https://github.com/crosire/reshade-shaders/archive/slim.zip";
pub(crate) const SWEETFX: &str = "https://github.com/CeeJayDK/SweetFX/archive/master.zip";

pub(crate) fn package(name: &str, url: &str, effects: &[&str], required: bool) -> Package {
    Package {
        name: name.into(),
        description: String::new(),
        install_path: ".\\reshade-shaders\\Shaders".into(),
        texture_install_path: ".\\reshade-shaders\\Textures".into(),
        download_url: url.into(),
        repository_url: String::new(),
        effect_files: effects.iter().map(|e| e.to_string()).collect(),
        deny_effect_files: vec!["Template.fx".into()],
        enabled: true,
        required,
    }
}

/// A network with ReShade 6.8.0 and two packages sharing `ReShade.fxh`.
pub(crate) fn net() -> FakeNet {
    let standard: [(&str, &[u8]); 3] = [
        ("reshade-shaders-slim/Shaders/ReShade.fxh", b"fxh"),
        ("reshade-shaders-slim/Shaders/Deband.fx", b"deband"),
        ("reshade-shaders-slim/Textures/UIMask.png", b"png"),
    ];
    let sweetfx: [(&str, &[u8]); 4] = [
        ("SweetFX-master/Shaders/ReShade.fxh", b"fxh"),
        ("SweetFX-master/Shaders/SweetFX/Tonemap.fx", b"tonemap"),
        ("SweetFX-master/Shaders/SweetFX/Curves.fx", b"curves"),
        ("SweetFX-master/Shaders/Template.fx", b"template"),
    ];
    FakeNet::online()
        .with_file(&setup_url("6.8.0"), fake_setup())
        .with_file(&setup_url("6.9.0"), fake_setup())
        .with_file(STANDARD, zip_bytes(b"", &standard))
        .with_file(SWEETFX, zip_bytes(b"", &sweetfx))
}

pub(crate) struct Game {
    pub dir: tempfile::TempDir,
    pub paths: DataPaths,
    pub _data: tempfile::TempDir,
}

pub(crate) fn game() -> Game {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("FL_2027.exe"), pe_with(MACHINE_X64, &["d3d11.dll", "dxgi.dll"], None, 0x10_0000)).unwrap();
    Game { paths: DataPaths::at(data.path()), dir, _data: data }
}

pub(crate) fn request(g: &Game, preset: Option<PathBuf>) -> Request {
    let target = detect::target(&g.dir.path().join("FL_2027.exe"), None).unwrap();
    assert_eq!(target.api, Some(Api::Dxgi));
    Request {
        game_id: "sp".into(),
        target,
        proxy: "dxgi.dll".into(),
        version: "6.8.0".into(),
        packages: vec![
            package("Standard effects", STANDARD, &["Deband.fx"], true),
            package("SweetFX", SWEETFX, &["Tonemap.fx", "Curves.fx"], false),
        ],
        preset,
        compiler: false,
        compat: None,
    }
}

pub(crate) fn ops<'a>(paths: &'a DataPaths, net: &'a FakeNet) -> Ops<'a> {
    Ops { paths, net, cancel: &|| false, progress: &|_| {} }
}

#[test]
fn installs_the_dll_the_packages_the_preset_and_the_config() {
    let g = game();
    let net = net();
    let preset = g._data.path().join("Machete.ini");
    std::fs::write(&preset, "Techniques=Tonemap@Tonemap.fx\n").unwrap();
    let m = install(&ops(&g.paths, &net), &request(&g, Some(preset.clone())), None).unwrap();
    let dir = g.dir.path();
    assert!(fetch::validate_dll(&dir.join("dxgi.dll"), Arch::X64).is_ok());
    assert_eq!(std::fs::read(dir.join("reshade-shaders/Shaders/SweetFX/Tonemap.fx")).unwrap(), b"tonemap");
    assert_eq!(std::fs::read(dir.join("reshade-shaders/Textures/UIMask.png")).unwrap(), b"png");
    assert!(!dir.join("reshade-shaders/Shaders/Template.fx").exists());
    assert_eq!(std::fs::read_to_string(dir.join("Machete.ini")).unwrap(), "Techniques=Tonemap@Tonemap.fx\n");
    let config = std::fs::read_to_string(dir.join(CONFIG)).unwrap();
    assert!(
        config.contains("PresetPath=.\\Machete.ini")
            && config.contains("SkipLoadingDisabledEffects=1")
            && config.contains("KeyOverlay=36,0,0,0")
    );
    assert_eq!(m.packages, ["Standard effects", "SweetFX"]);
    assert_eq!(m.file("reshade-shaders/Shaders/ReShade.fxh").unwrap().source, ["Standard effects", "SweetFX"]);
    assert_eq!(m.preset.as_deref(), Some("Machete.ini"));
    assert_eq!(manifest::load(&g.paths, "sp"), Some(m));
    assert!(!dir.join(".heronite-write-test").exists());
}

#[test]
fn another_programs_dll_is_never_replaced() {
    let g = game();
    let net = net();
    let dxvk = pe_with(MACHINE_X64, &[], Some("DXVK"), 0);
    std::fs::write(g.dir.path().join("dxgi.dll"), &dxvk).unwrap();
    let err = install(&ops(&g.paths, &net), &request(&g, None), None).unwrap_err();
    assert!(err.contains("DXVK"), "{err}");
    assert_eq!(std::fs::read(g.dir.path().join("dxgi.dll")).unwrap(), dxvk);
    assert!(net.requests().is_empty());
    std::fs::remove_file(g.dir.path().join("dxgi.dll")).unwrap();
    std::fs::write(g.dir.path().join("dxgi.dll"), pe_with(MACHINE_X64, &[], Some("ReShade"), 0)).unwrap();
    assert!(install(&ops(&g.paths, &net), &request(&g, None), None).unwrap_err().contains("by hand"));
}

#[test]
fn a_failure_midway_undoes_what_was_written() {
    let g = game();
    let net = net();
    std::fs::write(g.dir.path().join(CONFIG), "[GENERAL]\nMine=1\n").unwrap();
    let mut req = request(&g, None);
    req.packages.push(package("Gone", "https://example.org/404.zip", &[], false));
    assert!(install(&ops(&g.paths, &net), &req, None).unwrap_err().contains("404"));
    let dir = g.dir.path();
    assert!(!dir.join("dxgi.dll").exists());
    assert!(!dir.join("reshade-shaders").exists());
    assert_eq!(std::fs::read_to_string(dir.join(CONFIG)).unwrap(), "[GENERAL]\nMine=1\n");
    assert!(manifest::load(&g.paths, "sp").is_none());
}

#[test]
fn the_games_own_compiler_is_left_alone() {
    let g = game();
    let net = net();
    std::fs::write(g.dir.path().join(COMPILER), b"the game's own").unwrap();
    let mut req = request(&g, None);
    req.compiler = true;
    let m = install(&ops(&g.paths, &net), &req, None).unwrap();
    assert_eq!(m.compiler, None);
    assert!(m.file(COMPILER).is_none());
    assert_eq!(std::fs::read(g.dir.path().join(COMPILER)).unwrap(), b"the game's own");
}

#[test]
fn the_config_keeps_the_users_values() {
    let existing = "[GENERAL]\nPresetPath=.\\Mine.ini\nSkipLoadingDisabledEffects=0\n\n[OVERLAY]\nWindow=x\n";
    let compat = Compat { depth_reversed: Some("0".into()), depth_copy_before_clears: Some("1".into()), ..Default::default() };
    let text = config_text(Some(existing), None, Some(&compat));
    assert!(text.contains("PresetPath=.\\Mine.ini") && text.contains("SkipLoadingDisabledEffects=0") && text.contains("Window=x"));
    assert!(text.contains("RESHADE_DEPTH_INPUT_IS_REVERSED=0") && text.contains("[DEPTH]\nDepthCopyBeforeClears=1"));
    let imported = config_text(Some(existing), Some("Machete.ini"), None);
    assert!(imported.contains("PresetPath=.\\Machete.ini") && imported.contains("SkipLoadingDisabledEffects=0"));
    let fresh = config_text(None, None, None);
    assert!(fresh.starts_with("[GENERAL]\nEffectSearchPaths=.\\reshade-shaders\\Shaders\\**\n"));
}
