//! Emulator model (`local_library/emulators.json`), the built-in catalog
//! (port of `emulation/catalog.ts`, only what launching needs) and
//! argument-template expansion (port of `emulation/variables.ts`).

use heronite_data::models::Extra;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorsFile {
    #[serde(default)]
    pub emulators: Vec<UserEmulator>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEmulator {
    pub id: String,
    #[serde(default)]
    pub definition_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub executable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    /// 'native' | 'flatpak' | 'wine'
    #[serde(default)]
    pub launch_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flatpak_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playnite_id: Option<String>,
    #[serde(default)]
    pub profiles: Vec<UserEmulatorProfile>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEmulatorProfile {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_profile_id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_name: Option<String>,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_indicator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_extensions: Option<Vec<String>>,
    /// 'emulator' | 'rom'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl EmulatorsFile {
    pub fn get(&self, id: &str) -> Option<&UserEmulator> {
        self.emulators.iter().find(|e| e.id == id)
    }
}

impl UserEmulator {
    /// `getUserProfile`: by id, then by catalog profile id, then the first.
    pub fn profile(&self, profile_id: &str) -> Option<&UserEmulatorProfile> {
        self.profiles
            .iter()
            .find(|p| p.id == profile_id)
            .or_else(|| self.profiles.iter().find(|p| p.definition_profile_id.as_deref() == Some(profile_id)))
            .or_else(|| self.profiles.first())
    }
}

/// What `resolveProfileTemplate` yields that launching needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileTemplate {
    pub arguments: String,
    pub working_dir: Option<String>,
    pub core_path: Option<String>,
}

pub fn resolve_profile_template(emulator: &UserEmulator, profile: &UserEmulatorProfile) -> ProfileTemplate {
    let catalog = profile.definition_profile_id.as_deref().and_then(|pid| definition_profile(&emulator.definition_id, pid));
    ProfileTemplate {
        arguments: if !profile.arguments.is_empty() {
            profile.arguments.clone()
        } else {
            catalog.map(|c| c.arguments.to_string()).unwrap_or_else(|| "{ImagePath}".to_string())
        },
        working_dir: profile.working_dir.clone().or_else(|| catalog.and_then(|c| c.working_dir.map(str::to_string))),
        core_path: profile.core_path.clone(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogProfile {
    pub id: &'static str,
    pub arguments: &'static str,
    pub working_dir: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogEmulator {
    pub id: &'static str,
    pub flatpak_id: Option<&'static str>,
    pub stop_pattern: Option<&'static str>,
    pub profiles: &'static [CatalogProfile],
}

const fn p(id: &'static str, arguments: &'static str) -> CatalogProfile {
    CatalogProfile { id, arguments, working_dir: None }
}

const RA: &str = "-L {CorePath} {ImagePath}";

static CATALOG: &[CatalogEmulator] = &[
    CatalogEmulator {
        id: "dolphin",
        flatpak_id: Some("org.DolphinEmu.dolphin-emu"),
        stop_pattern: Some("dolphin-emu"),
        profiles: &[p("gamecube", "--exec={ImagePath} --batch"), p("wii", "--exec={ImagePath} --batch")],
    },
    CatalogEmulator {
        id: "pcsx2",
        flatpak_id: Some("net.pcsx2.PCSX2"),
        stop_pattern: Some("pcsx2"),
        profiles: &[p("ps2", "-fullscreen -slowboot -- {ImagePath}")],
    },
    CatalogEmulator {
        id: "duckstation",
        flatpak_id: Some("org.duckstation.DuckStation"),
        stop_pattern: Some("duckstation"),
        profiles: &[p("ps1", "-batch -fullscreen {ImagePath}")],
    },
    CatalogEmulator {
        id: "ppsspp",
        flatpak_id: Some("org.ppsspp.PPSSPP"),
        stop_pattern: Some("PPSSPP"),
        profiles: &[p("psp", "{ImagePath}")],
    },
    CatalogEmulator { id: "rpcs3", flatpak_id: Some("net.rpcs3.RPCS3"), stop_pattern: Some("rpcs3"), profiles: &[p("ps3", "{ImagePath}")] },
    CatalogEmulator {
        id: "cemu",
        flatpak_id: Some("info.cemu.Cemu"),
        stop_pattern: Some("Cemu"),
        profiles: &[p("wiiu", "-g {ImagePath} -f")],
    },
    CatalogEmulator {
        id: "ryujinx",
        flatpak_id: Some("org.ryujinx.Ryujinx"),
        stop_pattern: Some("Ryujinx"),
        profiles: &[p("switch", "{ImagePath}")],
    },
    CatalogEmulator {
        id: "xemu",
        flatpak_id: Some("app.xemu.xemu"),
        stop_pattern: Some("xemu"),
        profiles: &[p("xbox", "-full-screen -dvd_path {ImagePath}")],
    },
    CatalogEmulator {
        id: "melonds",
        flatpak_id: Some("net.kuribo64.melonDS"),
        stop_pattern: Some("melonDS"),
        profiles: &[p("nds", "{ImagePath}")],
    },
    CatalogEmulator {
        id: "mgba",
        flatpak_id: Some("io.mgba.mGBA"),
        stop_pattern: Some("mgba"),
        profiles: &[p("gba", "{ImagePath}"), p("gb", "{ImagePath}")],
    },
    CatalogEmulator {
        id: "flycast",
        flatpak_id: Some("org.flycast.Flycast"),
        stop_pattern: Some("flycast"),
        profiles: &[p("dreamcast", "{ImagePath}")],
    },
    CatalogEmulator {
        id: "mame",
        flatpak_id: Some("org.mamedev.MAME"),
        stop_pattern: Some("mame"),
        profiles: &[CatalogProfile { id: "arcade", arguments: "{ImageNameNoExt}", working_dir: Some("rom") }],
    },
    CatalogEmulator {
        id: "scummvm",
        flatpak_id: Some("org.scummvm.ScummVM"),
        stop_pattern: Some("scummvm"),
        profiles: &[p("scumm", "-f {ImagePath}")],
    },
    CatalogEmulator {
        id: "vita3k",
        flatpak_id: Some("org.vita3k.Vita3K"),
        stop_pattern: Some("Vita3K"),
        profiles: &[p("vita", "{ImagePath}")],
    },
    CatalogEmulator {
        id: "shadps4",
        flatpak_id: Some("net.shadps4.shadPS4"),
        stop_pattern: Some("shadps4"),
        profiles: &[p("ps4", "{ImagePath}")],
    },
    CatalogEmulator {
        id: "azahar",
        flatpak_id: Some("org.azahar_emu.Azahar"),
        stop_pattern: Some("azahar"),
        profiles: &[p("3ds", "{ImagePath}")],
    },
    CatalogEmulator {
        id: "retroarch",
        flatpak_id: Some("org.libretro.RetroArch"),
        stop_pattern: Some("retroarch"),
        profiles: &[
            p("nes", RA),
            p("snes", RA),
            p("gb", RA),
            p("gba", RA),
            p("n64", RA),
            p("nds", RA),
            p("md", RA),
            p("ms", RA),
            p("gg", RA),
            p("saturn", RA),
            p("dc", RA),
            p("ps1", RA),
            p("psp", RA),
            p("pce", RA),
            p("ngp", RA),
            p("wswan", RA),
            p("lynx", RA),
            p("a2600", RA),
            p("arcade", RA),
            p("mame", RA),
        ],
    },
    CatalogEmulator { id: "custom", flatpak_id: None, stop_pattern: None, profiles: &[p("custom", "{ImagePath}")] },
];

pub fn emulator_definition(id: &str) -> Option<&'static CatalogEmulator> {
    CATALOG.iter().find(|d| d.id == id)
}

pub fn definition_profile(definition_id: &str, profile_id: &str) -> Option<&'static CatalogProfile> {
    emulator_definition(definition_id)?.profiles.iter().find(|p| p.id == profile_id)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchVars {
    pub image_path: Option<String>,
    pub image_name: Option<String>,
    pub image_name_no_ext: Option<String>,
    pub image_dir: Option<String>,
    pub emulator_dir: Option<String>,
    pub core_path: Option<String>,
    pub name: Option<String>,
    pub install_dir: Option<String>,
}

/// Node's `path.dirname` (POSIX).
#[cfg(not(windows))]
pub fn node_dirname(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".into();
    }
    match trimmed.rfind('/') {
        None => ".".into(),
        Some(i) => {
            let dir = trimmed[..i].trim_end_matches('/');
            if dir.is_empty() {
                "/".into()
            } else {
                dir.into()
            }
        }
    }
}

/// Node's `path.basename` (POSIX).
#[cfg(not(windows))]
pub fn node_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or("").to_string()
}

/// Node's `path.dirname` on Windows (`path.win32`): `\` and `/` both
/// separate, and a drive (`C:\`) or share root is its own folder.
#[cfg(windows)]
pub fn node_dirname(path: &str) -> String {
    match std::path::Path::new(path).parent() {
        Some(dir) if dir.as_os_str().is_empty() => ".".into(),
        Some(dir) => dir.to_string_lossy().into_owned(),
        None if path.is_empty() => ".".into(),
        // A root (`C:\`, `\`, `\\server\share`).
        None => path.into(),
    }
}

/// Node's `path.basename` on Windows (`path.win32`): either separator, and
/// no drive (`C:x.iso` is `x.iso`).
#[cfg(windows)]
pub fn node_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    let b = last.as_bytes();
    let drive = last.len() == trimmed.len() && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    if drive { &last[2..] } else { last }.to_string()
}

fn node_extname(base: &str) -> &str {
    match base.rfind('.') {
        Some(0) | None => "",
        Some(i) => &base[i..],
    }
}

pub fn vars_for_rom(rom_path: &str, emulator_dir: Option<&str>, core_path: Option<&str>, name: &str) -> LaunchVars {
    let file = node_basename(rom_path);
    let ext = node_extname(&file);
    let emulator_dir = emulator_dir.map(str::to_string);
    LaunchVars {
        image_path: Some(rom_path.to_string()),
        image_name: Some(file.clone()),
        image_name_no_ext: Some(file[..file.len() - ext.len()].to_string()),
        image_dir: Some(node_dirname(rom_path)),
        emulator_dir: emulator_dir.clone(),
        core_path: core_path.map(str::to_string),
        name: Some(name.to_string()),
        install_dir: emulator_dir,
    }
}

pub fn expand_template(value: &str, vars: &LaunchVars) -> String {
    let pairs: [(&str, &Option<String>); 8] = [
        ("ImagePath", &vars.image_path),
        ("ImageName", &vars.image_name),
        ("ImageNameNoExt", &vars.image_name_no_ext),
        ("ImageDir", &vars.image_dir),
        ("EmulatorDir", &vars.emulator_dir),
        ("CorePath", &vars.core_path),
        ("Name", &vars.name),
        ("InstallDir", &vars.install_dir),
    ];
    let mut result = value.to_string();
    for (key, replacement) in pairs {
        match replacement {
            Some(r) if !r.is_empty() => result = result.replace(&format!("{{{key}}}"), r),
            _ => {}
        }
    }
    result
}

/// shlex-splits the template, expands each part and drops empty ones.
pub fn expand_argv(template: &str, vars: &LaunchVars) -> Vec<String> {
    let source = template.trim();
    if source.is_empty() {
        return Vec::new();
    }
    let parts = shlex::split(source).unwrap_or_else(|| source.split_whitespace().map(str::to_string).collect());
    parts.into_iter().map(|p| expand_template(&p, vars)).filter(|p| !p.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_path_helpers_match_node() {
        assert_eq!(node_dirname("/a/b/c.iso"), "/a/b");
        assert_eq!(node_dirname("/a"), "/");
        assert_eq!(node_dirname("c.iso"), ".");
        assert_eq!(node_dirname(""), ".");
        assert_eq!(node_dirname("/a/b/"), "/a");
        assert_eq!(node_basename("/a/b/c.iso"), "c.iso");
        assert_eq!(node_extname("c.tar.gz"), ".gz");
        assert_eq!(node_extname(".hidden"), "");
        assert_eq!(node_extname("a."), ".");
    }

    #[cfg(windows)]
    #[test]
    fn node_path_helpers_match_node_on_windows() {
        assert_eq!(node_dirname(r"C:\Emu\PCSX2\pcsx2-qt.exe"), r"C:\Emu\PCSX2");
        assert_eq!(node_dirname("C:/Emu/pcsx2-qt.exe"), "C:/Emu");
        assert_eq!(node_dirname(r"C:\pcsx2-qt.exe"), r"C:\");
        assert_eq!(node_dirname(r"C:\"), r"C:\");
        assert_eq!(node_dirname(r"\\nas\roms\ps2\a.iso"), r"\\nas\roms\ps2");
        assert_eq!(node_dirname("a.iso"), ".");
        assert_eq!(node_basename(r"C:\Roms\Zelda (USA).sfc"), "Zelda (USA).sfc");
        assert_eq!(node_basename(r"C:\Roms/ps2\a.iso"), "a.iso");
        assert_eq!(node_basename(r"C:\Roms\"), "Roms");
        assert_eq!(node_basename("C:a.iso"), "a.iso");
        let v = vars_for_rom(r"D:\Roms\gba\Advance Wars.gba", Some(r"C:\mGBA"), None, "Advance Wars");
        assert_eq!(v.image_dir.as_deref(), Some(r"D:\Roms\gba"));
        assert_eq!(v.image_name_no_ext.as_deref(), Some("Advance Wars"));
    }

    #[test]
    fn expands_variables_like_heroic() {
        let v = vars_for_rom("/roms/Mario Kart (USA).iso", Some("/emu"), Some("/cores/x.so"), "Mario Kart");
        assert_eq!(v.image_name_no_ext.as_deref(), Some("Mario Kart (USA)"));
        assert_eq!(
            expand_argv("-L {CorePath} \"{ImagePath}\" --dir={ImageDir} {Missing}", &v),
            vec!["-L", "/cores/x.so", "/roms/Mario Kart (USA).iso", "--dir=/roms", "{Missing}"]
        );
        // Empty variables are left alone; parts that become empty are dropped.
        let none = vars_for_rom("/roms/a.iso", None, None, "A");
        assert_eq!(expand_argv("-L {CorePath} {ImagePath}", &none), vec!["-L", "{CorePath}", "/roms/a.iso"]);
        assert!(expand_argv("   ", &none).is_empty());
    }

    #[test]
    fn catalog_lookups() {
        assert_eq!(emulator_definition("pcsx2").unwrap().stop_pattern, Some("pcsx2"));
        assert_eq!(definition_profile("retroarch", "snes").unwrap().arguments, RA);
        assert_eq!(definition_profile("mame", "arcade").unwrap().working_dir, Some("rom"));
        assert!(emulator_definition("custom").unwrap().stop_pattern.is_none());
    }
}
