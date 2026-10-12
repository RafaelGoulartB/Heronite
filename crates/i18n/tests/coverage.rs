//! Scans the workspace sources for `tr!`, `trn!`, `trc!` and `n_!` and checks
//! them against `locales/pt-BR/*.json`:
//!
//! - every key has a translation, and every translation is a key in use;
//! - a key translated in two files gets the same text;
//! - a translation only uses placeholders its key has (`{n}` for `trn!`).
//!
//! `HERONITE_I18N_MISSING=1 cargo test -p heronite-i18n --test coverage -- --nocapture`
//! prints the missing keys as JSON entries, ready to fill in.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use heronite_i18n::{embedded_files, Lang};

#[derive(Debug, Clone)]
struct Use {
    key: String,
    /// `file:line`.
    at: String,
    plural: bool,
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn sources() -> Vec<PathBuf> {
    let root = workspace();
    let mut files = Vec::new();
    rust_files(&root.join("app/src"), &mut files);
    for c in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        rust_files(&c.path().join("src"), &mut files);
    }
    files.sort();
    files
}

/// Parses a Rust string literal at the start of `s`; gives its value and the
/// bytes it took.
fn string_literal(s: &str) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if b.first() == Some(&b'r') {
        let hashes = s[1..].bytes().take_while(|&c| c == b'#').count();
        if b.get(1 + hashes) != Some(&b'"') {
            return None;
        }
        let body = 2 + hashes;
        let close = format!("\"{}", "#".repeat(hashes));
        let end = s[body..].find(&close)?;
        return Some((s[body..body + end].to_string(), body + end + close.len()));
    }
    if b.first() != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut chars = s[1..].char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, i + 2)),
            '\\' => {
                let (_, e) = chars.next()?;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    '\\' | '"' | '\'' => out.push(e),
                    'x' => {
                        let hex: String = (0..2).filter_map(|_| chars.next().map(|(_, c)| c)).collect();
                        out.push(u8::from_str_radix(&hex, 16).ok()? as char);
                    }
                    'u' => {
                        chars.next(); // {
                        let mut hex = String::new();
                        for (_, c) in chars.by_ref() {
                            if c == '}' {
                                break;
                            }
                            hex.push(c);
                        }
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    '\n' => {
                        // Line continuation: skip the next line's indentation.
                        let rest = &s[1 + i + 2..];
                        let skip = rest.len() - rest.trim_start().len();
                        for _ in 0..rest[..skip].chars().count() {
                            chars.next();
                        }
                    }
                    _ => return None,
                }
            }
            c => out.push(c),
        }
    }
    None
}

/// Skips one argument expression (up to a top-level `,`); gives the bytes taken.
fn skip_expr(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            b',' if depth == 0 => return Some(i),
            b'"' => i += string_literal(&s[i..])?.1 - 1,
            b'\'' if b.get(i + 2) == Some(&b'\'') => i += 2,
            _ => {}
        }
        i += 1;
    }
    None
}

fn literal_after(s: &str, at: &mut usize) -> Option<String> {
    let rest = &s[*at..];
    let ws = rest.len() - rest.trim_start().len();
    let (v, n) = string_literal(&rest[ws..])?;
    *at += ws + n;
    Some(v)
}

fn comma_after(s: &str, at: &mut usize) -> Option<()> {
    let rest = &s[*at..];
    let ws = rest.len() - rest.trim_start().len();
    (rest[ws..].starts_with(',')).then(|| *at += ws + 1)
}

fn scan(path: &Path, text: &str, uses: &mut Vec<Use>) {
    let rel = path.strip_prefix(workspace()).unwrap_or(path).display().to_string();
    // The unit tests at the end of a file do not count.
    let text = text.find("#[cfg(test)]\nmod tests").map_or(text, |end| &text[..end]);
    // Comments are blanked (keeping offsets) so examples in docs do not count.
    let code: String = text
        .lines()
        .map(|l| if l.trim_start().starts_with("//") { " ".repeat(l.len()) } else { l.to_string() })
        .collect::<Vec<_>>()
        .join("\n");
    for mac in ["tr!(", "trn!(", "trc!(", "n_!("] {
        let mut from = 0;
        while let Some(found) = code[from..].find(mac) {
            let start = from + found;
            from = start + mac.len();
            if code[..start].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let line = code[..start].matches('\n').count() + 1;
            let at = format!("{rel}:{line}");
            let mut i = from;
            let mut keys = Vec::new();
            let mut plural = false;
            let ok = (|| {
                match mac {
                    "trn!(" => {
                        i += skip_expr(&code[i..])? + 1;
                        keys.push(literal_after(&code, &mut i)?);
                        comma_after(&code, &mut i)?;
                        keys.push(literal_after(&code, &mut i)?);
                        plural = true;
                    }
                    "trc!(" => {
                        let ctx = literal_after(&code, &mut i)?;
                        comma_after(&code, &mut i)?;
                        keys.push(format!("{ctx}|{}", literal_after(&code, &mut i)?));
                    }
                    _ => keys.push(literal_after(&code, &mut i)?),
                }
                Some(())
            })();
            assert!(ok.is_some(), "{at}: could not read the literal of {mac}…); keys must be plain string literals");
            uses.extend(keys.into_iter().map(|key| Use { key, at: at.clone(), plural }));
        }
    }
}

