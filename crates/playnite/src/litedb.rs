//! LiteDB v4 / datafile v7 reader (4 KiB pages). Port of `playnite/litedb.ts`,
//! itself derived from Playnite's `FileReaderV7`: read-only, no LiteDB
//! dependency.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, SecondsFormat, Utc};

pub const PAGE_SIZE: usize = 4096;
const HEADER_INFO: &str = "** This is a LiteDB file **";
const UINT32_MAX: u32 = 0xffff_ffff;
const MAX_DEPTH: usize = 100;

/// A BSON document (`BsonDocument`).
pub type BsonDoc = HashMap<String, Bson>;

/// The JSON-like values the reader yields. Dates become ISO strings, GUID
/// binaries become lower-case dashed strings, other binaries hex strings,
/// ObjectId/Decimal/Min/Max become `Null` (exactly like the TS).
#[derive(Debug, Clone, PartialEq)]
pub enum Bson {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Array(Vec<Bson>),
    Doc(BsonDoc),
}

/// `String(number)` for the values BSON can hold.
pub fn js_number_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if n == 0.0 {
        "0".into()
    } else if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i128)
    } else {
        format!("{n}")
    }
}

impl Bson {
    /// `asString`: non-empty strings, numbers stringified.
    pub fn as_string(&self) -> Option<String> {
        match self {
            Bson::Str(s) if !s.is_empty() => Some(s.clone()),
            Bson::Num(n) => Some(js_number_to_string(*n)),
            _ => None,
        }
    }

    /// `asNumber`: finite numbers, numeric strings, `0` otherwise.
    pub fn as_number(&self) -> f64 {
        match self {
            Bson::Num(n) if n.is_finite() => *n,
            Bson::Str(s) if !s.is_empty() => {
                let parsed = s.trim().parse::<f64>().unwrap_or(f64::NAN);
                if parsed.is_finite() { parsed } else { 0.0 }
            }
            _ => 0.0,
        }
    }

    pub fn as_bool(&self) -> bool {
        matches!(self, Bson::Bool(true))
    }

    pub fn as_doc(&self) -> Option<&BsonDoc> {
        match self {
            Bson::Doc(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_array(&self) -> &[Bson] {
        match self {
            Bson::Array(a) => a,
            _ => &[],
        }
    }

    /// `asGuid`: `asString` lower-cased.
    pub fn as_guid(&self) -> Option<String> {
        self.as_string().map(|s| s.to_lowercase())
    }
}

/// Accessors on `doc.get(key)` results, mirroring the TS helpers that accept
/// `undefined`.
pub trait DocExt {
    fn string(&self, key: &str) -> Option<String>;
    fn number(&self, key: &str) -> f64;
    fn boolean(&self, key: &str) -> bool;
    fn doc(&self, key: &str) -> Option<&BsonDoc>;
    fn array(&self, key: &str) -> &[Bson];
    fn guid(&self, key: &str) -> Option<String>;
}

impl DocExt for BsonDoc {
    fn string(&self, key: &str) -> Option<String> {
        self.get(key).and_then(Bson::as_string)
    }
    fn number(&self, key: &str) -> f64 {
        self.get(key).map(Bson::as_number).unwrap_or(0.0)
    }
    fn boolean(&self, key: &str) -> bool {
        self.get(key).is_some_and(Bson::as_bool)
    }
    fn doc(&self, key: &str) -> Option<&BsonDoc> {
        self.get(key).and_then(Bson::as_doc)
    }
    fn array(&self, key: &str) -> &[Bson] {
        self.get(key).map(Bson::as_array).unwrap_or(&[])
    }
    fn guid(&self, key: &str) -> Option<String> {
        self.get(key).and_then(Bson::as_guid)
    }
}

// ---------------------------------------------------------------------------
// Byte reader
// ---------------------------------------------------------------------------

struct ByteReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn need(&self, n: usize) -> Result<()> {
        if self.pos.checked_add(n).is_none_or(|end| end > self.buf.len()) {
            bail!("read past the end of the buffer");
        }
        Ok(())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.need(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(&self.buf[self.pos..self.pos + N]);
        self.pos += N;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.array()?))
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    /// `readBytes`: like `Buffer.subarray`, clamps to the buffer.
    fn bytes_clamped(&mut self, n: usize) -> &'a [u8] {
        let start = self.pos.min(self.buf.len());
        let end = self.pos.saturating_add(n).min(self.buf.len());
        self.pos = self.pos.saturating_add(n);
        &self.buf[start..end]
    }

    fn fixed_string(&mut self, n: usize) -> String {
        String::from_utf8_lossy(self.bytes_clamped(n)).into_owned()
    }

    /// `readBsonString`: int32 length including the trailing NUL.
    fn bson_string(&mut self) -> Result<String> {
        let len = self.i32()?;
        if len < 0 {
            bail!("negative string length");
        }
        let len = len as usize;
        let text_len = len.saturating_sub(1);
        let start = self.pos.min(self.buf.len());
        let end = self.pos.saturating_add(text_len).min(self.buf.len());
        let text = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
        self.pos = self.pos.saturating_add(len);
        Ok(text)
    }

    fn cstring(&mut self) -> String {
        let start = self.pos.min(self.buf.len());
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.buf[start..self.pos.min(self.buf.len())]).into_owned();
        self.pos += 1;
        text
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A .NET `Guid.ToByteArray()` (first three groups little-endian) as text.
pub fn guid_from_dotnet_bytes(b: &[u8; 16]) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{}-{}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        hex(&b[8..10]),
        hex(&b[10..16]),
    )
}

