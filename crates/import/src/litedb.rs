//! The documents of a LiteDB 4 file, the database Playnite keeps its library
//! in. Read only and whole: the file is read into memory and its data pages
//! walked, so every document of every collection comes out, in no
//! particular order. Indexes are not read.
//!
//! The layout (LiteDB 4.1, file version 7): pages of [`PAGE_SIZE`] bytes,
//! each starting with a header of [`PAGE_HEADER`] bytes: its id (u32), type
//! (u8), the previous and next page (u32 each), an item count and the free
//! bytes (u16 each), then 8 reserved bytes. Numbers are little-endian. The
//! first page names the format ([`HEADER_INFO`], [`FILE_VERSION`]). A data
//! page holds `item count` blocks: the block's index (u16), its extend page
//! (u32), the length of its bytes (u16) and the bytes, a document. A
//! document too big for a page is in a chain of extend pages instead (by
//! their next page), each holding `item count` bytes of it. Documents are
//! BSON, a .NET `Guid` as binary of subtype 4 in .NET's byte order.

use std::collections::HashSet;

use anyhow::{bail, ensure, Context, Result};

pub const PAGE_SIZE: usize = 4096;
pub const PAGE_HEADER: usize = 25;
/// What the first page holds after its header.
pub const HEADER_INFO: &[u8] = b"** This is a LiteDB file **";
pub const FILE_VERSION: u8 = 7;
const DATA_PAGE: u8 = 4;
const EXTEND_PAGE: u8 = 5;
/// No page (the end of a chain, a block without extend pages).
const NO_PAGE: u32 = u32::MAX;
/// A block's index, extend page and length.
const BLOCK_HEADER: usize = 2 + 4 + 2;

/// A BSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Bson {
    Double(f64),
    String(String),
    Document(Document),
    Array(Vec<Bson>),
    /// Subtype and bytes.
    Binary(u8, Vec<u8>),
    ObjectId([u8; 12]),
    Bool(bool),
    /// Milliseconds since 1970, UTC.
    DateTime(i64),
    Null,
    Int32(i32),
    Int64(i64),
    /// A .NET `decimal`, as its 16 bytes.
    Decimal([u8; 16]),
    MinValue,
    MaxValue,
}

/// A BSON document: its fields in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document(pub Vec<(String, Bson)>);

impl Document {
    pub fn get(&self, key: &str) -> Option<&Bson> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Bson::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Bson::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// A whole number, whichever way it was stored.
    pub fn int(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Bson::Int32(n) => Some(i64::from(*n)),
            Bson::Int64(n) => Some(*n),
            Bson::Double(n) if n.is_finite() => Some(*n as i64),
            _ => None,
        }
    }

    /// A .NET `Guid`, as it is written (`3f2504e0-4f89-11d3-9a0c-0305e82c3301`).
    pub fn guid(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Bson::Binary(4, bytes) => guid(bytes),
            Bson::String(s) => Some(s.to_ascii_lowercase()),
            _ => None,
        }
    }

    /// Milliseconds since 1970, UTC.
    pub fn date(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Bson::DateTime(ms) => Some(*ms),
            _ => None,
        }
    }
}

