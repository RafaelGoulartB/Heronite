//! A picture's size from its first bytes, without the rest: what a source
//! whose database gives no sizes (LaunchBox) reads to learn the shape of
//! its box art, and what the Metadata dialog shows of each picture. PNG
//! says it in its first 24 bytes, WebP in its first 30; a JPEG in its frame
//! header, after the metadata segments (EXIF, ICC, a Photoshop thumbnail),
//! usually within the first 16 KB. The size decides the cover family box
//! art belongs to ([`family_of`]).

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use heronite_data::cover_family::CoverFamily;

use crate::http::{Http, APP_UA};

/// The most of a picture read to learn its size.
pub const HEAD_MAX: usize = 512 * 1024;
/// How long reading a picture's size over the network may take.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Width / height below which box art is a tall cover and up to which it
/// is a square one; above [`WIDE_FROM`] it is wide. The middles (in the
/// ratio) of 2:3..5:7 and 1:1, and of 1:1 and 16:9.
const SQUARE_FROM: f32 = 0.85;
const WIDE_FROM: f32 = 1.33;

/// The cover family a picture of `width` x `height` belongs to.
pub fn family_of(width: u32, height: u32) -> CoverFamily {
    let ratio = width as f32 / height.max(1) as f32;
    if ratio < SQUARE_FROM {
        CoverFamily::Tall
    } else if ratio < WIDE_FROM {
        CoverFamily::Square
    } else {
        CoverFamily::Wide
    }
}

/// What the bytes read so far say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    /// Width and height, in pixels.
    Size(u32, u32),
    /// A picture whose size comes later: read on.
    More,
    /// Not a PNG or JPEG, or a broken one: no use reading on.
    Unknown,
}

const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The size of the picture at `url`, from the first bytes of the answer
/// (read until [`read_head`] knows, at most [`HEAD_MAX`]; the rest is
/// dropped, for servers that ignore `Range`). `Err`: no answer, or one to
/// ask again (a server error, a timeout, too many requests); `Ok(None)`:
/// no picture whose size can be read there (gone, refused, not a picture).
pub fn fetch_size(http: &Http, url: &str) -> Result<Option<(u32, u32)>> {
    let r = http.get(url).header("User-Agent", APP_UA).timeout(HEAD_TIMEOUT).send_head(HEAD_MAX, |head| read_head(head) != Head::More)?;
    match r.status {
        _ if r.ok() => Ok(picture_size(&r.body)),
        408 | 429 | 500.. => anyhow::bail!("HTTP {}", r.status),
        _ => Ok(None),
    }
}

/// The size of the picture in the file `path`, from its first bytes.
pub fn file_size(path: &Path) -> Option<(u32, u32)> {
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(HEAD_MAX as u64).read_to_end(&mut head).ok()?;
    picture_size(&head)
}

/// What `head` (a file's first bytes) says of its size.
pub fn read_head(head: &[u8]) -> Head {
    if head.starts_with(&[0xff, 0xd8]) {
        return jpeg(head);
    }
    if head.starts_with(&PNG) {
        return png(head);
    }
    if head.starts_with(b"RIFF") {
        return webp(head);
    }
    if PNG.starts_with(head) || b"RIFF".starts_with(head) || head == [0xff] {
        Head::More
    } else {
        Head::Unknown
    }
}

