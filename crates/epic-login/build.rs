//! Windows resources of `heronite-epic-login.exe`: Heronite's icon (the
//! title bar, the taskbar and Explorer), the version information (file
//! properties, Task Manager's name for it) and the application manifest
//! (per-monitor DPI awareness, Windows 10/11, no elevation). Nothing on other
//! systems.
//!
//! The window loads its icon from resource id 1.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let icon = rc_path(&Path::new(&manifest_dir).join("../../packaging/windows/heronite.ico"));
    let manifest = rc_path(&Path::new(&manifest_dir).join("windows/login.manifest"));

    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let rc = Path::new(&out_dir).join("login.rc");
    std::fs::write(&rc, resource_script(&version, &icon, &manifest)).expect("writing login.rc");

    embed_resource::compile(&rc, embed_resource::NONE).manifest_required().unwrap_or_else(|e| panic!("login.rc: {e}"));
}

/// `path` as the resource compiler reads it: it takes backslashes in strings
/// as escapes (forward slashes work as separators) and does not understand
/// `\\?\` (from canonicalize).
fn rc_path(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("cargo:rerun-if-changed={}", path.display());
    let path = path.display().to_string();
    path.strip_prefix(r"\\?\").unwrap_or(&path).replace('\\', "/")
}

/// `0.18.0` -> `0,18,0,0`; a pre-release or build suffix is left out of the
/// numeric version, the text versions keep it.
fn numeric_version(version: &str) -> String {
    let mut parts: Vec<u16> = version.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).take(4).collect();
    parts.resize(4, 0);
    parts.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
}

fn resource_script(version: &str, icon: &str, manifest: &str) -> String {
    let numeric = numeric_version(version);
    format!(
        r#"#pragma code_page(65001)
#include <winver.h>

1 ICON "{icon}"

1 24 "{manifest}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK VS_FFI_FILEFLAGSMASK
FILEFLAGS 0
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
FILESUBTYPE VFT2_UNKNOWN
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "Heronite"
            VALUE "FileDescription", "Heronite store login"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "heronite-epic-login"
            VALUE "LegalCopyright", "GPL-3.0-only"
            VALUE "OriginalFilename", "heronite-epic-login.exe"
            VALUE "ProductName", "Heronite"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    )
}
