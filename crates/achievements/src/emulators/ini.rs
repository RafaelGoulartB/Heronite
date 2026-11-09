//! A line editor for the INI / cfg files of the emulators: `[Section]` headers
//! and `key = value` lines (spaces around `=` optional), or the flat
//! `key = "value"` file of RetroArch (section `""`).
//!
//! Only the lines we touch change; every other line, comment, blank line, the
//! order and the line endings stay as they are.

/// The top of the file, before any `[Section]` header.
pub const FLAT: &str = "";

#[derive(Debug, Clone, PartialEq)]
struct Line {
    text: String,
    /// `"\n"`, `"\r\n"` or `""` (the last line of a file without a final newline).
    eol: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ini {
    lines: Vec<Line>,
    /// The ending of new lines: the first one the file uses.
    eol: String,
}

enum Kind<'a> {
    Section(&'a str),
    Entry(&'a str),
    Other,
}

fn classify(text: &str) -> Kind<'_> {
    let t = text.trim();
    if t.is_empty() || t.starts_with(';') || t.starts_with('#') {
        return Kind::Other;
    }
    if let Some(rest) = t.strip_prefix('[') {
        return match rest.find(']') {
            Some(end) => Kind::Section(&rest[..end]),
            None => Kind::Other,
        };
    }
    match t.split_once('=') {
        Some((key, _)) => Kind::Entry(key.trim()),
        None => Kind::Other,
    }
}

/// The value of an entry line: trimmed, without one pair of surrounding quotes.
fn value_of(text: &str) -> &str {
    let value = text.split_once('=').map_or("", |(_, v)| v).trim();
    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value)
}

impl Ini {
    pub fn parse(text: &str) -> Ini {
        let mut lines = Vec::new();
        let mut eol = None;
        let mut rest = text;
        while !rest.is_empty() {
            let (body, tail) = match rest.find('\n') {
                Some(at) => (&rest[..at], &rest[at + 1..]),
                None => (rest, ""),
            };
            let had_newline = body.len() + 1 + tail.len() == rest.len();
            let (text, line_eol) = match (had_newline, body.strip_suffix('\r')) {
                (true, Some(t)) => (t, "\r\n"),
                (true, None) => (body, "\n"),
                (false, _) => (body, ""),
            };
            if eol.is_none() && !line_eol.is_empty() {
                eol = Some(line_eol.to_string());
            }
            lines.push(Line { text: text.to_string(), eol: line_eol.to_string() });
            rest = tail;
        }
        Ini { lines, eol: eol.unwrap_or_else(|| "\n".to_string()) }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(&line.text);
            out.push_str(&line.eol);
        }
        out
    }

    /// The index range of the lines in `section` (after its header).
    fn range(&self, section: &str) -> Option<std::ops::Range<usize>> {
        if section == FLAT {
            let end = self.lines.iter().position(|l| matches!(classify(&l.text), Kind::Section(_)));
            return Some(0..end.unwrap_or(self.lines.len()));
        }
        let start = self.lines.iter().position(|l| matches!(classify(&l.text), Kind::Section(s) if s == section))? + 1;
        let len =
            self.lines[start..].iter().position(|l| matches!(classify(&l.text), Kind::Section(_))).unwrap_or(self.lines.len() - start);
        Some(start..start + len)
    }

    /// The last value of `key` in `section`.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let range = self.range(section)?;
        self.lines[range]
            .iter()
            .rev()
            .find(|l| matches!(classify(&l.text), Kind::Entry(k) if k == key))
            .map(|l| value_of(&l.text).to_string())
    }

    pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        match self.get(section, key)?.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Some(true),
            "false" | "no" | "0" | "off" => Some(false),
            _ => None,
        }
    }

    /// Sets `key` (every copy of it) or adds it at the end of the section,
    /// adding the section at the end of the file when it is missing.
    /// `quote` writes `key = "value"`.
    pub fn set(&mut self, section: &str, key: &str, value: &str, quote: bool) {
        let value = if quote { format!("\"{value}\"") } else { value.to_string() };
        let Some(range) = self.range(section) else {
            self.append_section(section);
            return self.set(section, key, &value, false);
        };
        let mut found = false;
        for line in &mut self.lines[range.clone()] {
            if matches!(classify(&line.text), Kind::Entry(k) if k == key) {
                line.text = replace_value(&line.text, &value);
                found = true;
            }
        }
        if found {
            return;
        }
        // After the last entry, so the comments and blank lines that lead
        // to the next header stay with it.
        let lines = &self.lines[range.clone()];
        let last = |pred: &dyn Fn(&Line) -> bool| lines.iter().rposition(pred).map(|i| range.start + i + 1);
        let at = last(&|l| matches!(classify(&l.text), Kind::Entry(_))).unwrap_or_else(|| {
            if section == FLAT {
                last(&|l| !l.text.trim().is_empty()).unwrap_or(range.start)
            } else {
                range.start
            }
        });
        self.insert(at, format!("{key} = {value}"));
    }

    fn append_section(&mut self, section: &str) {
        if !self.lines.last().is_none_or(|l| l.text.trim().is_empty()) {
            self.insert(self.lines.len(), String::new());
        }
        self.insert(self.lines.len(), format!("[{section}]"));
    }

    fn insert(&mut self, at: usize, text: String) {
        // A last line without a newline gets one before anything follows it.
        if at > 0 && self.lines[at - 1].eol.is_empty() {
            self.lines[at - 1].eol = self.eol.clone();
        }
        self.lines.insert(at, Line { text, eol: self.eol.clone() });
    }
}

