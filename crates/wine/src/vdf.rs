//! Minimal Valve KeyValues (VDF) reader, enough for `libraryfolders.vdf`.

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = src.chars().peekable();
    while let Some(&c) = it.peek() {
        match c {
            '{' => {
                out.push(Tok::Open);
                it.next();
            }
            '}' => {
                out.push(Tok::Close);
                it.next();
            }
            '"' => {
                it.next();
                let mut s = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(o) => s.push(o),
                            None => break,
                        },
                        _ => s.push(c),
                    }
                }
                out.push(Tok::Str(s));
            }
            '/' => {
                // `//` line comment
                it.next();
                if it.peek() == Some(&'/') {
                    for c in it.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
            }
            c if c.is_whitespace() => {
                it.next();
            }
            _ => {
                // unquoted token
                let mut s = String::new();
                while let Some(&c) = it.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    s.push(c);
                    it.next();
                }
                out.push(Tok::Str(s));
            }
        }
    }
    out
}

fn parse_obj(toks: &[Tok], pos: &mut usize) -> Vec<(String, Vdf)> {
    let mut items = Vec::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::Close => {
                *pos += 1;
                return items;
            }
            Tok::Open => {
                *pos += 1; // stray
            }
            Tok::Str(key) => {
                *pos += 1;
                match toks.get(*pos) {
                    Some(Tok::Open) => {
                        *pos += 1;
                        let sub = parse_obj(toks, pos);
                        items.push((key.clone(), Vdf::Obj(sub)));
                    }
                    Some(Tok::Str(v)) => {
                        *pos += 1;
                        items.push((key.clone(), Vdf::Str(v.clone())));
                    }
                    _ => {}
                }
            }
        }
    }
    items
}

pub fn parse(src: &str) -> Vdf {
    let toks = tokenize(src);
    let mut pos = 0;
    Vdf::Obj(parse_obj(&toks, &mut pos))
}

/// `path` of every library listed in a `libraryfolders.vdf`.
pub fn library_paths(src: &str) -> Option<Vec<String>> {
    let doc = parse(src);
    let Vdf::Obj(folders) = doc.get("libraryfolders")? else { return None };
    Some(
        folders
            .iter()
            .filter_map(|(_, v)| v.get("path").and_then(Vdf::as_str).map(str::to_owned))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_libraryfolders() {
        let src = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/u/.local/share/Steam\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"1\"\t\t\"2\"\n\t\t}\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/data/SteamLibrary\"\n\t}\n}\n";
        assert_eq!(
            library_paths(src).unwrap(),
            vec!["/home/u/.local/share/Steam".to_owned(), "/data/SteamLibrary".to_owned()]
        );
        assert!(library_paths("\"other\" { }").is_none());
    }
}
