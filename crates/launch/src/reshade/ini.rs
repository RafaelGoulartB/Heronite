//! The INI dialect of ReShade and its catalogs: `[section]` headers,
//! `key=value` lines, `#`/`;` comments, keys before any section (presets
//! keep `Techniques` there). Sections and keys compare without case, like
//! ReShade's own `ini_file`.
//!
//! Edits keep every other line as it was (order, comments, unknown keys):
//! ReShade writes window state and options into `ReShade.ini`, and the
//! user's changes must survive Heronite's.

/// One line of the file, kept as written.
#[derive(Debug, Clone)]
enum Line {
    Section(String),
    Pair { key: String, value: String },
    Other(String),
}

#[derive(Debug, Clone, Default)]
pub struct Ini {
    lines: Vec<Line>,
    crlf: bool,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let lines = text
            .lines()
            .map(|raw| {
                let line = raw.trim();
                if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                    return Line::Section(name.trim().to_string());
                }
                if line.starts_with(['#', ';']) {
                    return Line::Other(raw.to_string());
                }
                match line.split_once('=') {
                    Some((k, v)) if !k.trim().is_empty() => Line::Pair { key: k.trim().to_string(), value: v.trim().to_string() },
                    _ => Line::Other(raw.to_string()),
                }
            })
            .collect();
        Self { lines, crlf: text.contains("\r\n") }
    }

    /// Indices of the lines of `section` (`""`: before the first header),
    /// without its header.
    fn range(&self, section: &str) -> Option<std::ops::Range<usize>> {
        let start = match section {
            "" => 0,
            _ => self.lines.iter().position(|l| matches!(l, Line::Section(s) if same(s, section)))? + 1,
        };
        let end = self.lines[start..].iter().position(|l| matches!(l, Line::Section(_))).map_or(self.lines.len(), |i| start + i);
        Some(start..end)
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let range = self.range(section)?;
        self.lines[range].iter().find_map(|l| match l {
            Line::Pair { key: k, value } if same(k, key) => Some(value.as_str()),
            _ => None,
        })
    }

    /// Section names in file order (the keys before any header are not one).
    pub fn sections(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|l| match l {
                Line::Section(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Sets `key` (replacing its value, else adding it at the end of the
    /// section, else adding the section at the end of the file).
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let pair = Line::Pair { key: key.to_string(), value: value.to_string() };
        let Some(range) = self.range(section) else {
            if !self.lines.is_empty() && !matches!(self.lines.last(), Some(Line::Other(l)) if l.trim().is_empty()) {
                self.lines.push(Line::Other(String::new()));
            }
            self.lines.push(Line::Section(section.to_string()));
            self.lines.push(pair);
            return;
        };
        if let Some(i) = range.clone().find(|i| matches!(&self.lines[*i], Line::Pair { key: k, .. } if same(k, key))) {
            self.lines[i] = pair;
            return;
        }
        // After the last key, before the blank lines that separate sections.
        let at =
            range.clone().rev().find(|i| !matches!(&self.lines[*i], Line::Other(l) if l.trim().is_empty())).map_or(range.start, |i| i + 1);
        self.lines.insert(at, pair);
    }

    /// Sets `key` only when the section does not have it. `true`: added.
    pub fn set_missing(&mut self, section: &str, key: &str, value: &str) -> bool {
        if self.get(section, key).is_some() {
            return false;
        }
        self.set(section, key, value);
        true
    }

    pub fn to_text(&self) -> String {
        let eol = if self.crlf { "\r\n" } else { "\n" };
        let mut out = String::new();
        for l in &self.lines {
            match l {
                Line::Section(s) => out.push_str(&format!("[{s}]")),
                Line::Pair { key, value } => out.push_str(&format!("{key}={value}")),
                Line::Other(raw) => out.push_str(raw),
            }
            out.push_str(eol);
        }
        out
    }
}

/// A comma-separated value, trimmed, without empty items.
pub fn list(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_global_keys_and_comments() {
        let ini = Ini::parse("\u{feff}Techniques=A@A.fx\n# note\n[GENERAL]\nPresetPath = .\\x.ini\n\n[input]\nKeyOverlay=36,0,0,0\n");
        assert_eq!(ini.get("", "techniques"), Some("A@A.fx"));
        assert_eq!(ini.get("general", "PRESETPATH"), Some(".\\x.ini"));
        assert_eq!(ini.get("INPUT", "KeyOverlay"), Some("36,0,0,0"));
        assert_eq!(ini.get("INPUT", "Missing"), None);
        assert_eq!(ini.sections(), ["GENERAL", "input"]);
    }

    #[test]
    fn edits_keep_order_comments_and_unknown_keys() {
        let text = "[GENERAL]\n; mine\nPresetPath=.\\a.ini\nUnknown=1\n\n[OVERLAY]\nWindow=x\n";
        let mut ini = Ini::parse(text);
        assert!(!ini.set_missing("GENERAL", "presetpath", ".\\b.ini"));
        assert!(ini.set_missing("GENERAL", "EffectSearchPaths", ".\\reshade-shaders\\Shaders\\**"));
        assert!(ini.set_missing("INPUT", "KeyOverlay", "36,0,0,0"));
        ini.set("OVERLAY", "Window", "y");
        assert_eq!(
            ini.to_text(),
            "[GENERAL]\n; mine\nPresetPath=.\\a.ini\nUnknown=1\nEffectSearchPaths=.\\reshade-shaders\\Shaders\\**\n\n[OVERLAY]\nWindow=y\n\n[INPUT]\nKeyOverlay=36,0,0,0\n"
        );
    }

    #[test]
    fn keeps_windows_line_endings() {
        let mut ini = Ini::parse("[A]\r\nk=1\r\n");
        ini.set("A", "j", "2");
        assert_eq!(ini.to_text(), "[A]\r\nk=1\r\nj=2\r\n");
        assert_eq!(list(" a.fx, ,b.fx,"), ["a.fx", "b.fx"]);
    }
}