/// `line` with the part after `=` replaced, keeping the key, the spacing
/// around `=` and the indentation.
fn replace_value(line: &str, value: &str) -> String {
    let eq = line.find('=').unwrap_or(line.len().saturating_sub(1));
    let after = &line[eq + 1..];
    let spaces = after.len() - after.trim_start().len();
    format!("{}{}", &line[..eq + 1 + spaces], value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "; top comment\n[Achievements]\nEnabled = false\nUsername=old\n\n# keep\n[Other]\nKey = 1\n";

    #[test]
    fn untouched_files_render_identically() {
        for text in [SAMPLE, "", "a = 1", "[A]\r\nx = 1\r\n\r\n", "\n\n[A]\n"] {
            assert_eq!(Ini::parse(text).render(), text);
        }
    }

    #[test]
    fn reads_values_with_or_without_spaces_and_quotes() {
        let ini = Ini::parse("flat = \"q\"\n[A]\nx=1\ny = 2 \nz = \"three\"\n; w = 4\n");
        assert_eq!(ini.get(FLAT, "flat").as_deref(), Some("q"));
        assert_eq!(ini.get("A", "x").as_deref(), Some("1"));
        assert_eq!(ini.get("A", "y").as_deref(), Some("2"));
        assert_eq!(ini.get("A", "z").as_deref(), Some("three"));
        assert_eq!(ini.get("A", "w"), None);
        assert_eq!(ini.get("B", "x"), None);
        assert_eq!(ini.get(FLAT, "x"), None);
    }

    #[test]
    fn set_changes_only_that_line_and_keeps_its_spacing() {
        let mut ini = Ini::parse(SAMPLE);
        ini.set("Achievements", "Enabled", "true", false);
        ini.set("Achievements", "Username", "me", false);
        assert_eq!(ini.render(), "; top comment\n[Achievements]\nEnabled = true\nUsername=me\n\n# keep\n[Other]\nKey = 1\n");
    }

    #[test]
    fn set_adds_a_key_before_the_blank_lines_that_end_the_section() {
        let mut ini = Ini::parse(SAMPLE);
        ini.set("Achievements", "Token", "abc", false);
        assert_eq!(ini.render(), "; top comment\n[Achievements]\nEnabled = false\nUsername=old\nToken = abc\n\n# keep\n[Other]\nKey = 1\n");
    }

    #[test]
    fn set_adds_a_missing_section_at_the_end() {
        let mut ini = Ini::parse("[Other]\nKey = 1");
        ini.set("New", "A", "b", false);
        assert_eq!(ini.render(), "[Other]\nKey = 1\n\n[New]\nA = b\n");
        let mut empty = Ini::parse("");
        empty.set("New", "A", "b", false);
        assert_eq!(empty.render(), "[New]\nA = b\n");
    }

    #[test]
    fn flat_keys_go_before_the_first_section() {
        let mut ini = Ini::parse("a = \"1\"\n[S]\nb = 2\n");
        ini.set(FLAT, "c", "3", true);
        ini.set(FLAT, "a", "9", true);
        assert_eq!(ini.render(), "a = \"9\"\nc = \"3\"\n[S]\nb = 2\n");
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let mut ini = Ini::parse("[A]\r\nx = 1\r\n");
        ini.set("A", "x", "2", false);
        ini.set("A", "y", "3", false);
        ini.set("B", "z", "4", false);
        assert_eq!(ini.render(), "[A]\r\nx = 2\r\ny = 3\r\n\r\n[B]\r\nz = 4\r\n");
    }

    #[test]
    fn quoted_values_and_empty_values() {
        let mut ini = Ini::parse("cheevos_token = \"old\"\n");
        ini.set(FLAT, "cheevos_token", "", true);
        assert_eq!(ini.render(), "cheevos_token = \"\"\n");
        assert_eq!(ini.get(FLAT, "cheevos_token").as_deref(), Some(""));
    }

    #[test]
    fn bools_read_the_spellings_of_every_emulator() {
        let ini = Ini::parse("[A]\na = True\nb = no\nc = false\nd = yes\ne = x\n");
        assert_eq!(ini.get_bool("A", "a"), Some(true));
        assert_eq!(ini.get_bool("A", "b"), Some(false));
        assert_eq!(ini.get_bool("A", "c"), Some(false));
        assert_eq!(ini.get_bool("A", "d"), Some(true));
        assert_eq!(ini.get_bool("A", "e"), None);
        assert_eq!(ini.get_bool("A", "f"), None);
    }
}
