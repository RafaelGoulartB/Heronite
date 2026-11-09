//! The DLLs a Windows executable imports, read from its PE headers: the
//! import table and the delay-load table. Only the headers and the tables are
//! read, never the whole file (games' executables reach hundreds of MB).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Import descriptors read at most (real programs have a few dozen).
const MAX_DESCRIPTORS: usize = 512;

/// The DLL names `path` imports, lowercase, in table order, without
/// repeats. Not a PE file (or a damaged one): an error.
pub fn imports(path: &Path) -> io::Result<Vec<String>> {
    let mut f = File::open(path)?;
    imports_of(&mut f)
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("not a PE file: {what}"))
}

fn u16_at(b: &[u8], at: usize) -> io::Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| bad("short header"))
}

fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| bad("short header"))
}

struct Section {
    va: u32,
    size: u32,
    raw: u32,
}

struct Image {
    sections: Vec<Section>,
    image_base: u64,
}

impl Image {
    fn offset(&self, rva: u32) -> Option<u64> {
        self.sections.iter().find(|s| rva >= s.va && rva - s.va < s.size).map(|s| u64::from(rva - s.va) + u64::from(s.raw))
    }
}

fn read_at<R: Read + Seek>(f: &mut R, at: u64, buf: &mut [u8]) -> io::Result<usize> {
    f.seek(SeekFrom::Start(at))?;
    let mut read = 0;
    while read < buf.len() {
        match f.read(&mut buf[read..])? {
            0 => break,
            n => read += n,
        }
    }
    Ok(read)
}

fn c_string<R: Read + Seek>(f: &mut R, img: &Image, rva: u32) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = read_at(f, img.offset(rva)?, &mut buf).ok()?;
    let end = buf[..n].iter().position(|b| *b == 0)?;
    let name = std::str::from_utf8(&buf[..end]).ok()?;
    (!name.is_empty() && name.is_ascii()).then(|| name.to_ascii_lowercase())
}

/// What a Windows program's headers say, besides its imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// `IMAGE_FILE_HEADER.Machine`: [`MACHINE_X86`], [`MACHINE_X64`] or another.
    pub machine: u16,
    /// DLL names, as [`imports`] gives them.
    pub imports: Vec<String>,
    /// `SizeOfStackReserve`, in bytes.
    pub stack_reserve: u64,
    /// `ProductName` and `ProductVersion` of the version resource.
    pub product_name: Option<String>,
    pub product_version: Option<String>,
}

pub const MACHINE_X86: u16 = 0x14c;
pub const MACHINE_X64: u16 = 0x8664;

/// [`Info`] of `path`. Not a PE file: an error.
pub fn info(path: &Path) -> io::Result<Info> {
    let mut f = File::open(path)?;
    info_of(&mut f)
}

/// The headers and data directories the readers below need.
struct Headers {
    img: Image,
    machine: u16,
    stack_reserve: u64,
    /// `(rva, size)` of each data directory.
    dirs: Vec<(u32, u32)>,
}

impl Headers {
    fn dir(&self, i: usize) -> (u32, u32) {
        self.dirs.get(i).copied().unwrap_or((0, 0))
    }
}

fn headers<R: Read + Seek>(f: &mut R) -> io::Result<Headers> {
    let mut dos = [0u8; 64];
    if read_at(f, 0, &mut dos)? < 64 || &dos[..2] != b"MZ" {
        return Err(bad("no MZ header"));
    }
    let pe = u64::from(u32_at(&dos, 0x3c)?);
    let mut head = vec![0u8; 4096];
    let n = read_at(f, pe, &mut head)?;
    head.truncate(n);
    if head.get(..4) != Some(b"PE\0\0".as_slice()) {
        return Err(bad("no PE signature"));
    }
    let machine = u16_at(&head, 4)?;
    let sections = usize::from(u16_at(&head, 6)?);
    let opt_size = usize::from(u16_at(&head, 20)?);
    let opt = 24;
    let (dirs, count_at, image_base, stack_reserve) = match u16_at(&head, opt)? {
        0x10b => (opt + 96, opt + 92, u64::from(u32_at(&head, opt + 28)?), u64::from(u32_at(&head, opt + 72)?)),
        0x20b => {
            let lo = u64::from(u32_at(&head, opt + 24)?);
            let hi = u64::from(u32_at(&head, opt + 28)?);
            let stack = u64::from(u32_at(&head, opt + 72)?) | u64::from(u32_at(&head, opt + 76)?) << 32;
            (opt + 112, opt + 108, lo | hi << 32, stack)
        }
        _ => return Err(bad("unknown optional header")),
    };
    let dir_count = (u32_at(&head, count_at)? as usize).min(16);
    let dirs = (0..dir_count).map(|i| Ok((u32_at(&head, dirs + i * 8)?, u32_at(&head, dirs + i * 8 + 4)?))).collect::<io::Result<_>>()?;
    let table = opt + opt_size;
    let mut img = Image { sections: Vec::with_capacity(sections), image_base };
    for i in 0..sections.min(96) {
        let s = table + i * 40;
        let (vsize, va, raw_size, raw) = (u32_at(&head, s + 8)?, u32_at(&head, s + 12)?, u32_at(&head, s + 16)?, u32_at(&head, s + 20)?);
        img.sections.push(Section { va, size: vsize.max(raw_size), raw });
    }
    Ok(Headers { img, machine, stack_reserve, dirs })
}