/// [`read_head`]'s size, when it has one.
pub fn picture_size(head: &[u8]) -> Option<(u32, u32)> {
    match read_head(head) {
        Head::Size(w, h) => Some((w, h)),
        _ => None,
    }
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn be16(b: &[u8], at: usize) -> u32 {
    u32::from(u16::from_be_bytes([b[at], b[at + 1]]))
}

fn sized(w: u32, h: u32) -> Head {
    if w > 0 && h > 0 {
        Head::Size(w, h)
    } else {
        Head::Unknown
    }
}

/// The `IHDR` chunk comes first.
fn png(head: &[u8]) -> Head {
    if head.len() < 24 {
        return Head::More;
    }
    if &head[12..16] != b"IHDR" {
        return Head::Unknown;
    }
    sized(be32(head, 16), be32(head, 20))
}

/// A RIFF file of WebP: its first chunk says the size, each kind its way
/// (`VP8X` the canvas, `VP8L` a lossless frame, `VP8 ` a lossy one).
fn webp(head: &[u8]) -> Head {
    if head.len() < 16 {
        return Head::More;
    }
    if &head[8..12] != b"WEBP" {
        return Head::Unknown;
    }
    let le24 = |at: usize| u32::from_le_bytes([head[at], head[at + 1], head[at + 2], 0]);
    match &head[12..16] {
        b"VP8X" if head.len() >= 30 => sized(le24(24) + 1, le24(27) + 1),
        b"VP8L" if head.len() >= 25 => {
            if head[20] != 0x2f {
                return Head::Unknown;
            }
            let bits = u32::from_le_bytes([head[21], head[22], head[23], head[24]]);
            sized((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
        }
        b"VP8 " if head.len() >= 30 => {
            if head[23..26] != [0x9d, 0x01, 0x2a] {
                return Head::Unknown;
            }
            let le16 = |at: usize| u32::from(u16::from_le_bytes([head[at], head[at + 1]])) & 0x3fff;
            sized(le16(26), le16(28))
        }
        b"VP8X" | b"VP8L" | b"VP8 " => Head::More,
        _ => Head::Unknown,
    }
}

/// The segments one after the other until a frame header (`SOF0`..`SOF15`
/// but the three markers among them that are not frames).
fn jpeg(head: &[u8]) -> Head {
    let mut at = 2;
    loop {
        // A marker may be padded with any number of 0xFF.
        while at + 1 < head.len() && head[at] == 0xff && head[at + 1] == 0xff {
            at += 1;
        }
        match head.get(at) {
            None => return Head::More,
            Some(0xff) => {}
            Some(_) => return Head::Unknown,
        }
        if at + 4 > head.len() {
            return Head::More;
        }
        let marker = head[at + 1];
        match marker {
            // Markers without a length.
            0x01 | 0xd0..=0xd7 => {
                at += 2;
                continue;
            }
            // The image data or its end before any frame header.
            0xd9 | 0xda => return Head::Unknown,
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                if at + 9 > head.len() {
                    return Head::More;
                }
                return sized(be16(head, at + 7), be16(head, at + 5));
            }
            _ => {}
        }
        let len = be16(head, at + 2) as usize;
        if len < 2 {
            return Head::Unknown;
        }
        at += 2 + len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32) -> Vec<u8> {
        let mut b = PNG.to_vec();
        b.extend([0, 0, 0, 13]);
        b.extend(b"IHDR");
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 2, 0, 0, 0]);
        b
    }

    /// A JPEG start: an APP1 segment of `app_len` bytes, then a frame header.
    fn jpeg_of(w: u16, h: u16, app_len: u16, frame: u8) -> Vec<u8> {
        let mut b = vec![0xff, 0xd8, 0xff, 0xe1];
        b.extend(app_len.to_be_bytes());
        b.extend(vec![0u8; app_len as usize - 2]);
        b.extend([0xff, 0xff, 0xff, 0xdb, 0x00, 0x04, 0x00, 0x00]); // padding, then a DQT of nothing
        b.extend([0xff, frame, 0x00, 0x11, 0x08]);
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        b
    }

    #[test]
    fn png_sizes() {
        assert_eq!(picture_size(&png_of(1024, 1024)), Some((1024, 1024)));
        assert_eq!(read_head(&png_of(2460, 1716)[..20]), Head::More);
        assert_eq!(read_head(&png_of(10, 10)[..3]), Head::More);
        assert_eq!(read_head(&png_of(0, 10)), Head::Unknown);
    }

    #[test]
    fn jpeg_sizes_after_the_metadata() {
        let big = jpeg_of(4209, 2937, 15_000, 0xc0);
        assert_eq!(picture_size(&big), Some((4209, 2937)));
        assert_eq!(picture_size(&jpeg_of(1280, 2302, 40, 0xc2)), Some((1280, 2302)), "progressive");
        let frame = big.len() - 19;
        assert_eq!(big[frame..frame + 2], [0xff, 0xc0]);
        for cut in [0, 1, 2, 3, 100, 15_000, frame + 8] {
            assert_eq!(read_head(&big[..cut]), Head::More, "cut at {cut}");
        }
        // A DHT (0xC4) is no frame: its length is skipped like any segment's.
        let mut dht = vec![0xff, 0xd8, 0xff, 0xc4, 0x00, 0x04, 0x00, 0x00];
        dht.extend(&jpeg_of(300, 200, 10, 0xc0)[2..]);
        assert_eq!(picture_size(&dht), Some((300, 200)));
    }

    fn webp_of(chunk: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        b.extend(chunk);
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        b
    }

    #[test]
    fn webp_sizes() {
        // VP8X: flags, then the canvas size minus one, 24 bits each.
        let x = webp_of(b"VP8X", &[0, 0, 0, 0, 0xff, 0x01, 0x00, 0xd6, 0x00, 0x00]);
        assert_eq!(picture_size(&x), Some((512, 215)));
        // VP8L: the signature, then 14 bits of width minus one and of height minus one.
        let (w, h) = (920u32 - 1, 430u32 - 1);
        let bits = w | (h << 14);
        let mut l = vec![0x2f];
        l.extend(bits.to_le_bytes());
        assert_eq!(picture_size(&webp_of(b"VP8L", &l)), Some((920, 430)));
        // VP8: a frame tag, the start code, then width and height.
        let mut v = vec![0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a];
        v.extend(600u16.to_le_bytes());
        v.extend(900u16.to_le_bytes());
        assert_eq!(picture_size(&webp_of(b"VP8 ", &v)), Some((600, 900)));
        assert_eq!(read_head(&x[..20]), Head::More);
        assert_eq!(read_head(b"RIF"), Head::More);
        assert_eq!(read_head(b"RIFF\0\0\0\0WAVEfmt "), Head::Unknown, "a RIFF of sound");
    }

    #[test]
    fn a_file_reads_its_start_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cover.png");
        let mut bytes = png_of(600, 900);
        bytes.extend(vec![0u8; HEAD_MAX * 2]);
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(file_size(&path), Some((600, 900)));
        assert_eq!(file_size(&dir.path().join("missing.png")), None);
    }

    #[test]
    fn anything_else_is_unknown() {
        assert_eq!(read_head(b"<html>busy</html>"), Head::Unknown);
        assert_eq!(read_head(b"GIF89a\x10\x00"), Head::Unknown);
        assert_eq!(read_head(&[0xff, 0xd8, 0xff, 0xda, 0x00, 0x04, 0, 0]), Head::Unknown, "image data before a frame");
        assert_eq!(read_head(&[0xff, 0xd8, 0x00, 0x00, 0x00]), Head::Unknown);
        assert_eq!(read_head(&[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x01, 0, 0]), Head::Unknown, "a length below its own");
    }
}
