//! The main icon of a Windows program (`.exe`), read from its resources
//! without Wine: the first `RT_GROUP_ICON` (the one Explorer shows) and, of
//! its pictures, the biggest. Shortcuts of programs run under Wine use it.

/// `RT_ICON` and `RT_GROUP_ICON` resource types.
const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The picture [`main_icon`] picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconImage {
    /// A PNG file, as stored in the program (most 256x256 icons).
    Png(Vec<u8>),
    /// An `.ico` file holding the one bitmap picture: the caller decodes it.
    Ico(Vec<u8>),
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// One section of the PE file: where its RVAs are in the file.
struct Section {
    rva: u32,
    size: u32,
    raw: u32,
    raw_size: u32,
}

struct Pe<'a> {
    bytes: &'a [u8],
    sections: Vec<Section>,
    /// File offset of the resource directory's root.
    rsrc: usize,
}

impl<'a> Pe<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(bytes, 0x3c)? as usize;
        if bytes.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let coff = pe + 4;
        let count = u16_at(bytes, coff + 2)? as usize;
        let opt_size = u16_at(bytes, coff + 16)? as usize;
        let opt = coff + 20;
        // The data directories start after the fields of PE32 / PE32+.
        let (rva_count_at, dirs_at) = match u16_at(bytes, opt)? {
            0x10b => (opt + 92, opt + 96),
            0x20b => (opt + 108, opt + 112),
            _ => return None,
        };
        if u32_at(bytes, rva_count_at)? < 3 {
            return None;
        }
        // Directory 2 is the resources.
        let rsrc_rva = u32_at(bytes, dirs_at + 2 * 8)?;
        if rsrc_rva == 0 {
            return None;
        }
        let table = opt + opt_size;
        let sections = (0..count.min(96))
            .map(|i| {
                let s = table + i * 40;
                Some(Section {
                    size: u32_at(bytes, s + 8)?,
                    rva: u32_at(bytes, s + 12)?,
                    raw_size: u32_at(bytes, s + 16)?,
                    raw: u32_at(bytes, s + 20)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mut this = Self { bytes, sections, rsrc: 0 };
        this.rsrc = this.offset(rsrc_rva)?;
        Some(this)
    }

    /// The file offset of an RVA.
    fn offset(&self, rva: u32) -> Option<usize> {
        let s = self.sections.iter().find(|s| rva >= s.rva && rva - s.rva < s.size.max(s.raw_size))?;
        let within = rva - s.rva;
        // Past the raw data is the zero fill of the section: nothing to read.
        (within < s.raw_size).then(|| (s.raw as usize).saturating_add(within as usize))
    }

    /// `(id, entry)` of a resource directory's entries named by number;
    /// `entry` is the high-bit-tagged offset (a subdirectory or a leaf).
    fn entries(&self, dir: usize) -> Vec<(u32, u32)> {
        let named = u16_at(self.bytes, dir + 12).unwrap_or(0) as usize;
        let ids = u16_at(self.bytes, dir + 14).unwrap_or(0) as usize;
        (0..(named + ids).min(4096))
            .filter_map(|i| {
                let e = dir + 16 + i * 8;
                Some((u32_at(self.bytes, e)?, u32_at(self.bytes, e + 4)?))
            })
            .collect()
    }

    fn subdir(&self, entry: u32) -> Option<usize> {
        (entry & 0x8000_0000 != 0).then(|| self.rsrc + (entry & 0x7fff_ffff) as usize)
    }

    /// The first leaf under `entry` (any name, any language), as bytes.
    fn first_data(&self, entry: u32) -> Option<&'a [u8]> {
        let mut entry = entry;
        // Type, name and language: never deeper than that.
        for _ in 0..3 {
            let Some(dir) = self.subdir(entry) else { break };
            entry = self.entries(dir).first()?.1;
        }
        if entry & 0x8000_0000 != 0 {
            return None;
        }
        let leaf = self.rsrc + entry as usize;
        let at = self.offset(u32_at(self.bytes, leaf)?)?;
        let len = u32_at(self.bytes, leaf + 4)? as usize;
        self.bytes.get(at..at.checked_add(len)?)
    }

    /// The directory of a resource type.
    fn type_dir(&self, kind: u32) -> Option<usize> {
        let (_, entry) = self.entries(self.rsrc).into_iter().find(|(id, _)| *id == kind)?;
        self.subdir(entry)
    }
}

/// One picture of an icon group.
struct Picture<'a> {
    /// Pixels a side; a group writes 256 as 0.
    side: u32,
    bits: u16,
    data: &'a [u8],
    /// The group's 14-byte entry, for an `.ico` of this picture.
    entry: &'a [u8],
}

