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

pub(crate) fn imports_of<R: Read + Seek>(f: &mut R) -> io::Result<Vec<String>> {
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
    let sections = usize::from(u16_at(&head, 6)?);
    let opt_size = usize::from(u16_at(&head, 20)?);
    let opt = 24;
    let (dirs, count_at, image_base) = match u16_at(&head, opt)? {
        0x10b => (opt + 96, opt + 92, u64::from(u32_at(&head, opt + 28)?)),
        0x20b => {
            let lo = u64::from(u32_at(&head, opt + 24)?);
            let hi = u64::from(u32_at(&head, opt + 28)?);
            (opt + 112, opt + 108, lo | hi << 32)
        }
        _ => return Err(bad("unknown optional header")),
    };
    let dir_count = u32_at(&head, count_at)? as usize;
    let dir = |i: usize| -> io::Result<u32> {
        if i < dir_count {
            u32_at(&head, dirs + i * 8)
        } else {
            Ok(0)
        }
    };
    let (import_rva, delay_rva) = (dir(1)?, dir(13)?);
    let table = opt + opt_size;
    let mut img = Image { sections: Vec::with_capacity(sections), image_base };
    for i in 0..sections.min(96) {
        let s = table + i * 40;
        let (vsize, va, raw_size, raw) = (u32_at(&head, s + 8)?, u32_at(&head, s + 12)?, u32_at(&head, s + 16)?, u32_at(&head, s + 20)?);
        img.sections.push(Section { va, size: vsize.max(raw_size), raw });
    }
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
            add(c_string(f, &img, u32_at(&d, 12)?));
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
            add(rva.and_then(|r| c_string(f, &img, r)));
        }
    }
    Ok(names)
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
