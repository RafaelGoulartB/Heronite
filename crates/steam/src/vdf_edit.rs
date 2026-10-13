//! In-place edits of Steam's text VDF files (`config.vdf`, `localconfig.vdf`).
//!
//! Only the bytes of the entry that changes are touched; the rest of the file
//! (thousands of lines that are not ours) stays byte for byte. New entries
//! use Steam's own layout: one tab per level, two tabs between key and value.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError(pub String);

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

fn err(text: impl Into<String>) -> EditError {
    EditError(text.into())
}

/// One `"key" value` entry with byte offsets into the text.
#[derive(Debug)]
struct Entry {
    key: String,
    /// Start of the key token.
    start: usize,
    value: Value,
}

#[derive(Debug)]
enum Value {
    /// The quoted (or bare) value token, `start..end`.
    Str { start: usize, end: usize, text: String },
    /// `close` is the offset of the closing `}`.
    Obj { close: usize, entries: Vec<Entry> },
}

impl Entry {
    /// Byte just after the entry.
    fn end(&self) -> usize {
        match &self.value {
            Value::Str { end, .. } => *end,
            Value::Obj { close, .. } => close + 1,
        }
    }
}

fn find<'a>(entries: &'a [Entry], key: &str) -> Option<&'a Entry> {
    entries.iter().find(|e| e.key.eq_ignore_ascii_case(key))
}

/// The string at `path` (the last element is the key), if any.
pub fn get(text: &str, path: &[&str]) -> Result<Option<String>, EditError> {
    let doc = parse(text)?;
    let Some((key, parents)) = path.split_last() else { return Ok(None) };
    let mut entries = &doc;
    for p in parents {
        match find(entries, p).map(|e| &e.value) {
            Some(Value::Obj { entries: inner, .. }) => entries = inner,
            _ => return Ok(None),
        }
    }
    Ok(match find(entries, key).map(|e| &e.value) {
        Some(Value::Str { text, .. }) => Some(text.clone()),
        _ => None,
    })
}

/// Sets the string at `path` (the last element is the key), creating the
/// objects on the way. The first element (the file's root object) must exist.
pub fn set(text: &str, path: &[&str], value: &str) -> Result<String, EditError> {
    let doc = parse(text)?;
    let (key, parents) = path.split_last().ok_or_else(|| err("empty path"))?;
    let root = parents.first().ok_or_else(|| err("no root object"))?;
    if !matches!(find(&doc, root).map(|e| &e.value), Some(Value::Obj { .. })) {
        return Err(err(format!("the file has no \"{root}\" object")));
    }
    let mut entries = &doc;
    let mut close = 0;
    for (depth, p) in parents.iter().enumerate() {
        match find(entries, p).map(|e| &e.value) {
            Some(Value::Obj { entries: inner, close: c }) => {
                entries = inner;
                close = *c;
            }
            Some(Value::Str { .. }) => return Err(err(format!("\"{p}\" is not an object"))),
            None => {
                let block = block(&parents[depth..], key, value, depth);
                return Ok(insert_before(text, close, &block));
            }
        }
    }
    match find(entries, key).map(|e| &e.value) {
        Some(Value::Str { start, end, .. }) => Ok(format!("{}{}{}", &text[..*start], quoted(value), &text[*end..])),
        Some(Value::Obj { .. }) => Err(err(format!("\"{key}\" is an object"))),
        None => Ok(insert_before(text, close, &format!("{}{}\t\t{}\n", tabs(parents.len()), quoted(key), quoted(value)))),
    }
}

/// Removes the entry at `path` (a string or a whole object), with its line.
/// The text comes back unchanged when it is not there.
pub fn remove(text: &str, path: &[&str]) -> Result<String, EditError> {
    let doc = parse(text)?;
    let Some((key, parents)) = path.split_last() else { return Ok(text.to_string()) };
    let mut entries = &doc;
    for p in parents {
        match find(entries, p).map(|e| &e.value) {
            Some(Value::Obj { entries: inner, .. }) => entries = inner,
            _ => return Ok(text.to_string()),
        }
    }
    let Some(entry) = find(entries, key) else { return Ok(text.to_string()) };
    let line_start = text[..entry.start].rfind('\n').map_or(0, |i| i + 1);
    let start = if text[line_start..entry.start].trim().is_empty() { line_start } else { entry.start };
    let mut end = entry.end();
    let rest = &text[end..];
    let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
    if rest[..line_end].trim().is_empty() {
        end += line_end;
    }
    Ok(format!("{}{}", &text[..start], &text[end..]))
}

/// Inserts `block` (whole lines) on the line of the `}` at `close`.
fn insert_before(text: &str, close: usize, block: &str) -> String {
    let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
    let at = if text[line_start..close].trim().is_empty() { line_start } else { close };
    let lead = if at == close { "\n" } else { "" };
    format!("{}{lead}{block}{}", &text[..at], &text[at..])
}

/// Nested objects `names` holding `key` = `value`, starting at `depth`.
fn block(names: &[&str], key: &str, value: &str, depth: usize) -> String {
    let Some((first, rest)) = names.split_first() else {
        return format!("{}{}\t\t{}\n", tabs(depth), quoted(key), quoted(value));
    };
    let t = tabs(depth);
    format!("{t}{}\n{t}{{\n{}{t}}}\n", quoted(first), block(rest, key, value, depth + 1))
}

fn tabs(n: usize) -> String {
    "\t".repeat(n)
}

/// A quoted VDF string, with Steam's escapes.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn parse(text: &str) -> Result<Vec<Entry>, EditError> {
    let mut p = Parser { s: text.as_bytes(), text, i: 0 };
    p.entries(true)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

