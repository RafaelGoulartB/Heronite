//! The lines of a launch log that tell a known problem.
//!
//! A pattern is a list of alternatives; an alternative matches a line holding
//! all of its words (lowercase, any case in the log). Wine's thread ids
//! (`0024:err:module:...`) and DXVK's levels stay in the lines kept.

use std::sync::LazyLock;

use regex::Regex;

pub(super) type Pattern = [&'static [&'static str]];

/// Wine cannot find the program it was asked to run.
pub(super) const EXE_MISSING: &Pattern = &[&["wine: cannot find"], &["wine: failed to open"]];

/// Wine without its 32-bit half (or without the libraries of it).
pub(super) const WINE32_MISSING: &Pattern =
    &[&["wine32 is missing"], &["could not load ntdll.so"], &["cannot support 32-bit"], &["wine-preloader", "could not"]];

/// A wineserver of another Wine version owns the prefix.
pub(super) const WINESERVER: &Pattern = &[&["wine client error", "version mismatch"], &["your wine client and server"]];

/// Wine Mono missing or failing (`err:mscoree:... Wine Mono is not installed`).
pub(super) const MONO: &Pattern = &[&["wine mono is not installed"], &["err:mscoree:load_mono"], &["could not find wine mono"]];

/// Vulkan (Wine's, DXVK's or VKD3D's) could not start.
pub(super) const VULKAN: &Pattern = &[
    &["vkcreateinstance failed"],
    &["vk_error_incompatible_driver"],
    &["failed to create vulkan instance"],
    &["dxvk", "failed to create instance"],
    &["no adapters found"],
    &["vkenumeratephysicaldevices failed"],
    &["libvulkan.so.1", "cannot open shared object file"],
    &["failed to load libvulkan"],
];

pub(super) const TOO_MANY_FILES: &Pattern = &[&["too many open files"]];

/// Wine's Direct3D 12 (vkd3d) failing.
pub(super) const D3D12: &Pattern = &[&["err:d3d12"], &["err:vkd3d"], &["d3d12createdevice", "fail"]];

/// Wine's own Direct3D 9-11 (wined3d) failing.
pub(super) const WINED3D: &Pattern = &[&["err:d3d:"], &["err:d3d_caps"], &["wined3d", "err:"]];

/// Proton's own warning (written with `PROTON_LOG`).
pub(super) const MAX_MAP_COUNT: &Pattern = &[&["max_map_count"]];

/// A crash Wine reports.
pub(super) const CRASH: &Pattern = &[
    &["unhandled page fault"],
    &["unhandled exception"],
    &["unhandled illegal instruction"],
    &["unhandled stack overflow"],
    &["unhandled privileged instruction"],
];

/// How long a line may be once kept.
const LINE_MAX: usize = 240;

/// What the game itself wrote: the log after its last `Game Output:` line
/// (the whole text when the header is not in it).
pub(super) fn game_output(log: &str) -> &str {
    match log.rfind("Game Output:") {
        Some(at) => &log[at + "Game Output:".len()..],
        None => log,
    }
}

/// The launch log is off for the game (`verboseLogs` or `disableLogs`).
pub(super) fn is_disabled(log: &str) -> bool {
    log.contains("IMPORTANT: Logs are disabled")
}

fn matches(line: &str, pattern: &Pattern) -> bool {
    let lower = line.to_ascii_lowercase();
    pattern.iter().any(|words| words.iter().all(|w| lower.contains(w)))
}

pub(super) fn any(out: &str, pattern: &Pattern) -> bool {
    out.lines().any(|l| matches(l, pattern))
}

fn shorten(line: &str) -> String {
    let line = line.trim();
    match line.char_indices().nth(LINE_MAX) {
        Some((at, _)) => format!("{}\u{2026}", &line[..at]),
        None => line.to_string(),
    }
}

/// The first `max` different lines matching `pattern`.
pub(super) fn lines(out: &str, pattern: &Pattern, max: usize) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in out.lines().filter(|l| matches(l, pattern)) {
        let line = shorten(line);
        if !found.contains(&line) {
            found.push(line);
            if found.len() == max {
                break;
            }
        }
    }
    found
}

/// `err:module:import_dll Library MSVCP140.dll (which is needed by
/// L"C:\\game\\game.exe") not found`, as Wine 5 and later write it.
static IMPORT_DLL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)import_dll\s+Library\s+([^\s(]+)\s+\(which is needed by\s+L?"[^"]*"\)\s+not found"#).unwrap());

/// The DLLs the loader could not find (as written, first seen first) and
/// the line that said so.
pub(super) fn missing_dlls(out: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in out.lines() {
        let Some(c) = IMPORT_DLL.captures(line) else { continue };
        let dll = c[1].to_string();
        if !found.iter().any(|(d, _)| d.eq_ignore_ascii_case(&dll)) {
            found.push((dll, shorten(line)));
        }
    }
    found
}

/// `./game: error while loading shared libraries: libGL.so.1: cannot open
/// shared object file: No such file or directory` (the Linux loader).
static SHARED_LIB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"error while loading shared libraries:\s+([^\s:]+):\s+(cannot open shared object file|wrong ELF class: ELFCLASS(?:32|64))")
        .unwrap()
});

#[derive(Debug, Default)]
pub(super) struct MissingLibs {
    pub libs: Vec<String>,
    pub lines: Vec<String>,
    /// A 32-bit program found only the 64-bit library (`ELFCLASS64`).
    pub wrong_class: bool,
}

pub(super) fn missing_libs(out: &str) -> MissingLibs {
    let mut m = MissingLibs::default();
    for line in out.lines() {
        let Some(c) = SHARED_LIB.captures(line) else { continue };
        m.wrong_class |= c[2].ends_with("ELFCLASS64");
        let lib = c[1].to_string();
        if !m.libs.contains(&lib) {
            m.libs.push(lib);
            if m.lines.len() < super::EVIDENCE {
                m.lines.push(shorten(line));
            }
        }
    }
    m
}