// ---------------------------------------------------------------------------
// BSON
// ---------------------------------------------------------------------------

fn read_document(r: &mut ByteReader, depth: usize) -> Result<BsonDoc> {
    if depth > MAX_DEPTH {
        bail!("document nested too deeply");
    }
    let len = r.i32()? as i64;
    let end = r.pos as i64 + len - 5;
    let mut doc = BsonDoc::new();
    while (r.pos as i64) < end {
        let (name, value) = read_element(r, depth)?;
        doc.insert(name, value);
    }
    r.u8()?;
    Ok(doc)
}

fn read_array(r: &mut ByteReader, depth: usize) -> Result<Vec<Bson>> {
    if depth > MAX_DEPTH {
        bail!("array nested too deeply");
    }
    let len = r.i32()? as i64;
    let end = r.pos as i64 + len - 5;
    let mut arr = Vec::new();
    while (r.pos as i64) < end {
        let (_, value) = read_element(r, depth)?;
        arr.push(value);
    }
    r.u8()?;
    Ok(arr)
}

fn read_element(r: &mut ByteReader, depth: usize) -> Result<(String, Bson)> {
    let ty = r.u8()?;
    let name = r.cstring();
    let value = match ty {
        0x01 => Bson::Num(r.f64()?),
        0x02 => Bson::Str(r.bson_string()?),
        0x03 => Bson::Doc(read_document(r, depth + 1)?),
        0x04 => Bson::Array(read_array(r, depth + 1)?),
        0x05 => {
            let len = r.i32()?;
            let sub_type = r.u8()?;
            if len < 0 {
                bail!("negative binary length");
            }
            let bytes = r.bytes_clamped(len as usize);
            if (sub_type == 0x03 || sub_type == 0x04) && len == 16 && bytes.len() == 16 {
                let mut b = [0u8; 16];
                b.copy_from_slice(bytes);
                Bson::Str(guid_from_dotnet_bytes(&b))
            } else {
                Bson::Str(hex(bytes))
            }
        }
        0x07 => {
            r.bytes_clamped(12);
            Bson::Null
        }
        0x08 => Bson::Bool(r.u8()? != 0),
        0x09 => {
            let ts = r.i64()?;
            if ts == 253_402_300_800_000 || ts == -62_135_596_800_000 {
                Bson::Null
            } else {
                let dt = DateTime::<Utc>::from_timestamp_millis(ts)
                    .ok_or_else(|| anyhow!("invalid date {ts}"))?;
                Bson::Str(dt.to_rfc3339_opts(SecondsFormat::Millis, true))
            }
        }
        0x0a => Bson::Null,
        0x10 => Bson::Num(r.i32()? as f64),
        0x12 => Bson::Num(r.i64()? as f64),
        0x13 => {
            r.bytes_clamped(16);
            Bson::Null
        }
        0xff | 0x7f => Bson::Null,
        other => bail!("Unsupported BSON type 0x{other:x}"),
    };
    Ok((name, value))
}

