//! Pictures that move: a video, or an animated GIF, PNG (APNG) or WebP, told
//! apart from a still picture by their first bytes. An animated picture
//! decodes here as its first frame like any other; a video does too when
//! [`crate::Opts::video_still`] gives its first frame (ffmpeg, in the app).

use std::io::Read;
use std::path::Path;

/// How much of a file [`file_motion`] reads: enough for the chunks and
/// extensions that say a picture is animated, which come before its pixels.
pub const HEAD_BYTES: usize = 64 * 1024;

/// What kind of moving picture a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// A video (WebM / Matroska, MP4 / QuickTime).
    Video,
    Gif,
    Apng,
    WebP,
}

/// Brands of an ISO media file (`ftyp`) that are still pictures (AVIF,
/// HEIF), not videos.
const STILL_BRANDS: [&[u8; 4]; 6] = [b"avif", b"avis", b"mif1", b"msf1", b"heic", b"heix"];

/// Whether `head` (a file's first bytes) is a video container.
pub fn is_video(head: &[u8]) -> bool {
    // EBML: WebM and Matroska.
    if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return true;
    }
    // ISO base media: MP4, M4V, QuickTime; AVIF and HEIF are pictures.
    head.len() >= 12 && &head[4..8] == b"ftyp" && !STILL_BRANDS.iter().any(|b| &head[8..12] == *b)
}

/// The kind of moving picture `head` (a file's first bytes, up to
/// [`HEAD_BYTES`]) is; `None` for a still picture or anything else.
pub fn motion(head: &[u8]) -> Option<Motion> {
    if is_video(head) {
        return Some(Motion::Video);
    }
    if head.starts_with(b"GIF8") {
        // The looping extension, or a second frame, makes it move.
        return (find(head, b"NETSCAPE2.0") || gif_frames(head) > 1).then_some(Motion::Gif);
    }
    if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return png_animated(head).then_some(Motion::Apng);
    }
    if head.len() >= 21 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" && &head[12..16] == b"VP8X" {
        // VP8X flags: bit 1 is "animation".
        return (head[20] & 0x02 != 0).then_some(Motion::WebP);
    }
    None
}

/// [`motion`] of the file at `path` (its first [`HEAD_BYTES`]); `None` when
/// it cannot be read.
pub fn file_motion(path: &Path) -> Option<Motion> {
    let mut head = Vec::with_capacity(HEAD_BYTES);
    std::fs::File::open(path).ok()?.take(HEAD_BYTES as u64).read_to_end(&mut head).ok()?;
    motion(&head)
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// An APNG has an `acTL` chunk before its first `IDAT`.
fn png_animated(head: &[u8]) -> bool {
    let mut at = 8;
    while at + 8 <= head.len() {
        let len = u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]) as usize;
        match &head[at + 4..at + 8] {
            b"acTL" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        at = at.saturating_add(12).saturating_add(len);
    }
    false
}

/// Image descriptors (`0x2c`) of a GIF within `head`, walking its blocks; at
/// most 2 are counted.
fn gif_frames(head: &[u8]) -> usize {
    if head.len() < 13 {
        return 0;
    }
    let mut at = 13;
    // Global colour table.
    if head[10] & 0x80 != 0 {
        at += 3 << ((head[10] & 0x07) + 1);
    }
    let skip_blocks = |mut at: usize| -> Option<usize> {
        loop {
            let n = *head.get(at)? as usize;
            at += 1 + n;
            if n == 0 {
                return Some(at);
            }
        }
    };
    let mut frames = 0;
    while let Some(&b) = head.get(at) {
        match b {
            0x2c => {
                frames += 1;
                if frames > 1 {
                    break;
                }
                let Some(flags) = head.get(at + 9) else { break };
                at += 10;
                if flags & 0x80 != 0 {
                    at += 3 << ((flags & 0x07) + 1);
                }
                // LZW minimum code size, then the data blocks.
                let Some(next) = skip_blocks(at + 1) else { break };
                at = next;
            }
            0x21 => {
                let Some(next) = skip_blocks(at + 2) else { break };
                at = next;
            }
            _ => break,
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(chunks: &[&[u8; 4]]) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        for c in chunks {
            v.extend_from_slice(&0u32.to_be_bytes());
            v.extend_from_slice(*c);
            v.extend_from_slice(&[0; 4]);
        }
        v
    }

    fn webp(flags: u8) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        v.extend_from_slice(&[10, 0, 0, 0, flags, 0, 0, 0]);
        v
    }

    #[test]
    fn videos_are_told_from_pictures() {
        assert_eq!(motion(&[0x1a, 0x45, 0xdf, 0xa3, 1, 2]), Some(Motion::Video));
        assert_eq!(motion(b"\0\0\0\x20ftypisom\0\0\0\0"), Some(Motion::Video));
        assert_eq!(motion(b"\0\0\0\x20ftypqt  \0\0\0\0"), Some(Motion::Video));
        assert_eq!(motion(b"\0\0\0\x20ftypavif\0\0\0\0"), None, "AVIF is a picture");
        assert_eq!(motion(&[0xff, 0xd8, 0xff, 0xe0]), None, "JPEG");
    }

    #[test]
    fn animated_pictures_are_found_by_their_chunks() {
        assert_eq!(motion(&png(&[b"IHDR", b"acTL", b"IDAT"])), Some(Motion::Apng));
        assert_eq!(motion(&png(&[b"IHDR", b"IDAT", b"acTL"])), None, "acTL after the pixels does not count");
        assert_eq!(motion(&webp(0x02)), Some(Motion::WebP));
        assert_eq!(motion(&webp(0x10)), None, "alpha only");
        assert_eq!(motion(b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0"), None, "simple WebP");
    }

    #[test]
    fn a_gif_moves_with_its_loop_extension_or_a_second_frame() {
        // Header and logical screen, no colour table.
        let head = |rest: &[u8]| [b"GIF89a".as_slice(), &[1, 0, 1, 0, 0, 0, 0], rest].concat();
        let frame: &[u8] = &[0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 1, 0, 0];
        assert_eq!(motion(&head(&[frame, &[0x3b]].concat())), None, "one frame");
        assert_eq!(motion(&head(&[frame, frame, &[0x3b]].concat())), Some(Motion::Gif));
        let looping: &[u8] = &[0x21, 0xff, 11, b'N', b'E', b'T', b'S', b'C', b'A', b'P', b'E', b'2', b'.', b'0', 3, 1, 0, 0, 0];
        assert_eq!(motion(&head(&[looping, frame, &[0x3b]].concat())), Some(Motion::Gif));
    }
}