fn placeholders(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let tail = &rest[i + 1..];
        if let Some(after) = tail.strip_prefix('{') {
            rest = after;
            continue;
        }
        match tail.find('}') {
            Some(end) if !tail[..end].is_empty() && tail[..end].chars().all(|c| c.is_alphanumeric() || c == '_') => {
                out.insert(tail[..end].to_string());
                rest = &tail[end + 1..];
            }
            _ => rest = tail,
        }
    }
    out
}

#[test]
fn every_key_is_translated_and_every_translation_used() {
    let mut uses = Vec::new();
    for f in sources() {
        scan(&f, &std::fs::read_to_string(&f).unwrap(), &mut uses);
    }
    assert!(uses.len() > 10, "the scan found almost nothing ({} keys)", uses.len());

    let mut translations: BTreeMap<String, (String, &str)> = BTreeMap::new();
    let mut problems = Vec::new();
    for (file, text) in embedded_files(Lang::PtBr) {
        let map: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                problems.push(format!("locales/pt-BR/{file}: not a JSON object of strings: {e}"));
                continue;
            }
        };
        for (k, v) in map {
            let Some(v) = v.as_str() else {
                problems.push(format!("locales/pt-BR/{file}: {k:?} is not a string"));
                continue;
            };
            if v.trim().is_empty() {
                problems.push(format!("locales/pt-BR/{file}: {k:?} has an empty translation"));
                continue;
            }
            if let Some((prev, prev_file)) = translations.get(&k) {
                if prev != v {
                    problems.push(format!("{k:?}: {prev:?} in {prev_file}, {v:?} in {file}"));
                }
                continue;
            }
            translations.insert(k, (v.to_string(), file));
        }
    }

    let used: BTreeSet<&str> = uses.iter().map(|u| u.key.as_str()).collect();
    let mut missing: BTreeMap<&str, &Use> = BTreeMap::new();
    for u in &uses {
        match translations.get(&u.key) {
            None => {
                missing.entry(&u.key).or_insert(u);
            }
            Some((v, file)) => {
                let mut allowed = placeholders(&u.key);
                if u.plural {
                    allowed.insert("n".into());
                }
                let extra: Vec<_> = placeholders(v).difference(&allowed).cloned().collect();
                if !extra.is_empty() {
                    problems.push(format!("{} ({file}): {:?} uses {extra:?}, which {:?} does not have", u.at, v, u.key));
                }
            }
        }
    }
    for (k, (_, file)) in &translations {
        if !used.contains(k.as_str()) {
            problems.push(format!("locales/pt-BR/{file}: {k:?} is not used anywhere"));
        }
    }
    if crate::env_flag("HERONITE_I18N_MISSING") {
        for (k, u) in &missing {
            println!("  {}: \"\",  // {}", serde_json::to_string(k).unwrap(), u.at);
        }
    }
    for (k, u) in &missing {
        problems.push(format!("{}: {k:?} has no pt-BR translation", u.at));
    }
    assert!(problems.is_empty(), "{} problem(s):\n{}", problems.len(), problems.join("\n"));
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

#[test]
fn literal_parser_handles_escapes_and_raw_strings() {
    assert_eq!(string_literal(r#""a \"b\" \u{201c}c\u{201d}\n" x"#).map(|v| v.0), Some("a \"b\" \u{201c}c\u{201d}\n".into()));
    assert_eq!(string_literal(r###"r#"say "hi""# rest"###).map(|v| v.0), Some("say \"hi\"".into()));
    assert_eq!(string_literal("\"one \\\n      two\"").map(|v| v.0), Some("one two".into()));
    let mut uses = Vec::new();
    let code = "let a = tr!(\"Play\");\n// tr!(\"Ignored\")\nlet b = trn!(v.len(), \"{n} game\", \"{n} games\");\nlet c = trc!(\"verb\", \"Run\"); let d = str!(\"no\");";
    scan(Path::new("x.rs"), code, &mut uses);
    let keys: Vec<_> = uses.iter().map(|u| u.key.as_str()).collect();
    assert_eq!(keys, ["Play", "{n} game", "{n} games", "verb|Run"]);
    assert_eq!(placeholders("{a} {{b}} {c d} {e}"), BTreeSet::from(["a".to_string(), "e".to_string()]));
}