enum Tok {
    Open,
    Close,
    Str { start: usize, end: usize, text: String },
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(&[0xEF, 0xBB, 0xBF]) {
                self.i += 3;
                continue;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
    }

    fn token(&mut self) -> Result<Option<Tok>, EditError> {
        self.skip_ws();
        let start = self.i;
        let Some(&c) = self.s.get(self.i) else { return Ok(None) };
        self.i += 1;
        match c {
            b'{' => Ok(Some(Tok::Open)),
            b'}' => Ok(Some(Tok::Close)),
            b'"' => {
                let mut out = Vec::new();
                loop {
                    let Some(&b) = self.s.get(self.i) else { return Err(err("unterminated string")) };
                    self.i += 1;
                    match b {
                        b'"' => break,
                        b'\\' => {
                            let Some(&e) = self.s.get(self.i) else { return Err(err("dangling escape")) };
                            self.i += 1;
                            out.push(match e {
                                b'n' => b'\n',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                        other => out.push(other),
                    }
                }
                Ok(Some(Tok::Str { start, end: self.i, text: String::from_utf8_lossy(&out).into_owned() }))
            }
            _ => {
                while self.i < self.s.len() && !self.s[self.i].is_ascii_whitespace() && !matches!(self.s[self.i], b'{' | b'}' | b'"') {
                    self.i += 1;
                }
                Ok(Some(Tok::Str { start, end: self.i, text: self.text[start..self.i].to_string() }))
            }
        }
    }

    fn entries(&mut self, top: bool) -> Result<Vec<Entry>, EditError> {
        let mut out = Vec::new();
        loop {
            match self.token()? {
                None if top => return Ok(out),
                None => return Err(err("unexpected end of the file")),
                Some(Tok::Close) if top => return Err(err("unbalanced '}'")),
                Some(Tok::Close) => return Ok(out),
                Some(Tok::Open) => return Err(err("'{' without a key")),
                Some(Tok::Str { start, text: key, .. }) => {
                    let value = match self.token()? {
                        Some(Tok::Open) => {
                            let entries = self.entries(false)?;
                            // `entries` stopped right after the `}`.
                            Value::Obj { close: self.i - 1, entries }
                        }
                        Some(Tok::Str { start, end, text }) => Value::Str { start, end, text },
                        _ => return Err(err(format!("\"{key}\" has no value"))),
                    };
                    out.push(Entry { key, start, value });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"400\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"cloud\"\n\t\t\t\t\t\t{\n\t\t\t\t\t\t\t\"last_sync_state\"\t\t\"synchronized\"\n\t\t\t\t\t\t}\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"A=\\\"x\\\" %command%\"\n\t\t\t\t\t}\n\t\t\t\t\t\"500\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    fn path(app: &str) -> [&str; 7] {
        ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps", app, "LaunchOptions"]
    }

    #[test]
    fn reads_and_replaces_a_value_in_place() {
        assert_eq!(get(LOCAL, &path("400")).unwrap().as_deref(), Some("A=\"x\" %command%"));
        assert_eq!(get(LOCAL, &path("500")).unwrap(), None);
        let out = set(LOCAL, &path("400"), "B=\"y z\" C=\\d %command%").unwrap();
        assert_eq!(out, LOCAL.replace("A=\\\"x\\\" %command%", "B=\\\"y z\\\" C=\\\\d %command%"));
        assert_eq!(get(&out, &path("400")).unwrap().as_deref(), Some("B=\"y z\" C=\\d %command%"));
    }

    #[test]
    fn adds_keys_and_objects_in_steams_layout() {
        let out = set(LOCAL, &path("500"), "%command% -x").unwrap();
        assert!(out.contains("\t\t\t\t\t\t\"LastPlayed\"\t\t\"1\"\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"%command% -x\"\n\t\t\t\t\t}\n"));
        let out = set(LOCAL, &path("600"), "-y").unwrap();
        assert!(
            out.contains("\t\t\t\t\t}\n\t\t\t\t\t\"600\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-y\"\n\t\t\t\t\t}\n\t\t\t\t}\n")
        );
        assert_eq!(get(&out, &path("600")).unwrap().as_deref(), Some("-y"));
        assert_eq!(get(&out, &path("400")).unwrap().as_deref(), Some("A=\"x\" %command%"));
        // Several missing levels at once.
        let bare = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t}\n}\n";
        let p = ["InstallConfigStore", "Software", "Valve", "Steam", "CompatToolMapping", "70", "name"];
        let out = set(bare, &p, "proton_9").unwrap();
        assert_eq!(get(&out, &p).unwrap().as_deref(), Some("proton_9"));
        assert!(out.contains("\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n"));
        assert!(set("\"Other\"\n{\n}\n", &p, "x").is_err());
    }

    #[test]
    fn removes_whole_lines_and_objects() {
        let out = remove(LOCAL, &path("400")).unwrap();
        assert!(!out.contains("LaunchOptions"));
        assert!(out.contains("\t\t\t\t\t\t}\n\t\t\t\t\t}\n\t\t\t\t\t\"500\""));
        let out = remove(LOCAL, &["UserLocalConfigStore", "Software", "Valve", "Steam", "apps", "500"]).unwrap();
        assert!(!out.contains("\"500\""));
        assert!(out.ends_with("\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n"));
        assert_eq!(remove(LOCAL, &path("700")).unwrap(), LOCAL);
    }

    #[test]
    fn broken_files_are_refused() {
        assert!(set("\"a\" {", &["a", "b"], "c").is_err());
        assert!(get("\"a\" { \"b\" }", &["a", "b"]).is_err());
    }
}
