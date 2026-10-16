use serde_json::json;

use super::detect::{display_name, parse_vulkan_summary, pci_name, proton_features, Card};
use super::*;
use crate::probe::FakeProbe;

fn gpu(v: Value) -> Gpu {
    Gpu::from_map(v.as_object().unwrap())
}

/// A machine with the NVIDIA card driving the screens and an AMD iGPU.
fn probe() -> FakeProbe {
    FakeProbe::new("/h")
        .with_file("/sys/bus/pci/devices/0000:01:00.0/vendor", b"0x10de\n")
        .with_file("/sys/bus/pci/devices/0000:01:00.0/device", b"0x2f04\n")
        .with_file("/sys/bus/pci/devices/0000:01:00.0/boot_vga", b"1\n")
        .with_file("/sys/bus/pci/devices/0000:0d:00.0/vendor", b"0x1002\n")
        .with_file("/sys/bus/pci/devices/0000:0d:00.0/device", b"0x164e\n")
        .with_file("/sys/bus/pci/devices/0000:0d:00.0/boot_vga", b"0\n")
}

#[test]
fn settings_read_with_defaults() {
    assert_eq!(gpu(json!({})), Gpu::default());
    let g = gpu(json!({
        "heroniteGpuDevice": " 0000:01:00.0 ", "heroniteGpuDlssPreset": "K", "heroniteGpuVsync": "off",
        "heroniteGpuVendor": "amd", "heroniteGpuVram": "4096", "heroniteGpuSmoothMotion": true, "nvidiaPrime": true
    }));
    assert_eq!(g.device, "0000:01:00.0");
    assert_eq!((g.dlss_preset, g.vsync, g.vendor, g.vram), (DlssPreset::K, Vsync::Off, Some(Vendor::Amd), 4096));
    assert!(g.smooth_motion && g.dedicated);
    assert_eq!(gpu(json!({"heroniteGpuVendor": "matrox", "heroniteGpuVsync": "x"})).vendor, None);
}

#[test]
fn picking_a_card_sets_mesa_and_nvidia_selection() {
    let p = probe();
    // The iGPU: Mesa picks it, NVIDIA's GLX is left out.
    let e = env(&gpu(json!({"heroniteGpuDevice": "0000:0d:00.0"})), &p);
    assert_eq!(e["DRI_PRIME"], "pci-0000_0d_00_0");
    assert_eq!(e["MESA_VK_DEVICE_SELECT"], "1002:164e");
    assert_eq!(e["__GLX_VENDOR_LIBRARY_NAME"], "mesa");
    assert!(!e.contains_key("__NV_PRIME_RENDER_OFFLOAD"));
    // The NVIDIA card drives the screens: no render offload.
    let e = env(&gpu(json!({"heroniteGpuDevice": "0000:01:00.0"})), &p);
    assert_eq!(e["MESA_VK_DEVICE_SELECT"], "10de:2f04");
    assert!(!e.contains_key("__NV_PRIME_RENDER_OFFLOAD"));
    // A laptop's NVIDIA card is offloaded to.
    let laptop = FakeProbe::new("/h")
        .with_file("/sys/bus/pci/devices/0000:01:00.0/vendor", b"0x10de")
        .with_file("/sys/bus/pci/devices/0000:01:00.0/device", b"0x28e0");
    let e = env(&gpu(json!({"heroniteGpuDevice": "0000:01:00.0"})), &laptop);
    assert_eq!(e["__NV_PRIME_RENDER_OFFLOAD"], "1");
    assert_eq!(e["__VK_LAYER_NV_optimus"], "NVIDIA_only");
    assert_eq!(e["__GLX_VENDOR_LIBRARY_NAME"], "nvidia");
    // A card that is gone, and the system's choice, add nothing.
    assert!(env(&gpu(json!({"heroniteGpuDevice": "0000:09:00.0"})), &p).is_empty());
    assert!(env(&gpu(json!({})), &p).is_empty());
    // Heroic's "dedicated GPU" switch keeps its variables.
    let e = env(&gpu(json!({"nvidiaPrime": true})), &p);
    assert_eq!((e["DRI_PRIME"].as_str(), e["__NV_PRIME_RENDER_OFFLOAD"].as_str()), ("1", "1"));
    // A picked card wins over it.
    assert_eq!(env(&gpu(json!({"nvidiaPrime": true, "heroniteGpuDevice": "0000:0d:00.0"})), &p)["DRI_PRIME"], "pci-0000_0d_00_0");
}