/// Decodes one BSON document.
pub fn deserialize_bson(data: &[u8]) -> Result<BsonDoc> {
    read_document(&mut ByteReader::new(data), 0)
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

struct DataBlock {
    extend_page_id: u32,
    data: Vec<u8>,
}

struct Page {
    page_type: u8,
    next_page_id: u32,
    blocks: Vec<DataBlock>,
    extend_data: Vec<u8>,
}

/// `Ok(None)` when the page lies beyond the file.
fn parse_page(file: &[u8], page_id: u32) -> Result<Option<Page>> {
    let offset = page_id as usize * PAGE_SIZE;
    if offset + PAGE_SIZE > file.len() {
        return Ok(None);
    }
    let mut r = ByteReader::new(&file[offset..offset + PAGE_SIZE]);
    let _page_id = r.u32()?;
    let page_type = r.u8()?;
    let _prev = r.u32()?;
    let next_page_id = r.u32()?;
    let item_count = r.u16()?;
    r.skip(2 + 8); // freeBytes + reserved
    let mut page = Page { page_type, next_page_id, blocks: Vec::new(), extend_data: Vec::new() };

    if page_type == 1 {
        let info = r.fixed_string(27);
        let ver = r.u8()?;
        if info != HEADER_INFO || ver != 7 {
            bail!("Not a LiteDB v4 (file version 7) database");
        }
        return Ok(Some(page));
    }

    if page_type == 4 {
        for _ in 0..item_count {
            let _index = r.u16()?;
            let extend_page_id = r.u32()?;
            let length = r.u16()? as usize;
            page.blocks.push(DataBlock { extend_page_id, data: r.bytes_clamped(length).to_vec() });
        }
    }

    if page_type == 5 {
        page.extend_data = r.bytes_clamped(item_count as usize).to_vec();
    }
    Ok(Some(page))
}

fn read_extend_data(file: &[u8], start_page_id: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut page_id = start_page_id;
    // A corrupt chain could loop forever; a file has at most len/PAGE_SIZE pages.
    let mut guard = file.len() / PAGE_SIZE + 1;
    while page_id != UINT32_MAX && guard > 0 {
        guard -= 1;
        let Some(page) = parse_page(file, page_id)? else { break };
        if page.page_type != 5 {
            break;
        }
        out.extend_from_slice(&page.extend_data);
        page_id = page.next_page_id;
    }
    Ok(out)
}

/// All documents of every collection in the file, in page order. Unreadable
/// blocks are skipped; a file that is not LiteDB v4 is an error.
pub fn read_litedb_documents(file: &[u8]) -> Result<Vec<BsonDoc>> {
    parse_page(file, 0)?;
    let page_count = file.len() / PAGE_SIZE;
    let mut documents = Vec::new();

    for page_id in 1..page_count as u32 {
        let Ok(Some(page)) = parse_page(file, page_id) else { continue };
        if page.page_type != 4 {
            continue;
        }
        for block in page.blocks {
            let data = if block.extend_page_id == UINT32_MAX {
                block.data
            } else {
                match read_extend_data(file, block.extend_page_id) {
                    Ok(d) => d,
                    Err(_) => continue,
                }
            };
            if data.is_empty() {
                continue;
            }
            if let Ok(doc) = deserialize_bson(&data) {
                documents.push(doc);
            }
        }
    }
    Ok(documents)
}

#[cfg(test)]
pub(crate) mod testkit {
    //! Builders for small synthetic LiteDB files (also used by other tests).
    use super::*;

    pub fn el(ty: u8, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![ty];
        v.extend_from_slice(name.as_bytes());
        v.push(0);
        v.extend_from_slice(payload);
        v
    }

    pub fn doc(elements: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = elements.concat();
        let mut v = ((body.len() + 5) as i32).to_le_bytes().to_vec();
        v.extend(body);
        v.push(0);
        v
    }

    pub fn s(name: &str, val: &str) -> Vec<u8> {
        let mut p = ((val.len() + 1) as i32).to_le_bytes().to_vec();
        p.extend_from_slice(val.as_bytes());
        p.push(0);
        el(0x02, name, &p)
    }
    pub fn i32e(name: &str, v: i32) -> Vec<u8> {
        el(0x10, name, &v.to_le_bytes())
    }
    pub fn i64e(name: &str, v: i64) -> Vec<u8> {
        el(0x12, name, &v.to_le_bytes())
    }
    pub fn dbl(name: &str, v: f64) -> Vec<u8> {
        el(0x01, name, &v.to_le_bytes())
    }
    pub fn boolean(name: &str, v: bool) -> Vec<u8> {
        el(0x08, name, &[v as u8])
    }
    pub fn date(name: &str, ms: i64) -> Vec<u8> {
        el(0x09, name, &ms.to_le_bytes())
    }
    pub fn null(name: &str) -> Vec<u8> {
        el(0x0a, name, &[])
    }
    pub fn sub(name: &str, d: Vec<u8>) -> Vec<u8> {
        el(0x03, name, &d)
    }
    pub fn arr(name: &str, items: &[Vec<u8>]) -> Vec<u8> {
        // Array elements are named "0", "1", ...
        let named: Vec<Vec<u8>> = items
            .iter()
            .enumerate()
            .map(|(i, e)| {
                // `e` was built with an empty name; rebuild with the index.
                let mut v = vec![e[0]];
                v.extend_from_slice(i.to_string().as_bytes());
                v.push(0);
                v.extend_from_slice(&e[2..]);
                v
            })
            .collect();
        let mut p = doc(&named);
        let mut out = Vec::new();
        out.append(&mut p);
        el(0x04, name, &out)
    }
    /// Binary subtype 4 (UUID) of a .NET `Guid.ToByteArray()`.
    pub fn guid(name: &str, text: &str) -> Vec<u8> {
        let b = guid_bytes(text);
        let mut p = 16i32.to_le_bytes().to_vec();
        p.push(0x04);
        p.extend_from_slice(&b);
        el(0x05, name, &p)
    }
    pub fn guid_bytes(text: &str) -> [u8; 16] {
        let h: String = text.chars().filter(|c| *c != '-').collect();
        let raw: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&[raw[3], raw[2], raw[1], raw[0]]);
        out[4..6].copy_from_slice(&[raw[5], raw[4]]);
        out[6..8].copy_from_slice(&[raw[7], raw[6]]);
        out[8..].copy_from_slice(&raw[8..]);
        out
    }

    fn page_header(page_id: u32, ty: u8, next: u32, item_count: u16) -> Vec<u8> {
        let mut p = Vec::with_capacity(PAGE_SIZE);
        p.extend_from_slice(&page_id.to_le_bytes());
        p.push(ty);
        p.extend_from_slice(&u32::MAX.to_le_bytes()); // prev
        p.extend_from_slice(&next.to_le_bytes());
        p.extend_from_slice(&item_count.to_le_bytes());
        p.extend_from_slice(&[0, 0]); // freeBytes
        p.extend_from_slice(&[0; 8]); // reserved
        p
    }

    fn pad(mut p: Vec<u8>) -> Vec<u8> {
        assert!(p.len() <= PAGE_SIZE, "page overflow");
        p.resize(PAGE_SIZE, 0);
        p
    }

    pub fn header_page() -> Vec<u8> {
        let mut p = page_header(0, 1, u32::MAX, 0);
        p.extend_from_slice(HEADER_INFO.as_bytes());
        p.push(7);
        pad(p)
    }

    /// Data page holding `docs` inline.
    pub fn data_page(page_id: u32, docs: &[Vec<u8>]) -> Vec<u8> {
        let mut p = page_header(page_id, 4, u32::MAX, docs.len() as u16);
        for (i, d) in docs.iter().enumerate() {
            p.extend_from_slice(&(i as u16).to_le_bytes());
            p.extend_from_slice(&u32::MAX.to_le_bytes());
            p.extend_from_slice(&(d.len() as u16).to_le_bytes());
            p.extend_from_slice(d);
        }
        pad(p)
    }

    /// A data page with one block pointing at an extend-page chain.
    pub fn data_page_extended(page_id: u32, first_extend: u32) -> Vec<u8> {
        let mut p = page_header(page_id, 4, u32::MAX, 1);
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&first_extend.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        pad(p)
    }

    pub fn extend_page(page_id: u32, next: u32, data: &[u8]) -> Vec<u8> {
        let mut p = page_header(page_id, 5, next, data.len() as u16);
        p.extend_from_slice(data);
        pad(p)
    }

    /// A whole file: header + the given data pages (already padded).
    pub fn file(pages: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = header_page();
        for p in pages {
            assert_eq!(p.len(), PAGE_SIZE);
            out.extend(p);
        }
        out
    }

    /// Convenience: header + one data page of `docs`.
    pub fn simple_db(docs: &[Vec<u8>]) -> Vec<u8> {
        file(vec![data_page(1, docs)])
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;

    #[test]
    fn parses_types_and_guids() {
        let gid = "7d26e552-7ba1-4f00-8cb0-6c72f1caf235";
        let d = doc(&[
            guid("_id", gid),
            s("Name", "Zelda ção"),
            i32e("A", -5),
            i64e("B", 3_000_000_000),
            dbl("C", 1.5),
            boolean("D", true),
            null("E"),
            date("F", 1_700_000_000_123),
            date("G", 253_402_300_800_000),
            sub("H", doc(&[s("x", "y")])),
            arr("I", &[s("", "a"), i32e("", 7)]),
        ]);
        let parsed = deserialize_bson(&d).unwrap();
        assert_eq!(parsed.guid("_id").unwrap(), gid);
        assert_eq!(parsed.string("Name").unwrap(), "Zelda ção");
        assert_eq!(parsed.number("A"), -5.0);
        assert_eq!(parsed.number("B"), 3e9);
        assert_eq!(parsed.number("C"), 1.5);
        assert!(parsed.boolean("D"));
        assert!(parsed.string("E").is_none());
        assert_eq!(parsed.string("F").unwrap(), "2023-11-14T22:13:20.123Z");
        assert_eq!(parsed.get("G"), Some(&Bson::Null));
        assert_eq!(parsed.doc("H").unwrap().string("x").unwrap(), "y");
        assert_eq!(parsed.array("I"), &[Bson::Str("a".into()), Bson::Num(7.0)]);
    }

    #[test]
    fn as_helpers_follow_the_ts() {
        assert_eq!(Bson::Num(42.0).as_string().as_deref(), Some("42"));
        assert_eq!(Bson::Num(1.5).as_string().as_deref(), Some("1.5"));
        assert_eq!(Bson::Str(String::new()).as_string(), None);
        assert_eq!(Bson::Str("12".into()).as_number(), 12.0);
        assert_eq!(Bson::Str("zz".into()).as_number(), 0.0);
        assert_eq!(Bson::Str("AbC".into()).as_guid().as_deref(), Some("abc"));
        assert!(!Bson::Num(1.0).as_bool());
    }

    #[test]
    fn reads_inline_and_extended_documents() {
        let small = doc(&[s("Name", "inline")]);
        let big_name = "x".repeat(6000);
        let big = doc(&[s("Name", &big_name)]);
        let (a, b) = big.split_at(3000);
        // pages: 1 = data(inline), 2 = data(extended -> 3 -> 4), 3/4 = extend
        let file = file(vec![
            data_page(1, &[small]),
            data_page_extended(2, 3),
            extend_page(3, 4, a),
            extend_page(4, u32::MAX, b),
        ]);
        let docs = read_litedb_documents(&file).unwrap();
        let names: Vec<String> = docs.iter().map(|d| d.string("Name").unwrap()).collect();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], "inline");
        assert_eq!(names[1].len(), 6000);
    }

    #[test]
    fn skips_garbage_blocks_and_rejects_foreign_files() {
        let good = doc(&[s("Name", "ok")]);
        let file = simple_db(&[vec![0xde, 0xad, 0xbe], good]);
        let docs = read_litedb_documents(&file).unwrap();
        assert_eq!(docs.len(), 1);

        let mut bad = file.clone();
        bad[32] = 0x58; // corrupt the header text
        assert!(read_litedb_documents(&bad).is_err());
        // shorter than a page: nothing to read, like the TS
        assert!(read_litedb_documents(&[1, 2, 3]).unwrap().is_empty());
    }

    #[test]
    fn unsupported_types_and_truncation_are_errors() {
        assert!(deserialize_bson(&doc(&[el(0x44, "x", &[])])).is_err());
        let mut d = doc(&[s("Name", "abcdef")]);
        d.truncate(d.len() - 4);
        assert!(deserialize_bson(&d).is_err());
    }
}