/// The program's main icon; `None` when it has none or the file is not a
/// Windows program.
pub fn main_icon(exe: &[u8]) -> Option<IconImage> {
    let pe = Pe::parse(exe)?;
    let groups = pe.type_dir(RT_GROUP_ICON)?;
    let group = pe.first_data(pe.entries(groups).first()?.1)?;
    let icons = pe.entries(pe.type_dir(RT_ICON)?);
    let count = u16_at(group, 4)? as usize;
    let best = (0..count)
        .filter_map(|i| {
            let entry = group.get(6 + i * 14..6 + (i + 1) * 14)?;
            let id = u16_at(entry, 12)? as u32;
            let (_, e) = icons.iter().find(|(icon, _)| (*icon & 0x8000_0000 == 0) && *icon == id)?;
            let side = match entry[0] {
                0 => 256,
                w => w as u32,
            };
            Some(Picture { side, bits: u16_at(entry, 6)?, data: pe.first_data(*e)?, entry })
        })
        // The biggest, then the most colours, then a PNG over a bitmap.
        .max_by_key(|p| (p.side, p.bits, p.data.starts_with(PNG_MAGIC)))?;
    if best.data.starts_with(PNG_MAGIC) {
        return Some(IconImage::Png(best.data.to_vec()));
    }
    Some(IconImage::Ico(ico_file(&best)))
}