pub(crate) fn imports_of<R: Read + Seek>(f: &mut R) -> io::Result<Vec<String>> {
    let h = headers(f)?;
    imports_with(f, &h)
}

pub(crate) fn info_of<R: Read + Seek>(f: &mut R) -> io::Result<Info> {
    let h = headers(f)?;
    let imports = imports_with(f, &h)?;
    // A damaged resource section only loses the version strings.
    let blob = version_blob(f, &h).ok().flatten().unwrap_or_default();
    Ok(Info {
        machine: h.machine,
        imports,
        stack_reserve: h.stack_reserve,
        product_name: version_string(&blob, "ProductName"),
        product_version: version_string(&blob, "ProductVersion"),
    })
}

fn imports_with<R: Read + Seek>(f: &mut R, h: &Headers) -> io::Result<Vec<String>> {
    let img = &h.img;
    let (import_rva, delay_rva) = (h.dir(1).0, h.dir(13).0);
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: Option<String>| {
        if let Some(n) = name.filter(|n| !names.contains(n)) {
            names.push(n);
        }
    };
    // IMAGE_IMPORT_DESCRIPTOR: 20 bytes, the name's RVA at 12.
    if let Some(at) = (import_rva != 0).then(|| img.offset(import_rva)).flatten() {
        for i in 0..MAX_DESCRIPTORS {
            let mut d = [0u8; 20];
            if read_at(f, at + (i * 20) as u64, &mut d)? < 20 || d.iter().all(|b| *b == 0) {
                break;
            }
            add(c_string(f, img, u32_at(&d, 12)?));
        }
    }
    // IMAGE_DELAYLOAD_DESCRIPTOR: 32 bytes, attributes at 0 (bit 0: RVAs,
    // else old-style addresses), the name at 4.
    if let Some(at) = (delay_rva != 0).then(|| img.offset(delay_rva)).flatten() {
        for i in 0..MAX_DESCRIPTORS {
            let mut d = [0u8; 32];
            if read_at(f, at + (i * 32) as u64, &mut d)? < 32 || d.iter().all(|b| *b == 0) {
                break;
            }
            let name = u32_at(&d, 4)?;
            let rva = match u32_at(&d, 0)? & 1 {
                1 => Some(name),
                _ => u64::from(name).checked_sub(img.image_base).and_then(|r| u32::try_from(r).ok()),
            };
            add(rva.and_then(|r| c_string(f, img, r)));
        }
    }
    Ok(names)
}

/// `RT_VERSION`, the resource type of `VS_VERSIONINFO`.
const RT_VERSION: u32 = 16;
/// Version resources are a few KB; anything bigger is not one.
const MAX_VERSION_SIZE: u32 = 64 * 1024;

/// The first version resource: resource directory, type `RT_VERSION`, its
/// first name, its first language.
fn version_blob<R: Read + Seek>(f: &mut R, h: &Headers) -> io::Result<Option<Vec<u8>>> {
    let (rsrc, _) = h.dir(2);
    let Some(base) = (rsrc != 0).then(|| h.img.offset(rsrc)).flatten() else { return Ok(None) };
    let Some(names) = resource_entry(f, base, 0, Some(RT_VERSION))? else { return Ok(None) };
    let Some(langs) = resource_entry(f, base, names & 0x7fff_ffff, None)? else { return Ok(None) };
    let Some(data) = resource_entry(f, base, langs & 0x7fff_ffff, None)? else { return Ok(None) };
    if data & 0x8000_0000 != 0 {
        return Ok(None);
    }
    // IMAGE_RESOURCE_DATA_ENTRY: RVA, size.
    let mut entry = [0u8; 8];
    read_at(f, base + u64::from(data), &mut entry)?;
    let (rva, size) = (u32_at(&entry, 0)?, u32_at(&entry, 4)?.min(MAX_VERSION_SIZE));
    let Some(at) = h.img.offset(rva) else { return Ok(None) };
    let mut blob = vec![0u8; size as usize];
    let n = read_at(f, at, &mut blob)?;
    blob.truncate(n);
    Ok(Some(blob))
}

