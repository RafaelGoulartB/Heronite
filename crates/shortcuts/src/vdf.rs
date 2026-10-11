//! Valve's binary KeyValues, the format of `shortcuts.vdf`.
//!
//! Every node is `<type byte> <key>\0 <value>`; objects end with `0x08`.
//! The parser keeps everything it reads (key order, raw string bytes, value
//! types, bytes after the last terminator) so an unmodified document
//! serializes **byte-identical** to what Steam wrote.

use anyhow::{bail, Result};

const T_OBJECT: u8 = 0x00;
const T_STRING: u8 = 0x01;
const T_INT: u8 = 0x02;
const T_FLOAT: u8 = 0x03;
const T_PTR: u8 = 0x04;
const T_WSTRING: u8 = 0x05;
const T_COLOR: u8 = 0x06;
const T_UINT64: u8 = 0x07;
const T_END: u8 = 0x08;
const T_INT64: u8 = 0x0A;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Obj(Vec<Entry>),
    /// Raw bytes (Steam writes UTF-8).
    Str(Vec<u8>),
    Int(u32),
    Float(u32),
    Ptr(u32),
    Color(u32),
    UInt64(u64),
    Int64(i64),
    WStr(Vec<u16>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub value: Node,
}

impl Entry {
    pub fn new(key: &str, value: Node) -> Self {
        Self { key: key.as_bytes().to_vec(), value }
    }

    pub fn key_str(&self) -> String {
        String::from_utf8_lossy(&self.key).into_owned()
    }
}

impl Node {
    pub fn string(s: &str) -> Node {
        Node::Str(s.as_bytes().to_vec())
    }