/// An `.ico` file of one picture: the 6-byte header, its 16-byte entry (the
/// group's entry with the data's offset in place of its id) and the data.
fn ico_file(p: &Picture) -> Vec<u8> {
    let mut out = Vec::with_capacity(22 + p.data.len());
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    out.extend_from_slice(&p.entry[..8]);
    out.extend_from_slice(&(p.data.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(p.data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PE32+ file with one section (`.rsrc` at RVA 0x1000, file offset
    /// 0x200) holding one icon group whose pictures are `pictures`
    /// (`(id, width, bits, data)`).
    fn exe(pictures: &[(u16, u8, u16, &[u8])]) -> Vec<u8> {
        const RVA: u32 = 0x1000;
        const RAW: usize = 0x200;
        // The resource section, laid out as directories, then leaves, then data.
        let mut rsrc: Vec<u8> = Vec::new();
        let dir = |out: &mut Vec<u8>, entries: &[(u32, u32)]| {
            out.extend_from_slice(&[0; 12]);
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for (id, off) in entries {
                out.extend_from_slice(&id.to_le_bytes());
                out.extend_from_slice(&off.to_le_bytes());
            }
        };
        let mut group = vec![0, 0, 1, 0];
        group.extend_from_slice(&(pictures.len() as u16).to_le_bytes());
        for (id, w, bits, data) in pictures {
            group.extend_from_slice(&[*w, *w, 0, 0]);
            group.extend_from_slice(&1u16.to_le_bytes());
            group.extend_from_slice(&bits.to_le_bytes());
            group.extend_from_slice(&(data.len() as u32).to_le_bytes());
            group.extend_from_slice(&id.to_le_bytes());
        }
        // Sizes of the fixed parts, to compute the offsets up front.
        let n = pictures.len() as u32;
        let dir_size = |entries: u32| 16 + 8 * entries;
        let root = 0u32;
        let icon_types = root + dir_size(2);
        let group_types = icon_types + dir_size(n);
        let icon_langs = group_types + dir_size(1);
        let group_lang = icon_langs + n * dir_size(1);
        let leaves = group_lang + dir_size(1);
        let data_at = leaves + (n + 1) * 16;
        let sub = |off: u32| off | 0x8000_0000;
        dir(&mut rsrc, &[(RT_ICON, sub(icon_types)), (RT_GROUP_ICON, sub(group_types))]);
        let names: Vec<(u32, u32)> =
            pictures.iter().enumerate().map(|(i, p)| (p.0 as u32, sub(icon_langs + i as u32 * dir_size(1)))).collect();
        dir(&mut rsrc, &names);
        dir(&mut rsrc, &[(1, sub(group_lang))]);
        for i in 0..n {
            dir(&mut rsrc, &[(1033, leaves + i * 16)]);
        }
        dir(&mut rsrc, &[(1033, leaves + n * 16)]);
        let mut blobs: Vec<&[u8]> = pictures.iter().map(|p| p.3).collect();
        blobs.push(&group);
        let mut at = data_at;
        for b in &blobs {
            rsrc.extend_from_slice(&(RVA + at).to_le_bytes());
            rsrc.extend_from_slice(&(b.len() as u32).to_le_bytes());
            rsrc.extend_from_slice(&[0; 8]);
            at += b.len() as u32;
        }
        for b in &blobs {
            rsrc.extend_from_slice(b);
        }

        let mut f = vec![0u8; RAW];
        f[..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f[0x40..0x44].copy_from_slice(b"PE\0\0");
        let coff = 0x44;
        f[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        f[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
        let opt = coff + 20;
        f[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        f[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        f[opt + 112 + 16..opt + 112 + 20].copy_from_slice(&RVA.to_le_bytes());
        f[opt + 112 + 20..opt + 112 + 24].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        let s = opt + 240;
        f[s..s + 5].copy_from_slice(b".rsrc");
        f[s + 8..s + 12].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        f[s + 12..s + 16].copy_from_slice(&RVA.to_le_bytes());
        f[s + 16..s + 20].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        f[s + 20..s + 24].copy_from_slice(&(RAW as u32).to_le_bytes());
        f.extend_from_slice(&rsrc);
        f
    }

    #[test]
    fn the_biggest_picture_of_the_first_group_is_picked() {
        let png = [PNG_MAGIC, b"the big one"].concat();
        let file = exe(&[(1, 16, 32, b"small bitmap"), (2, 0, 32, &png), (3, 48, 32, b"medium bitmap")]);
        assert_eq!(main_icon(&file), Some(IconImage::Png(png)));
    }

    #[test]
    fn a_bitmap_comes_back_as_an_ico_file_of_its_own() {
        let file = exe(&[(7, 32, 8, b"low colour"), (9, 32, 32, b"true colour")]);
        let Some(IconImage::Ico(ico)) = main_icon(&file) else { panic!("no icon") };
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        // Width 32, 32 bits, the data's size and offset, then the data.
        assert_eq!(ico[6], 32);
        assert_eq!(u16::from_le_bytes([ico[12], ico[13]]), 32);
        assert_eq!(u32::from_le_bytes(ico[14..18].try_into().unwrap()), 11);
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 22);
        assert_eq!(&ico[22..], b"true colour");
    }

    #[test]
    fn files_that_are_not_programs_have_no_icon() {
        assert_eq!(main_icon(b""), None);
        assert_eq!(main_icon(b"MZ"), None);
        assert_eq!(main_icon(&[PNG_MAGIC, &[0; 64]].concat()), None);
        // Cut anywhere, a program never panics.
        let file = exe(&[(1, 0, 32, &[PNG_MAGIC, b"x"].concat())]);
        for len in 0..file.len() {
            let _ = main_icon(&file[..len]);
        }
    }

    #[test]
    fn a_group_pointing_at_a_missing_picture_skips_it() {
        let mut file = exe(&[(1, 16, 32, b"only one")]);
        // Rename the picture so the group's id 1 finds nothing: the first
        // entry of the RT_ICON directory (the section's 0x200, after the
        // root's 32 bytes and the directory's 16).
        assert_eq!(file[0x200 + 48], 1);
        file[0x200 + 48] = 5;
        assert_eq!(main_icon(&file), None);
    }
}
