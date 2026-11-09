//! Windows resources of `heronite-rs.exe`: the icon (Explorer, the title bar
//! and the taskbar) and the version information (file properties, the
//! uninstall entry, SmartScreen). Nothing on other systems.
//!
//! GPUI's Windows platform loads the window class icon from resource id 1 of
//! the program (`LoadImageW(module, 1, IMAGE_ICON)`), so the icon is `1`.
//! GPUI already embeds the application manifest (DPI awareness, common
//! controls), so this file has none. `packaging/windows/heronite.ico` is made
//! by `scripts/windows/make-icon.py`.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let icon = Path::new(&manifest_dir).join("../packaging/windows/heronite.ico");
    let icon = icon.canonicalize().unwrap_or_else(|e| panic!("{}: {e}", icon.display()));
    println!("cargo:rerun-if-changed={}", icon.display());

    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let rc = Path::new(&out_dir).join("heronite.rc");
    std::fs::write(&rc, resource_script(&version, &icon)).expect("writing heronite.rc");

    embed_resource::compile(&rc, embed_resource::NONE).manifest_required().unwrap_or_else(|e| panic!("heronite.rc: {e}"));
}

/// `0.18.0` -> `0,18,0,0`; a pre-release or build suffix (`1.2.3-rc.1`) is
/// left out of the numeric version, the text versions keep it.
fn numeric_version(version: &str) -> String {
    let mut parts: Vec<u16> = version.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).take(4).collect();
    parts.resize(4, 0);
    parts.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
}

fn resource_script(version: &str, icon: &Path) -> String {
    // The resource compiler reads backslashes in strings as escapes; forward
    // slashes work as path separators and need none. `\\?\` (from
    // canonicalize) is not a path it understands.
    let icon = icon.display().to_string();
    let icon = icon.strip_prefix(r"\\?\").unwrap_or(&icon).replace('\\', "/");
    let numeric = numeric_version(version);
    format!(
        r#"#pragma code_page(65001)
#include <winver.h>

1 ICON "{icon}"

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
            VALUE "FileDescription", "Heronite"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "heronite-rs"
            VALUE "LegalCopyright", "GPL-3.0-only"
            VALUE "OriginalFilename", "heronite-rs.exe"
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