/// A .NET `Guid` of its 16 bytes (`Guid.ToByteArray`: the first three
/// groups little-endian).
pub fn guid(b: &[u8]) -> Option<String> {
    let b: &[u8; 16] = b.try_into().ok()?;
    let hex = |bytes: &[u8]| bytes.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let rev = |bytes: &[u8]| hex(&bytes.iter().rev().copied().collect::<Vec<_>>());
    Some(format!("{}-{}-{}-{}-{}", rev(&b[0..4]), rev(&b[4..6]), rev(&b[6..8]), hex(&b[8..10]), hex(&b[10..16])))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

/// The documents of the LiteDB 4 file `bytes`. A file of another format is
/// an error; a document that cannot be read is left out.
pub fn documents(bytes: &[u8]) -> Result<Vec<Document>> {
    ensure!(bytes.len() >= PAGE_SIZE, "too short for a LiteDB file");
    let info = &bytes[PAGE_HEADER..PAGE_HEADER + HEADER_INFO.len()];
    ensure!(info == HEADER_INFO, "not a LiteDB file");
    let version = bytes[PAGE_HEADER + HEADER_INFO.len()];
    if version != FILE_VERSION {
        bail!("LiteDB file version {version} (only {FILE_VERSION}, LiteDB 4, is read)");
    }
    let pages = bytes.len() / PAGE_SIZE;
    let page = |id: usize| &bytes[id * PAGE_SIZE..(id + 1) * PAGE_SIZE];
    let mut out = Vec::new();
    for id in 1..pages {
        let p = page(id);
        if p[4] != DATA_PAGE {
            continue;
        }
        let mut at = PAGE_HEADER;
        for _ in 0..u16_at(p, 13) {
            if at + BLOCK_HEADER > PAGE_SIZE {
                log::debug!("LiteDB page {id}: its blocks overrun it");
                break;
            }
            let extend = u32_at(p, at + 2);
            let len = usize::from(u16_at(p, at + 6));
            let start = at + BLOCK_HEADER;
            at = start + len;
            if at > PAGE_SIZE {
                log::debug!("LiteDB page {id}: a block overruns it");
                break;
            }
            let doc = if extend == NO_PAGE { Ok(p[start..at].to_vec()) } else { extended(bytes, extend) };
            match doc.and_then(|d| document(&d)) {
                Ok(doc) => out.push(doc),
                Err(e) => log::debug!("LiteDB page {id}: a document cannot be read: {e:#}"),
            }
        }
    }
    Ok(out)
}

/// The bytes of a chain of extend pages starting at `first`.
fn extended(bytes: &[u8], first: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut id = first;
    while id != NO_PAGE {
        ensure!(seen.insert(id), "extend pages loop at page {id}");
        let start = id as usize * PAGE_SIZE;
        let p = bytes.get(start..start + PAGE_SIZE).with_context(|| format!("extend page {id} is past the end"))?;
        ensure!(p[4] == EXTEND_PAGE, "page {id} is not an extend page");
        let len = usize::from(u16_at(p, 13));
        ensure!(PAGE_HEADER + len <= PAGE_SIZE, "extend page {id} overruns");
        out.extend_from_slice(&p[PAGE_HEADER..PAGE_HEADER + len]);
        id = u32_at(p, 9);
    }
    Ok(out)
}

/// A BSON document, all of `bytes`.
pub fn document(bytes: &[u8]) -> Result<Document> {
    let mut r = Reader { b: bytes, at: 0 };
    let doc = r.document()?;
    Ok(doc)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.b.len()).context("BSON ends early")?;
        let out = &self.b[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn len(&mut self) -> Result<usize> {
        usize::try_from(self.i32()?).context("negative BSON length")
    }

    fn cstring(&mut self) -> Result<String> {
        let rest = &self.b[self.at..];
        let n = rest.iter().position(|b| *b == 0).context("BSON name without its end")?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.at += n + 1;
        Ok(s)
    }

    /// Fields up to the document's closing 0; `array` drops their names.
    fn fields(&mut self) -> Result<Vec<(String, Bson)>> {
        let start = self.at;
        let len = self.len()?;
        ensure!(len >= 5 && start + len <= self.b.len(), "BSON document of a wrong length");
        let end = start + len - 1;
        let mut out = Vec::new();
        while self.at < end {
            let kind = self.byte()?;
            let name = self.cstring()?;
            out.push((name, self.value(kind)?));
        }
        ensure!(self.at == end && self.byte()? == 0, "BSON document does not end where it says");
        Ok(out)
    }

    fn document(&mut self) -> Result<Document> {
        Ok(Document(self.fields()?))
    }

    fn value(&mut self, kind: u8) -> Result<Bson> {
        Ok(match kind {
            0x01 => Bson::Double(f64::from_le_bytes(self.take(8)?.try_into()?)),
            0x02 => {
                let len = self.len()?;
                ensure!(len >= 1, "BSON string of a wrong length");
                let bytes = self.take(len)?;
                Bson::String(String::from_utf8_lossy(&bytes[..len - 1]).into_owned())
            }
            0x03 => Bson::Document(self.document()?),
            0x04 => Bson::Array(self.fields()?.into_iter().map(|(_, v)| v).collect()),
            0x05 => {
                let len = self.len()?;
                let subtype = self.byte()?;
                Bson::Binary(subtype, self.take(len)?.to_vec())
            }
            0x07 => Bson::ObjectId(self.take(12)?.try_into()?),
            0x08 => Bson::Bool(self.byte()? != 0),
            0x09 => Bson::DateTime(self.i64()?),
            0x0A => Bson::Null,
            0x10 => Bson::Int32(self.i32()?),
            0x12 => Bson::Int64(self.i64()?),
            0x13 => Bson::Decimal(self.take(16)?.try_into()?),
            0xFF => Bson::MinValue,
            0x7F => Bson::MaxValue,
            other => bail!("BSON type {other:#04x} is not LiteDB's"),
        })
    }
}

/// Writes LiteDB 4 files for tests: documents in data pages, the big ones in
/// extend pages.
#[cfg(test)]
pub(crate) mod write {
    use super::*;

    fn cstring(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    fn value(out: &mut Vec<u8>, v: &Bson) -> u8 {
        match v {
            Bson::Double(n) => {
                out.extend_from_slice(&n.to_le_bytes());
                0x01
            }
            Bson::String(s) => {
                out.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
                cstring(out, s);
                0x02
            }
            Bson::Document(d) => {
                out.extend(document(d));
                0x03
            }
            Bson::Array(items) => {
                let d = Document(items.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect());
                out.extend(document(&d));
                0x04
            }
            Bson::Binary(subtype, bytes) => {
                out.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
                out.push(*subtype);
                out.extend_from_slice(bytes);
                0x05
            }
            Bson::ObjectId(b) => {
                out.extend_from_slice(b);
                0x07
            }
            Bson::Bool(b) => {
                out.push(u8::from(*b));
                0x08
            }
            Bson::DateTime(ms) => {
                out.extend_from_slice(&ms.to_le_bytes());
                0x09
            }
            Bson::Null => 0x0A,
            Bson::Int32(n) => {
                out.extend_from_slice(&n.to_le_bytes());
                0x10
            }
            Bson::Int64(n) => {
                out.extend_from_slice(&n.to_le_bytes());
                0x12
            }
            Bson::Decimal(b) => {
                out.extend_from_slice(b);
                0x13
            }
            Bson::MinValue => 0xFF,
            Bson::MaxValue => 0x7F,
        }
    }

    pub fn document(d: &Document) -> Vec<u8> {
        let mut body = Vec::new();
        for (k, v) in &d.0 {
            let mut bytes = Vec::new();
            let kind = value(&mut bytes, v);
            body.push(kind);
            cstring(&mut body, k);
            body.extend(bytes);
        }
        let mut out = ((body.len() + 5) as i32).to_le_bytes().to_vec();
        out.extend(body);
        out.push(0);
        out
    }

    /// A .NET `Guid`'s bytes of its text.
    pub fn guid_bytes(text: &str) -> Vec<u8> {
        let hex: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&text.replace('-', "")[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let mut b = hex.clone();
        b[0..4].reverse();
        b[4..6].reverse();
        b[6..8].reverse();
        b
    }

    fn page(id: u32, kind: u8, next: u32, items: u16) -> Vec<u8> {
        let mut p = vec![0u8; PAGE_SIZE];
        p[0..4].copy_from_slice(&id.to_le_bytes());
        p[4] = kind;
        p[5..9].copy_from_slice(&NO_PAGE.to_le_bytes());
        p[9..13].copy_from_slice(&next.to_le_bytes());
        p[13..15].copy_from_slice(&items.to_le_bytes());
        p
    }

    /// A file with `docs`: small ones in data pages, big ones in extend pages.
    pub fn file(docs: &[Document]) -> Vec<u8> {
        let mut pages = vec![page(0, 1, NO_PAGE, 0)];
        pages[0][PAGE_HEADER..PAGE_HEADER + HEADER_INFO.len()].copy_from_slice(HEADER_INFO);
        pages[0][PAGE_HEADER + HEADER_INFO.len()] = FILE_VERSION;
        // Blocks of the data page being filled: (extend page, bytes).
        let mut blocks: Vec<(u32, Vec<u8>)> = Vec::new();
        // Extend pages handed out so far.
        let mut extends = 0u32;
        let mut data_pages: Vec<Vec<(u32, Vec<u8>)>> = Vec::new();
        let room = PAGE_SIZE - PAGE_HEADER;
        let used = |b: &[(u32, Vec<u8>)]| b.iter().map(|(_, d)| BLOCK_HEADER + d.len()).sum::<usize>();
        for d in docs {
            let bytes = document(d);
            let block = if BLOCK_HEADER + bytes.len() > room { (u32::MAX - 1, bytes) } else { (NO_PAGE, bytes) };
            let size = if block.0 == NO_PAGE { BLOCK_HEADER + block.1.len() } else { BLOCK_HEADER };
            if used(&blocks) + size > room {
                data_pages.push(std::mem::take(&mut blocks));
            }
            blocks.push(block);
        }
        data_pages.push(blocks);
        let first_extend = 1 + data_pages.len() as u32;
        for blocks in &data_pages {
            let id = pages.len() as u32;
            let mut p = page(id, DATA_PAGE, NO_PAGE, blocks.len() as u16);
            let mut at = PAGE_HEADER;
            for (i, (extend, bytes)) in blocks.iter().enumerate() {
                let (extend, inline) = if *extend == NO_PAGE {
                    (NO_PAGE, bytes.as_slice())
                } else {
                    let start = first_extend + extends;
                    extends += bytes.chunks(room).count() as u32;
                    (start, &[][..])
                };
                p[at..at + 2].copy_from_slice(&(i as u16).to_le_bytes());
                p[at + 2..at + 6].copy_from_slice(&extend.to_le_bytes());
                p[at + 6..at + 8].copy_from_slice(&(inline.len() as u16).to_le_bytes());
                p[at + 8..at + 8 + inline.len()].copy_from_slice(inline);
                at += BLOCK_HEADER + inline.len();
            }
            pages.push(p);
        }
        // The extend pages, in the order they were handed out: a document's
        // chain runs through consecutive pages.
        let mut id = first_extend;
        for blocks in &data_pages {
            for (extend, bytes) in blocks {
                if *extend == NO_PAGE {
                    continue;
                }
                let chunks: Vec<&[u8]> = bytes.chunks(room).collect();
                for (i, chunk) in chunks.iter().enumerate() {
                    let next = if i + 1 < chunks.len() { id + 1 } else { NO_PAGE };
                    let mut p = page(id, EXTEND_PAGE, next, chunk.len() as u16);
                    p[PAGE_HEADER..PAGE_HEADER + chunk.len()].copy_from_slice(chunk);
                    pages.push(p);
                    id += 1;
                }
            }
        }
        pages.concat()
    }
}

#[cfg(test)]
mod tests {
    use super::write::{file, guid_bytes};
    use super::*;

    fn doc(fields: &[(&str, Bson)]) -> Document {
        Document(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn reads_every_kind_of_value() {
        let id = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        let d = doc(&[
            ("_id", Bson::Binary(4, guid_bytes(id))),
            ("Name", Bson::String("Hollow Knight".into())),
            ("IsInstalled", Bson::Bool(true)),
            ("Playtime", Bson::Int64(5430)),
            ("Small", Bson::Int32(7)),
            ("Score", Bson::Double(9.5)),
            ("LastActivity", Bson::DateTime(1_767_323_045_000)),
            ("Nothing", Bson::Null),
            ("Tags", Bson::Array(vec![Bson::String("a".into()), Bson::Int32(1)])),
            ("Inner", Bson::Document(doc(&[("x", Bson::MinValue), ("y", Bson::MaxValue)]))),
            ("Oid", Bson::ObjectId([1; 12])),
            ("Money", Bson::Decimal([2; 16])),
        ]);
        let back = document(&write::document(&d)).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.guid("_id").as_deref(), Some(id));
        assert_eq!((back.str("Name"), back.bool("IsInstalled")), (Some("Hollow Knight"), Some(true)));
        assert_eq!((back.int("Playtime"), back.int("Small"), back.int("Score")), (Some(5430), Some(7), Some(9)));
        assert_eq!(back.date("LastActivity"), Some(1_767_323_045_000));
        assert_eq!(back.str("Missing"), None);
    }

    #[test]
    fn guids_are_in_dotnets_byte_order() {
        let bytes = [0xe0, 0x04, 0x25, 0x3f, 0x89, 0x4f, 0xd3, 0x11, 0x9a, 0x0c, 0x03, 0x05, 0xe8, 0x2c, 0x33, 0x01];
        assert_eq!(guid(&bytes).as_deref(), Some("3f2504e0-4f89-11d3-9a0c-0305e82c3301"));
        assert_eq!(guid(&bytes[..15]), None);
    }

    #[test]
    fn reads_the_documents_of_a_file_big_ones_too() {
        let long = "x".repeat(PAGE_SIZE * 2 + 100);
        let mut docs: Vec<Document> = (0..120).map(|i| doc(&[("n", Bson::Int32(i)), ("pad", Bson::String("p".repeat(50)))])).collect();
        docs.insert(40, doc(&[("n", Bson::Int32(-1)), ("Description", Bson::String(long.clone()))]));
        let file = file(&docs);
        let mut read = documents(&file).unwrap();
        read.sort_by_key(|d| d.int("n"));
        assert_eq!(read.len(), docs.len());
        assert_eq!(read[0].str("Description").map(str::len), Some(long.len()));
        let mut want = docs.clone();
        want.sort_by_key(|d| d.int("n"));
        assert_eq!(read, want);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(documents(b"short").is_err());
        assert!(documents(&vec![0u8; PAGE_SIZE * 2]).is_err());
        let mut v5 = file(&[]);
        v5[PAGE_HEADER + HEADER_INFO.len()] = 8;
        assert!(documents(&v5).unwrap_err().to_string().contains("version 8"));
    }

    #[test]
    fn a_broken_document_is_left_out() {
        let mut f = file(&[doc(&[("n", Bson::Int32(1))]), doc(&[("n", Bson::Int32(2))])]);
        // The second block's BSON length, made wrong.
        let second = PAGE_SIZE + PAGE_HEADER + BLOCK_HEADER + write::document(&doc(&[("n", Bson::Int32(1))])).len() + BLOCK_HEADER;
        f[second] = 0xFF;
        let read = documents(&f).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].int("n"), Some(1));
    }
}
