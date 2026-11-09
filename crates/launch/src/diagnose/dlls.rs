//! Which winetricks component brings a DLL a game needs.
//!
//! Only the runtimes games ship without and Wine has no builtin copy of that
//! works for every game. Every verb here is in winetricks' own list.

/// The winetricks verb that installs `dll` (any case, with or without
/// `.dll`). `None`: no component brings it (the game's own file, or one
/// Wine provides).
pub fn verb_for_dll(dll: &str) -> Option<&'static str> {
    let lower = dll.to_ascii_lowercase();
    let name = lower.strip_suffix(".dll").unwrap_or(&lower);
    let exact = match name {
        // Visual C++ 2015-2022 share one redistributable.
        "vcruntime140"
        | "vcruntime140_1"
        | "msvcp140"
        | "msvcp140_1"
        | "msvcp140_2"
        | "msvcp140_atomic_wait"
        | "msvcp140_codecvt_ids"
        | "concrt140"
        | "vccorlib140"
        | "vcomp140"
        | "vcamp140"
        | "mfc140"
        | "mfc140u"
        | "mfcm140"
        | "mfcm140u" => "vcrun2022",
        "msvcr120" | "msvcp120" | "vcomp120" | "vccorlib120" | "mfc120" | "mfc120u" => "vcrun2013",
        "msvcr110" | "msvcp110" | "vcomp110" | "mfc110" | "mfc110u" => "vcrun2012",
        "msvcr100" | "msvcp100" | "vcomp100" | "mfc100" | "mfc100u" => "vcrun2010",
        "msvcr90" | "msvcp90" | "mfc90" | "mfc90u" => "vcrun2008",
        "msvcr80" | "msvcp80" | "mfc80" | "mfc80u" => "vcrun2005",
        "msvcr71" | "msvcp71" | "mfc71" | "mfc71u" => "vcrun2003",
        "msvcp60" | "mfc42" | "mfc42u" => "mfc42",
        "msvbvm60" => "vb6run",
        "d3dcompiler_43" => "d3dcompiler_43",
        "d3dcompiler_46" | "d3dcompiler_47" => "d3dcompiler_47",
        "d3dx11_42" | "d3dx11_43" => "d3dx11_43",
        "physxloader" | "physxcore" | "physxcooking" | "physxdevice" | "physxextensions" | "cudart32_30_9" | "cudart64_30_9" => "physx",
        "openal32" | "wrap_oal" => "openal",
        "xnafx40_redist" => "xna40",
        _ => "",
    };
    if !exact.is_empty() {
        return Some(exact);
    }
    if name.starts_with("api-ms-win-crt-") || name == "ucrtbase" {
        return Some("vcrun2022");
    }
    if name.starts_with("d3dx9_") {
        return Some("d3dx9");
    }
    if name.starts_with("d3dx10_") {
        return Some("d3dx10");
    }
    // XAudio 2.0-2.7, X3DAudio and XAPOFX come with the DirectX runtime's
    // XACT; 2.8 and 2.9 are part of Windows (and Wine).
    if ["xactengine", "x3daudio", "xapofx"].iter().any(|p| name.starts_with(p))
        || name.strip_prefix("xaudio2_").is_some_and(|v| v.parse::<u8>().is_ok_and(|n| n <= 7))
    {
        return Some("xact");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::verb_for_dll;

    #[test]
    fn runtimes_map_to_their_redistributables() {
        assert_eq!(verb_for_dll("MSVCP140.dll"), Some("vcrun2022"));
        assert_eq!(verb_for_dll("VCRUNTIME140_1.dll"), Some("vcrun2022"));
        assert_eq!(verb_for_dll("api-ms-win-crt-runtime-l1-1-0.dll"), Some("vcrun2022"));
        assert_eq!(verb_for_dll("msvcr120.dll"), Some("vcrun2013"));
        assert_eq!(verb_for_dll("MSVCR100"), Some("vcrun2010"));
        assert_eq!(verb_for_dll("d3dx9_43.dll"), Some("d3dx9"));
        assert_eq!(verb_for_dll("D3DX10_43.DLL"), Some("d3dx10"));
        assert_eq!(verb_for_dll("d3dcompiler_47.dll"), Some("d3dcompiler_47"));
        assert_eq!(verb_for_dll("XAudio2_7.dll"), Some("xact"));
        assert_eq!(verb_for_dll("x3daudio1_7.dll"), Some("xact"));
        assert_eq!(verb_for_dll("PhysXLoader.dll"), Some("physx"));
    }

    #[test]
    fn dlls_of_wine_or_of_the_game_have_no_verb() {
        assert_eq!(verb_for_dll("xaudio2_9.dll"), None);
        assert_eq!(verb_for_dll("kernel32.dll"), None);
        assert_eq!(verb_for_dll("steam_api64.dll"), None);
        assert_eq!(verb_for_dll("UnityPlayer.dll"), None);
    }
}