#[test]
fn only_the_drawing_cards_options_apply() {
    let all = gpu(json!({
        "heroniteGpuSmoothMotion": true, "heroniteGpuDlssUpgrade": true, "heroniteGpuFsr4Upgrade": true,
        "heroniteGpuAntiLag": true, "heroniteGpuGlthread": true, "heroniteGpuXessUpgrade": true, "heroniteGpuVsync": "off"
    }));
    let p = probe()
        .with_file("/sys/bus/pci/devices/0000:01:00.0/class", b"0x030000")
        .with_file("/sys/bus/pci/devices/0000:0d:00.0/class", b"0x030000")
        .with_file("/sys/bus/pci/devices/0000:00:01.0/class", b"0x060000");
    // The system's choice: the NVIDIA card the screens start on.
    let g = for_launch(&all, &p);
    assert!(g.smooth_motion && g.dlss_upgrade && !g.fsr4_upgrade && !g.anti_lag && !g.glthread);
    // The AMD iGPU picked: its options, none of NVIDIA's.
    let mut on_amd = all.clone();
    on_amd.device = "0000:0d:00.0".into();
    let g = for_launch(&on_amd, &p);
    assert!(!g.smooth_motion && !g.dlss_upgrade && g.fsr4_upgrade && g.anti_lag && g.glthread);
    assert!(g.xess_upgrade && g.vsync == Vsync::Off, "what every card has stays");
    assert!(!launch_options_vars(&on_amd, &p, None).iter().any(|(k, _)| k == "NVPRESENT_ENABLE_SMOOTH_MOTION"));
    // An Intel card: neither NVIDIA's nor FSR 4.
    let g = all.for_card(Some(INTEL));
    assert!(!g.smooth_motion && !g.fsr4_upgrade && g.anti_lag);
    // Unknown card: everything as set.
    assert_eq!(for_launch(&all, &FakeProbe::new("/h")), all);
}

#[test]
fn switches_split_between_every_game_and_windows_games() {
    let g = gpu(json!({
        "heroniteGpuSmoothMotion": true, "heroniteGpuShaderCache": true, "heroniteGpuGlthread": true,
        "heroniteGpuAntiLag": true, "heroniteGpuDlssUpgrade": true, "heroniteGpuDlssIndicator": true,
        "heroniteGpuFsr4Indicator": true, "heroniteGpuReflex": true, "heroniteGpuXessUpgrade": true
    }));
    let e = env(&g, &probe());
    assert_eq!(e["NVPRESENT_ENABLE_SMOOTH_MOTION"], "1");
    assert_eq!(e["__GL_SHADER_DISK_CACHE_SIZE"], SHADER_CACHE_BYTES);
    assert_eq!(e["__GL_SHADER_DISK_CACHE_SKIP_CLEANUP"], "1");
    assert_eq!(e["mesa_glthread"], "true");
    assert_eq!(e["ENABLE_LAYER_MESA_ANTI_LAG"], "1");
    assert!(!e.contains_key("PROTON_DLSS_UPGRADE"), "Proton's switches are for Windows games");
    let w = wine_env(&g, true, None);
    assert_eq!(w["PROTON_DLSS_UPGRADE"], "1");
    assert_eq!(w["PROTON_DLSS_INDICATOR"], "1");
    assert_eq!(w["PROTON_XESS_UPGRADE"], "1");
    assert_eq!(w["DXVK_NVAPI_VKREFLEX"], "1");
    assert!(!w.contains_key("PROTON_FSR4_INDICATOR"), "no indicator without its upgrade");
    assert!(!w.contains_key("NVPRESENT_ENABLE_SMOOTH_MOTION"));
    assert!(wine_env(&g, false, None).is_empty(), "plain Wine has none of Proton's switches");
}

