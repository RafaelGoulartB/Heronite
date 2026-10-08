//! ROM name cleanup (port of `emulation/names.ts`).

use regex::Regex;
use std::sync::OnceLock;

fn props_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[(.*?)\]|\((.*?)\)").unwrap())
}

fn ws_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").unwrap())
}

fn ext_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\.[^.]+$").unwrap())
}

/// `sanitizeRomName`.
pub fn sanitize_rom_name(name: &str) -> String {
    let no_props = props_re().replace_all(name, "");
    let mut s = String::with_capacity(no_props.len());
    for c in no_props.chars() {
        match c {
            '\u{2019}' | '`' => s.push('\''),
            '\u{2122}' | '\u{00AE}' | '\u{00A9}' => {}
            '_' => s.push(' '),
            c => s.push(c),
        }
    }
    ws_re().replace_all(&s, " ").trim().to_string()
}

/// `romProperties`: the comma-separated parts of every `(...)` / `[...]`.
pub fn rom_properties(original: &str) -> Vec<String> {
    let mut out = Vec::new();
    for caps in props_re().captures_iter(original) {
        let value = caps
            .get(1)
            .map(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| caps.get(2).map(|m| m.as_str()).filter(|s| !s.is_empty()));
        let Some(value) = value else { continue };
        for part in value.split(',') {
            let t = part.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
    }
    out
}

/// `romDiscLabel`: the first property that starts with disc/disk/side.
pub fn rom_disc_label(original: &str) -> Option<String> {
    rom_properties(original).into_iter().find(|p| {
        let lower = p.to_lowercase();
        ["disc", "disk", "side"].iter().any(|w| {
            lower.strip_prefix(w).is_some_and(|rest| {
                // `\b` after the word: next char must not be a word char.
                rest.chars()
                    .next()
                    .map(|c| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(true)
            })
        })
    })
}

/// `titleFromRomPath(fileName)`.
pub fn title_from_rom_path(file_name: &str) -> String {
    let base = ext_re().replace(file_name, "").to_string();
    let clean = sanitize_rom_name(&base);
    if clean.is_empty() {
        base
    } else {
        clean
    }
}

/// `fileName.replace(/\.[^.]+$/, '')`.
pub fn strip_ext(file_name: &str) -> String {
    ext_re().replace(file_name, "").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_like_ts() {
        assert_eq!(sanitize_rom_name("Tony Hawk_s Underground (USA) [!]"), "Tony Hawk s Underground");
        assert_eq!(sanitize_rom_name("Mario\u{2019}s Game\u{2122}"), "Mario's Game");
        assert_eq!(sanitize_rom_name("(USA)"), "");
    }

    #[test]
    fn properties_and_disc() {
        assert_eq!(
            rom_properties("Final Fantasy VII (USA) (Disc 1) [!]"),
            ["USA", "Disc 1", "!"]
        );
        assert_eq!(rom_properties("A (En,Fr,De)"), ["En", "Fr", "De"]);
        assert_eq!(
            rom_disc_label("Final Fantasy VII (USA) (Disc 1)").as_deref(),
            Some("Disc 1")
        );
        assert_eq!(rom_disc_label("Game (Side B)").as_deref(), Some("Side B"));
        assert_eq!(rom_disc_label("Game (Discovery)"), None);
        assert_eq!(rom_disc_label("Game (USA)"), None);
    }

    #[test]
    fn title_from_path() {
        assert_eq!(title_from_rom_path("ATV Offroad Fury 4 (USA).iso"), "ATV Offroad Fury 4");
        assert_eq!(title_from_rom_path("(USA).iso"), "(USA)");
        assert_eq!(title_from_rom_path("noext"), "noext");
        assert_eq!(title_from_rom_path("Game v1.2.zip"), "Game v1.2");
    }
}