    pub fn as_str(&self) -> Option<String> {
        match self {
            Node::Str(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&Vec<Entry>> {
        match self {
            Node::Obj(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_obj_mut(&mut self) -> Option<&mut Vec<Entry>> {
        match self {
            Node::Obj(v) => Some(v),
            _ => None,
        }
    }
}

/// A whole file: the root object plus the bits needed for exact re-writing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    pub root: Vec<Entry>,
    /// The root object ended with its `0x08` (a truncated file has none).
    pub terminated: bool,
    /// Bytes after the root terminator, kept verbatim.
    pub trailer: Vec<u8>,
}

impl Document {
    /// Empty document with an empty `shortcuts` object (what Heroic writes for
    /// a new `shortcuts.vdf`).
    pub fn empty_shortcuts() -> Self {
        Self { root: vec![Entry::new("shortcuts", Node::Obj(Vec::new()))], terminated: true, trailer: Vec::new() }
    }

    /// Case-insensitive lookup of a top-level key.
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.root.iter().find(|e| e.key.eq_ignore_ascii_case(key.as_bytes())).map(|e| &e.value)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Node> {
        self.root.iter_mut().find(|e| e.key.eq_ignore_ascii_case(key.as_bytes())).map(|e| &mut e.value)
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).ok_or_else(|| anyhow::anyhow!("unexpected end of data at {}", self.pos))?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.data.len());
        let Some(end) = end else { bail!("unexpected end of data at {}", self.pos) };
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn cstr(&mut self) -> Result<Vec<u8>> {
        let rest = &self.data[self.pos..];
        let Some(n) = rest.iter().position(|b| *b == 0) else { bail!("unterminated string at {}", self.pos) };
        let s = rest[..n].to_vec();
        self.pos += n + 1;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }

    /// Parses entries until the object terminator; `Ok(false)` when the data
    /// ended first (only accepted at the root).
    fn object(&mut self, out: &mut Vec<Entry>, depth: usize) -> Result<bool> {
        if depth > 64 {
            bail!("nesting too deep");
        }
        loop {
            if self.pos >= self.data.len() {
                return Ok(false);
            }
            let t = self.byte()?;
            if t == T_END {
                return Ok(true);
            }
            let key = self.cstr()?;
            let value = match t {
                T_OBJECT => {
                    let mut children = Vec::new();
                    if !self.object(&mut children, depth + 1)? {
                        bail!("unterminated object {:?}", String::from_utf8_lossy(&key));
                    }
                    Node::Obj(children)
                }
                T_STRING => Node::Str(self.cstr()?),
                T_INT => Node::Int(self.u32()?),
                T_FLOAT => Node::Float(self.u32()?),
                T_PTR => Node::Ptr(self.u32()?),
                T_COLOR => Node::Color(self.u32()?),
                T_UINT64 => Node::UInt64(self.u64()?),
                T_INT64 => Node::Int64(self.u64()? as i64),
                T_WSTRING => {
                    let mut units = Vec::new();
                    loop {
                        let b = self.take(2)?;
                        let u = u16::from_le_bytes([b[0], b[1]]);
                        if u == 0 {
                            break;
                        }
                        units.push(u);
                    }
                    Node::WStr(units)
                }
                other => bail!("unknown value type 0x{other:02x} at byte {}", self.pos - 1),
            };
            out.push(Entry { key, value });
        }
    }
}

/// Parses a binary VDF file.
pub fn parse(data: &[u8]) -> Result<Document> {
    let mut r = Reader { data, pos: 0 };
    let mut root = Vec::new();
    let terminated = r.object(&mut root, 0)?;
    let trailer = data[r.pos..].to_vec();
    Ok(Document { root, terminated, trailer })
}

fn write_entries(out: &mut Vec<u8>, entries: &[Entry]) {
    for e in entries {
        let t = match &e.value {
            Node::Obj(_) => T_OBJECT,
            Node::Str(_) => T_STRING,
            Node::Int(_) => T_INT,
            Node::Float(_) => T_FLOAT,
            Node::Ptr(_) => T_PTR,
            Node::Color(_) => T_COLOR,
            Node::UInt64(_) => T_UINT64,
            Node::Int64(_) => T_INT64,
            Node::WStr(_) => T_WSTRING,
        };
        out.push(t);
        out.extend_from_slice(&e.key);
        out.push(0);
        match &e.value {
            Node::Obj(children) => {
                write_entries(out, children);
                out.push(T_END);
            }
            Node::Str(s) => {
                out.extend_from_slice(s);
                out.push(0);
            }
            Node::Int(v) | Node::Float(v) | Node::Ptr(v) | Node::Color(v) => out.extend_from_slice(&v.to_le_bytes()),
            Node::UInt64(v) => out.extend_from_slice(&v.to_le_bytes()),
            Node::Int64(v) => out.extend_from_slice(&v.to_le_bytes()),
            Node::WStr(units) => {
                for u in units {
                    out.extend_from_slice(&u.to_le_bytes());
                }
                out.extend_from_slice(&[0, 0]);
            }
        }
    }
}

/// Serializes a document (exactly the parsed bytes when nothing changed).
pub fn serialize(doc: &Document) -> Vec<u8> {
    let mut out = Vec::new();
    write_entries(&mut out, &doc.root);
    if doc.terminated {
        out.push(T_END);
    }
    out.extend_from_slice(&doc.trailer);
    out
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// A real `shortcuts.vdf` written by Steam (one "Control.exe" entry).
    pub const REAL: &[u8] = b"\x00shortcuts\x00\x000\x00\x02appid\x00\xed\x0f\xae\x99\x01AppName\x00Control.exe\x00\x01Exe\x00\"/data/Games/Heroic/Control/Control.exe\"\x00\x01StartDir\x00/data/Games/Heroic/Control/\x00\x01icon\x00\x00\x01ShortcutPath\x00\x00\x01LaunchOptions\x00WINEDLLOVERRIDES=dxgi.dll=n,b %COMMAND%\x00\x02IsHidden\x00\x00\x00\x00\x00\x02AllowDesktopConfig\x00\x01\x00\x00\x00\x02AllowOverlay\x00\x01\x00\x00\x00\x02OpenVR\x00\x00\x00\x00\x00\x02Devkit\x00\x00\x00\x00\x00\x01DevkitGameID\x00\x00\x02DevkitOverrideAppID\x00\x00\x00\x00\x00\x02LastPlayTime\x00\x85m\xa3j\x01FlatpakAppID\x00\x00\x01sortas\x00\x00\x00tags\x00\x08\x08\x08\x08";
}

#[cfg(test)]
mod tests {
    use super::tests_support::REAL;
    use super::*;

    #[test]
    fn real_file_round_trips_byte_identical() {
        let doc = parse(REAL).unwrap();
        assert!(doc.terminated && doc.trailer.is_empty());
        let shortcuts = doc.get("shortcuts").unwrap().as_obj().unwrap();
        assert_eq!(shortcuts.len(), 1);
        let entry = shortcuts[0].value.as_obj().unwrap();
        assert_eq!(entry.iter().find(|e| e.key == b"AppName").unwrap().value.as_str().as_deref(), Some("Control.exe"));
        assert_eq!(serialize(&doc), REAL);
    }

    /// Heroic's own fixtures (when the sibling checkout is there) and the real
    /// Steam file (when `/tmp/system-agent/real-shortcuts.vdf` was copied).
    #[test]
    fn heroic_fixtures_round_trip() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../HeroicGamesLauncher/src/backend/shortcuts/nonesteamgame/__tests__/test_data");
        let mut files: Vec<_> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        files.push("/tmp/system-agent/real-shortcuts.vdf".into());
        let mut checked = 0;
        for f in files {
            let Ok(bytes) = std::fs::read(&f) else { continue };
            let doc = parse(&bytes).unwrap_or_else(|e| panic!("{}: {e:#}", f.display()));
            assert_eq!(serialize(&doc), bytes, "{}", f.display());
            checked += 1;
        }
        eprintln!("round-tripped {checked} files");
    }

    #[test]
    fn all_value_types_round_trip() {
        let doc = Document {
            root: vec![Entry::new(
                "o",
                Node::Obj(vec![
                    Entry::new("s", Node::string("héllo")),
                    Entry::new("i", Node::Int(7)),
                    Entry::new("f", Node::Float(0x3f80_0000)),
                    Entry::new("p", Node::Ptr(1)),
                    Entry::new("c", Node::Color(0xff00ff00)),
                    Entry::new("u", Node::UInt64(u64::MAX - 3)),
                    Entry::new("l", Node::Int64(-5)),
                    Entry::new("w", Node::WStr(vec![0x68, 0x69])),
                    Entry::new("e", Node::Obj(vec![])),
                ]),
            )],
            terminated: true,
            trailer: Vec::new(),
        };
        let bytes = serialize(&doc);
        assert_eq!(parse(&bytes).unwrap(), doc);
    }

    #[test]
    fn unterminated_root_and_trailer_are_kept() {
        // Truncated root terminator.
        let cut = &REAL[..REAL.len() - 1];
        let doc = parse(cut).unwrap();
        assert!(!doc.terminated);
        assert_eq!(serialize(&doc), cut);
        // Extra bytes after the root terminator.
        let mut extra = REAL.to_vec();
        extra.extend_from_slice(b"\x08\x08");
        let doc = parse(&extra).unwrap();
        assert!(doc.terminated);
        assert_eq!(doc.trailer, b"\x08\x08");
        assert_eq!(serialize(&doc), extra);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse(b"\x00shortcuts\x00\x000\x00\x02appid\x00\x01").is_err());
        assert!(parse(b"\x09key\x00").is_err());
        assert!(parse(b"\x01key").is_err());
        assert!(parse(b"\x00a\x00\x00b\x00").is_err());
        assert!(parse(b"").unwrap().root.is_empty());
    }
}