#[test]
fn the_dlss_preset_needs_the_upgrade() {
    let on = wine_env(&gpu(json!({"heroniteGpuDlssUpgrade": true, "heroniteGpuDlssPreset": "latest"})), true, None);
    assert_eq!(on["DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION"], "render_preset_latest");
    let off = wine_env(&gpu(json!({"heroniteGpuDlssPreset": "k"})), true, None);
    assert!(!off.contains_key("DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION"));
}

#[test]
fn vsync_reaches_opengl_mesa_and_dxvk() {
    let g = gpu(json!({"heroniteGpuVsync": "off"}));
    let e = env(&g, &probe());
    assert_eq!((e["vblank_mode"].as_str(), e["__GL_SYNC_TO_VBLANK"].as_str()), ("0", "0"));
    assert_eq!(e["MESA_VK_WSI_PRESENT_MODE"], "immediate");
    assert_eq!(wine_env(&g, false, None)["DXVK_CONFIG"], "dxgi.syncInterval=0;d3d9.presentInterval=0");
    assert_eq!(env(&gpu(json!({"heroniteGpuVsync": "on"})), &probe())["MESA_VK_WSI_PRESENT_MODE"], "fifo");
}

#[test]
fn dxvk_config_keeps_the_users_items_last() {
    let g = gpu(json!({"heroniteGpuVendor": "nvidia", "heroniteGpuVram": 4096}));
    assert_eq!(wine_env(&g, true, None)["DXVK_CONFIG"], "dxgi.customVendorId=10de;dxgi.maxDeviceMemory=4096");
    assert_eq!(
        wine_env(&g, true, Some("d3d11.maxFeatureLevel=11_0"))["DXVK_CONFIG"],
        "dxgi.customVendorId=10de;dxgi.maxDeviceMemory=4096;d3d11.maxFeatureLevel=11_0"
    );
    assert!(!wine_env(&Gpu::default(), true, Some("a=1")).contains_key("DXVK_CONFIG"), "the user's own value stays theirs");
}

/// Writes the settings as a launch line's variables and reads them back.
fn roundtrip(v: Value) -> (Map<String, Value>, Vec<(String, String)>) {
    let g = gpu(v);
    let mut env: Vec<(String, String)> = launch_options_vars(&g, &probe(), None).into_iter().collect();
    let mut s = Map::new();
    read_launch_options(&mut env, &mut s);
    (s, env)
}

#[test]
fn launch_options_read_back() {
    let all = json!({
        "heroniteGpuDevice": "0000:0d:00.0", "heroniteGpuSmoothMotion": true, "heroniteGpuShaderCache": true,
        "heroniteGpuDlssUpgrade": true, "heroniteGpuDlssPreset": "k", "heroniteGpuDlssIndicator": true,
        "heroniteGpuFsr4Upgrade": true, "heroniteGpuXessUpgrade": true, "heroniteGpuAntiLag": true,
        "heroniteGpuGlthread": true, "heroniteGpuReflex": true, "heroniteGpuVsync": "on",
        "heroniteGpuVendor": "intel", "heroniteGpuVram": 6144
    });
    let (s, rest) = roundtrip(all.clone());
    assert!(rest.is_empty(), "every variable is a setting again: {rest:?}");
    // The AMD card is picked: NVIDIA's options are not written.
    assert_eq!(Gpu::from_map(&s), gpu(all).for_card(Some(AMD)));
    let (s, rest) = roundtrip(json!({}));
    assert!(rest.is_empty());
    assert_eq!(Gpu::from_map(&s), Gpu::default());
}