/// The `OffsetToData` of an entry of the resource directory at `dir`
/// (relative to the section): the one with id `id`, or the first one.
fn resource_entry<R: Read + Seek>(f: &mut R, base: u64, dir: u32, id: Option<u32>) -> io::Result<Option<u32>> {
    let mut head = [0u8; 16];
    if read_at(f, base + u64::from(dir), &mut head)? < 16 {
        return Ok(None);
    }
    let count = usize::from(u16_at(&head, 12)?) + usize::from(u16_at(&head, 14)?);
    for i in 0..count.min(256) {
        let mut e = [0u8; 8];
        if read_at(f, base + u64::from(dir) + 16 + (i * 8) as u64, &mut e)? < 8 {
            break;
        }
        let (name, offset) = (u32_at(&e, 0)?, u32_at(&e, 4)?);
        if id.is_none_or(|id| name == id) {
            return Ok(Some(offset));
        }
    }
    Ok(None)
}

/// The value of `key` in the string tables of a `VS_VERSIONINFO` blob. Each
/// `String` holds its UTF-16 key, padding to 32 bits, then the value.
fn version_string(blob: &[u8], key: &str) -> Option<String> {
    let mut needle: Vec<u8> = key.encode_utf16().flat_map(u16::to_le_bytes).collect();
    needle.extend([0, 0]);
    let at = blob.windows(needle.len()).position(|w| w == needle.as_slice())?;
    let start = (at + needle.len()).next_multiple_of(4);
    let units: Vec<u16> =
        blob.get(start..)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0).take(256).collect();
    let value = String::from_utf16_lossy(&units).trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    /// A minimal PE32+ image: one section at RVA 0x1000 (file 0x400) holding
    /// the import table of `dlls` and the delay-load table of `delayed`.
    pub(crate) fn tiny_pe(dlls: &[&str], delayed: &[&str]) -> Vec<u8> {
        let mut f = vec![0u8; 0x400];
        f[..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        let pe = 0x80;
        f[pe..pe + 4].copy_from_slice(b"PE\0\0");
        f[pe + 4..pe + 6].copy_from_slice(&0x8664u16.to_le_bytes());
        f[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes());
        let opt_size = 240u16;
        f[pe + 20..pe + 22].copy_from_slice(&opt_size.to_le_bytes());
        let opt = pe + 24;
        f[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        f[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        f[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        // The section's content: descriptors, then the names.
        let mut sec = vec![0u8; 0x200];
        let imports_at = 0usize;
        let delay_at = (dlls.len() + 1) * 20;
        let mut names_at = delay_at + (delayed.len() + 1) * 32;
        for (i, d) in dlls.iter().enumerate() {
            let rva = 0x1000 + names_at as u32;
            sec[imports_at + i * 20 + 12..imports_at + i * 20 + 16].copy_from_slice(&rva.to_le_bytes());
            sec[names_at..names_at + d.len()].copy_from_slice(d.as_bytes());
            names_at += d.len() + 1;
        }
        for (i, d) in delayed.iter().enumerate() {
            let rva = 0x1000 + names_at as u32;
            let at = delay_at + i * 32;
            sec[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
            sec[at + 4..at + 8].copy_from_slice(&rva.to_le_bytes());
            sec[names_at..names_at + d.len()].copy_from_slice(d.as_bytes());
            names_at += d.len() + 1;
        }
        let dirs = opt + 112;
        f[dirs + 8..dirs + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        f[dirs + 13 * 8..dirs + 13 * 8 + 4].copy_from_slice(&(0x1000 + delay_at as u32).to_le_bytes());
        let s = opt + usize::from(opt_size);
        f[s..s + 5].copy_from_slice(b".idat");
        f[s + 8..s + 12].copy_from_slice(&0x200u32.to_le_bytes());
        f[s + 12..s + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        f[s + 16..s + 20].copy_from_slice(&0x200u32.to_le_bytes());
        f[s + 20..s + 24].copy_from_slice(&0x400u32.to_le_bytes());
        f.extend(sec);
        f
    }

    /// `tiny_pe` with a machine, a stack size and, with `product`, a
    /// second section holding a version resource.
    pub(crate) fn pe_with(machine: u16, dlls: &[&str], product: Option<&str>, stack: u64) -> Vec<u8> {
        let mut f = tiny_pe(dlls, &[]);
        f[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        let opt = 0x80 + 24;
        f[opt + 72..opt + 80].copy_from_slice(&stack.to_le_bytes());
        let Some(product) = product else { return f };
        // .rsrc at RVA 0x2000, file 0x600.
        let mut rsrc = vec![0u8; 0x400];
        let dir = |r: &mut Vec<u8>, at: usize, id: u32, to: u32| {
            r[at + 14..at + 16].copy_from_slice(&1u16.to_le_bytes());
            r[at + 16..at + 20].copy_from_slice(&id.to_le_bytes());
            r[at + 20..at + 24].copy_from_slice(&to.to_le_bytes());
        };
        dir(&mut rsrc, 0, 16, 0x8000_0000 | 0x18);
        dir(&mut rsrc, 0x18, 1, 0x8000_0000 | 0x30);
        dir(&mut rsrc, 0x30, 0x409, 0x48);
        // VS_VERSIONINFO: just the String entry the reader looks for.
        let mut blob = vec![0u8; 6];
        blob.extend("ProductName\0".encode_utf16().flat_map(u16::to_le_bytes));
        while !blob.len().is_multiple_of(4) {
            blob.push(0);
        }
        blob.extend(format!("{product}\0").encode_utf16().flat_map(u16::to_le_bytes));
        rsrc[0x48..0x4c].copy_from_slice(&(0x2000u32 + 0x60).to_le_bytes());
        rsrc[0x4c..0x50].copy_from_slice(&(blob.len() as u32).to_le_bytes());
        rsrc[0x60..0x60 + blob.len()].copy_from_slice(&blob);
        let pe = 0x80;
        f[pe + 6..pe + 8].copy_from_slice(&2u16.to_le_bytes());
        let dirs = opt + 112;
        f[dirs + 16..dirs + 20].copy_from_slice(&0x2000u32.to_le_bytes());
        f[dirs + 20..dirs + 24].copy_from_slice(&0x400u32.to_le_bytes());
        let s = opt + 240 + 40;
        f[s..s + 5].copy_from_slice(b".rsrc");
        f[s + 8..s + 12].copy_from_slice(&0x400u32.to_le_bytes());
        f[s + 12..s + 16].copy_from_slice(&0x2000u32.to_le_bytes());
        f[s + 16..s + 20].copy_from_slice(&0x400u32.to_le_bytes());
        f[s + 20..s + 24].copy_from_slice(&0x600u32.to_le_bytes());
        f.extend(rsrc);
        f
    }

    #[test]
    fn reads_the_machine_the_stack_and_the_product_name() {
        let x64 = info_of(&mut Cursor::new(pe_with(MACHINE_X64, &["dxgi.dll"], Some("ReShade"), 0x10_0000))).unwrap();
        assert_eq!(x64.machine, MACHINE_X64);
        assert_eq!(x64.imports, ["dxgi.dll"]);
        assert_eq!(x64.stack_reserve, 0x10_0000);
        assert_eq!(x64.product_name.as_deref(), Some("ReShade"));
        let x86 = info_of(&mut Cursor::new(pe_with(MACHINE_X86, &[], None, 0x4000))).unwrap();
        assert_eq!((x86.machine, x86.product_name), (MACHINE_X86, None));
    }

    #[test]
    fn version_strings_are_aligned_after_the_key() {
        let mut blob = vec![0u8; 2];
        blob.extend("ProductName\0".encode_utf16().flat_map(u16::to_le_bytes));
        blob.extend([0, 0]);
        blob.extend("DXVK\0".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(version_string(&blob, "ProductName").as_deref(), Some("DXVK"));
        assert_eq!(version_string(&blob, "CompanyName"), None);
    }

    #[test]
    fn reads_the_import_and_delay_load_tables() {
        let pe = tiny_pe(&["KERNEL32.dll", "d3d12.dll", "KERNEL32.dll"], &["XINPUT1_3.dll"]);
        let names = imports_of(&mut Cursor::new(pe)).unwrap();
        assert_eq!(names, ["kernel32.dll", "d3d12.dll", "xinput1_3.dll"]);
    }

    #[test]
    fn other_files_are_errors() {
        assert!(imports_of(&mut Cursor::new(b"#!/bin/sh\necho hi\n".to_vec())).is_err());
        assert!(imports_of(&mut Cursor::new(vec![0u8; 10])).is_err());
        let mut no_pe = tiny_pe(&["a.dll"], &[]);
        no_pe[0x80] = b'X';
        assert!(imports_of(&mut Cursor::new(no_pe)).is_err());
    }
}
