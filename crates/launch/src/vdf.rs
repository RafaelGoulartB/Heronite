//! Minimal parser for Valve's text KeyValues format (`libraryfolders.vdf`,
//! `appmanifest_*.acf`). Handles quoted and bare tokens, `//` comments and
//! the usual backslash escapes. Duplicate keys are kept in order.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdfError(pub String);

impl fmt::Display for VdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid VDF: {}", self.0)
    }
}

impl std::error::Error for VdfError {}

impl Vdf {
    /// Parses a document. The result is always an object holding the
    /// top-level keys (e.g. `AppState`, `libraryfolders`).
    pub fn parse(text: &str) -> Result<Vdf, VdfError> {
        let mut p = Parser { s: text.as_bytes(), text, i: 0 };
        let entries = p.entries(true)?;
        Ok(Vdf::Obj(entries))
    }

    /// Case-insensitive key lookup (Steam is inconsistent about casing).
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(entries) => entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(e) => e,
            Vdf::Str(_) => &[],
        }
    }

    /// String value of `key`, if it exists and is a string.
    pub fn str_of(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Vdf::as_str)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            // Strip a UTF-8 BOM that some tools leave at the start.
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

    fn token(&mut self) -> Result<Option<Tok>, VdfError> {
        self.skip_ws();
        let Some(&c) = self.s.get(self.i) else {
            return Ok(None);
        };
        match c {
            b'{' => {
                self.i += 1;
                Ok(Some(Tok::Open))
            }
            b'}' => {
                self.i += 1;
                Ok(Some(Tok::Close))
            }
            b'"' => {
                self.i += 1;
                let mut out = Vec::new();
                loop {
                    let Some(&b) = self.s.get(self.i) else {
                        return Err(VdfError("unterminated string".into()));
                    };
                    self.i += 1;
                    match b {
                        b'"' => break,
                        b'\\' => {
                            let Some(&e) = self.s.get(self.i) else {
                                return Err(VdfError("dangling escape".into()));
                            };
                            self.i += 1;
                            out.push(match e {
                                b'n' => b'\n',
                                b't' => b'\t',
                                b'\\' => b'\\',
                                b'"' => b'"',
                                other => other,
                            });
                        }
                        other => out.push(other),
                    }
                }
                Ok(Some(Tok::Str(String::from_utf8_lossy(&out).into_owned())))
            }
            _ => {
                let start = self.i;
                while self.i < self.s.len() && !self.s[self.i].is_ascii_whitespace() && !matches!(self.s[self.i], b'{' | b'}' | b'"') {
                    self.i += 1;
                }
                Ok(Some(Tok::Str(self.text[start..self.i].to_string())))
            }
        }
    }

    fn entries(&mut self, top: bool) -> Result<Vec<(String, Vdf)>, VdfError> {
        let mut out = Vec::new();
        loop {
            match self.token()? {
                None if top => return Ok(out),
                None => return Err(VdfError("unexpected end of input".into())),
                Some(Tok::Close) if !top => return Ok(out),
                Some(Tok::Close) => return Err(VdfError("unbalanced '}'".into())),
                Some(Tok::Open) => return Err(VdfError("'{' without a key".into())),
                Some(Tok::Str(key)) => match self.token()? {
                    Some(Tok::Open) => {
                        let inner = self.entries(false)?;
                        out.push((key, Vdf::Obj(inner)));
                    }
                    Some(Tok::Str(value)) => out.push((key, Vdf::Str(value))),
                    _ => return Err(VdfError(format!("key '{key}' has no value"))),
                },
            }
        }
    }
}

enum Tok {
    Open,
    Close,
    Str(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_objects_comments_and_escapes() {
        let v = Vdf::parse(
            "// comment\n\"AppState\"\n{\n\t\"appid\"\t\t\"400\"\n\t\"name\" \"Portal \\\"x\\\"\"\n\t\"UserConfig\" { \"language\" \"english\" }\n}\n",
        )
        .unwrap();
        let st = v.get("appstate").unwrap();
        assert_eq!(st.str_of("AppID"), Some("400"));
        assert_eq!(st.str_of("name"), Some("Portal \"x\""));
        assert_eq!(st.get("UserConfig").unwrap().str_of("language"), Some("english"));
    }

    #[test]
    fn parses_libraryfolders_in_both_formats() {
        let modern =
            Vdf::parse("\"libraryfolders\" { \"0\" { \"path\" \"/a\" \"apps\" { \"1\" \"2\" } } \"1\" { \"path\" \"/b\" } }").unwrap();
        let folders = modern.get("libraryfolders").unwrap();
        assert_eq!(folders.entries().len(), 2);
        assert_eq!(folders.entries()[1].1.str_of("path"), Some("/b"));
        let old = Vdf::parse("\"LibraryFolders\" { \"TimeNextStatsReport\" \"1\" \"1\" \"/old\" }").unwrap();
        assert_eq!(old.get("libraryfolders").unwrap().str_of("1"), Some("/old"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(Vdf::parse("\"a\" {").is_err());
        assert!(Vdf::parse("}").is_err());
        assert!(Vdf::parse("\"a\"").is_err());
    }
}