#[test]
fn foreign_variables_stay_plain() {
    let mut env: Vec<(String, String)> = vec![
        ("DXVK_CONFIG".into(), "dxgi.syncInterval=0;d3d11.cachedDynamicResources=a".into()),
        ("PROTON_DLSS_INDICATOR".into(), "1".into()),
        ("DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION".into(), "render_preset_j".into()),
        ("DRI_PRIME".into(), "1".into()),
        ("__GL_SHADER_DISK_CACHE_SKIP_CLEANUP".into(), "1".into()),
    ];
    let mut s = Map::new();
    read_launch_options(&mut env, &mut s);
    let g = Gpu::from_map(&s);
    assert_eq!(g, Gpu::default());
    // Half of the vsync pair is the user's; the other variables are untouched.
    assert!(env.contains(&("DXVK_CONFIG".into(), "d3d11.cachedDynamicResources=a;dxgi.syncInterval=0".into())));
    assert_eq!(env.len(), 5);
}

#[test]
fn pci_ids_and_vulkan_names() {
    let ids = "# comment\n10de  NVIDIA Corporation\n\t2f04  GB205 [GeForce RTX 5070]\n\t\t1043 8a2b  sub\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t164e  Raphael\n14e4  Broadcom\n\t164e  NetXtreme\n";
    assert_eq!(pci_name(ids, 0x10de, 0x2f04).as_deref(), Some("GB205 [GeForce RTX 5070]"));
    assert_eq!(pci_name(ids, 0x1002, 0x164e).as_deref(), Some("Raphael"));
    assert_eq!(pci_name(ids, 0x1002, 0x9999), None);
    assert_eq!(display_name(0x10de, 0x2f04, Some("GB205 [GeForce RTX 5070]"), None), "NVIDIA GeForce RTX 5070");
    assert_eq!(display_name(0x1002, 0x164e, Some("Raphael"), None), "AMD Raphael");
    assert_eq!(display_name(0x1002, 0x744c, None, Some("AMD Radeon RX 7900 XTX (RADV NAVI31)")), "AMD Radeon RX 7900 XTX");
    assert_eq!(display_name(0x8086, 0x56a0, None, Some("Intel(R) Arc(tm) A770 Graphics (DG2)")), "Intel Arc A770 Graphics");
    assert_eq!(display_name(0x10de, 0x2f04, None, Some("NVIDIA GeForce RTX 5070")), "NVIDIA GeForce RTX 5070");
    assert_eq!(display_name(0x1234, 0xabcd, None, None), "abcd");

    let summary = "Devices:\n========\nGPU0:\n\tvendorID           = 0x10de\n\tdeviceID           = 0x2f04\n\tdeviceType         = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU\n\tdeviceName         = NVIDIA GeForce RTX 5070\nGPU1:\n\tvendorID = 0x1002\n\tdeviceID = 0x164e\n\tdeviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n\tdeviceName = AMD Ryzen 5 7600 (RADV RAPHAEL_MENDOCINO)\nGPU2:\n\tvendorID = 0x10005\n";
    let devices = parse_vulkan_summary(summary);
    assert_eq!(devices.len(), 2, "llvmpipe's id does not fit a PCI id");
    assert_eq!((devices[0].integrated, devices[1].integrated), (Some(false), Some(true)));
    assert_eq!(devices[1].name, "AMD Ryzen 5 7600 (RADV RAPHAEL_MENDOCINO)");
}

#[test]
fn smooth_motion_cards_and_proton_features() {
    let card = |name: &str| Card {
        slot: String::new(),
        vendor_id: NVIDIA,
        device_id: 0,
        name: name.into(),
        driver: "nvidia".into(),
        primary: true,
        integrated: None,
    };
    assert_eq!(card("NVIDIA GeForce RTX 5070").smooth_motion_capable(), Some(true));
    assert_eq!(card("NVIDIA GeForce RTX 4060 Laptop GPU").smooth_motion_capable(), Some(true));
    assert_eq!(card("NVIDIA GeForce RTX 3080").smooth_motion_capable(), Some(false));
    assert_eq!(card("NVIDIA RTX A4000").smooth_motion_capable(), None);
    let ge = FakeProbe::new("/h").with_file("/p/proton", b"check_environment(\"PROTON_DLSS_UPGRADE\", \"dlss\")\nDXVK_NVAPI_VKREFLEX");
    let f = proton_features("/p/proton", &ge);
    assert!(f.upgrades && f.reflex);
    assert_eq!(proton_features("/valve/proton", &ge), Default::default());
}
