//! PNG side of the compression: a cheap probe of the chunks before the image
//! data (what decides whether a file can become a lossless WebP), the full
//! decode, and the marker of a file that stays PNG.

use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::Path;

use super::webp::MAX_SIDE;

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
/// `tEXt` keyword and text of a PNG we tried and kept (the WebP was not
/// smaller). Placed right after `IHDR`, where [`probe`] reads it.
const KEPT_KEYWORD: &str = "Heronite";
const KEPT_TEXT: &str = "compression=kept-png";
/// sRGB's gamma (1/2.2) in `gAMA` units, and how far a writer may round it.
const SRGB_GAMMA: i64 = 45455;
const GAMMA_SLACK: i64 = 1000;
/// Decoded buffers above this are refused (a 16383x16383 RGBA image).
const MAX_DECODED: usize = 16383 * 16383 * 4;

/// What [`probe`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// Can become a lossless WebP.
    Convertible,
    /// Tried before and kept as PNG (carries our marker).
    Kept,
    /// WebP cannot hold it exactly (reason for the log).
    Unsupported(&'static str),
}

/// Reads the chunk headers up to the image data. Only a few KB of the file.
pub fn probe(path: &Path) -> io::Result<Probe> {
    let mut f = BufReader::with_capacity(8192, File::open(path)?);
    let mut sig = [0u8; 8];
    if f.read_exact(&mut sig).is_err() || sig != SIGNATURE {
        return Ok(Probe::Unsupported("not a PNG file"));
    }
    let (mut srgb, mut icc, mut gamma, mut chrm) = (false, false, None, false);
    let mut header = [0u8; 8];
    loop {
        if f.read_exact(&mut header).is_err() {
            return Ok(Probe::Unsupported("truncated PNG"));
        }
        let len = u32::from_be_bytes(header[..4].try_into().expect("4 bytes"));
        let kind: [u8; 4] = header[4..].try_into().expect("4 bytes");
        let mut body = Vec::new();
        let small = matches!(&kind, b"IHDR" | b"gAMA") || (&kind == b"tEXt" && len < 64);
        if small {
            body.resize(len as usize, 0);
            f.read_exact(&mut body)?;
            f.seek_relative(4)?;
        } else if !matches!(&kind, b"IDAT" | b"IEND") {
            f.seek_relative(i64::from(len) + 4)?;
        }
        match &kind {
            b"IHDR" if body.len() == 13 => {
                let w = u32::from_be_bytes(body[..4].try_into().expect("4 bytes"));
                let h = u32::from_be_bytes(body[4..8].try_into().expect("4 bytes"));
                if w > MAX_SIDE || h > MAX_SIDE {
                    return Ok(Probe::Unsupported("larger than WebP's 16383 px limit"));
                }
                if body[8] > 8 {
                    return Ok(Probe::Unsupported("16-bit colour"));
                }
            }
            b"acTL" => return Ok(Probe::Unsupported("animated PNG")),
            b"cICP" | b"mDCV" | b"cLLI" => return Ok(Probe::Unsupported("HDR colour")),
            b"sRGB" => srgb = true,
            b"iCCP" => icc = true,
            b"cHRM" => chrm = true,
            b"gAMA" if body.len() == 4 => gamma = Some(i64::from(u32::from_be_bytes(body[..4].try_into().expect("4 bytes")))),
            b"tEXt" if small && body.starts_with(KEPT_KEYWORD.as_bytes()) && body.get(KEPT_KEYWORD.len()) == Some(&0) => {
                return Ok(Probe::Kept);
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
    }
    // WebP has no gamma or primaries: without an sRGB chunk or an ICC
    // profile (which the WebP keeps), anything other than sRGB would look
    // different after the conversion.
    if !srgb && !icc {
        if gamma.is_some_and(|g| (g - SRGB_GAMMA).abs() > GAMMA_SLACK) {
            return Ok(Probe::Unsupported("non-sRGB gamma"));
        }
        if chrm {
            return Ok(Probe::Unsupported("custom colour primaries"));
        }
    }
    Ok(Probe::Convertible)
}

/// 8-bit pixels of a PNG plus the metadata the WebP keeps.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// RGB rows, or RGBA when some pixel is not fully opaque.
    pub pixels: Vec<u8>,
    pub alpha: bool,
    pub icc: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
    /// `tEXt`, `zTXt` and `iTXt` chunks as (keyword, text).
    pub text: Vec<(String, String)>,
}

/// Decodes a PNG that [`probe`] called convertible. Palette, low bit depths
/// and `tRNS` are expanded; grey becomes RGB (same values in each channel);
/// an alpha channel that is opaque everywhere is dropped (it holds nothing).
pub fn decode(path: &Path) -> Result<Decoded, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new_with_limits(BufReader::new(file), png::Limits { bytes: MAX_DECODED });
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader.output_buffer_size().ok_or("image too large")?;
    let mut buf = vec![0u8; size];
    let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(frame.buffer_size());
    if frame.bit_depth != png::BitDepth::Eight {
        return Err("16-bit colour".into());
    }
    let (pixels, alpha) = match frame.color_type {
        png::ColorType::Rgb => (buf, false),
        png::ColorType::Rgba => to_rgb_if_opaque(buf),
        png::ColorType::Grayscale => (buf.iter().flat_map(|&g| [g, g, g]).collect(), false),
        png::ColorType::GrayscaleAlpha => {
            let rgba = buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect();
            to_rgb_if_opaque(rgba)
        }
        png::ColorType::Indexed => return Err("palette was not expanded".into()),
    };
    // Text after the image data is only known once the file is read.
    reader.finish().map_err(|e| e.to_string())?;
    let info = reader.info();
    let mut text: Vec<(String, String)> = info.uncompressed_latin1_text.iter().map(|t| (t.keyword.clone(), t.text.clone())).collect();
    text.extend(info.compressed_latin1_text.iter().filter_map(|t| Some((t.keyword.clone(), t.get_text().ok()?))));
    text.extend(info.utf8_text.iter().filter_map(|t| Some((t.keyword.clone(), t.get_text().ok()?))));
    Ok(Decoded {
        width: frame.width,
        height: frame.height,
        pixels,
        alpha,
        icc: info.icc_profile.as_ref().map(|c| c.to_vec()),
        exif: info.exif_metadata.as_ref().map(|c| c.to_vec()),
        text,
    })
}

fn to_rgb_if_opaque(rgba: Vec<u8>) -> (Vec<u8>, bool) {
    let pixels = rgba.as_chunks::<4>().0;
    if pixels.iter().all(|p| p[3] == 255) {
        (pixels.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(), false)
    } else {
        (rgba, true)
    }
}

/// Adds our "kept" `tEXt` chunk after `IHDR` (the pixels are untouched) and
/// restores the file's times, so the next scan skips it without decoding.
pub fn mark_kept(path: &Path) -> io::Result<()> {
    let meta = fs::metadata(path)?;
    let bytes = fs::read(path)?;
    // Signature (8) + IHDR length, type, 13 bytes of data and CRC.
    const IHDR_END: usize = 8 + 8 + 13 + 4;
    if bytes.len() < IHDR_END || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a PNG file"));
    }
    let mut data = Vec::with_capacity(KEPT_KEYWORD.len() + 1 + KEPT_TEXT.len());
    data.extend_from_slice(KEPT_KEYWORD.as_bytes());
    data.push(0);
    data.extend_from_slice(KEPT_TEXT.as_bytes());
    let mut crc = crc32fast::Hasher::new();
    crc.update(b"tEXt");
    crc.update(&data);

    let mut out = Vec::with_capacity(bytes.len() + data.len() + 12);
    out.extend_from_slice(&bytes[..IHDR_END]);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(b"tEXt");
    out.extend_from_slice(&data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
    out.extend_from_slice(&bytes[IHDR_END..]);
    heronite_data::temps::write_atomic(path, &out, Some(super::unix_mode(&meta)))?;
    super::restore_times(path, &meta)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A noisy gradient: like a capture, it gets smaller as a lossless WebP.
    pub(crate) fn noisy_rgb(w: u32, h: u32) -> Vec<u8> {
        let mut seed = 7u32;
        (0..w * h)
            .flat_map(|i| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let (x, y) = (i % w, i / w);
                [(x * 2) as u8, (y * 3) as u8, ((x + y) as u8) ^ ((seed >> 28) as u8)]
            })
            .collect()
    }

    /// Writes an 8-bit PNG with optional extra chunks before the image data.
    pub(crate) fn write_png(path: &Path, w: u32, h: u32, color: png::ColorType, data: &[u8], extra: &[(&[u8; 4], &[u8])]) {
        let file = File::create(path).unwrap();
        let mut enc = png::Encoder::new(io::BufWriter::new(file), w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        for (kind, body) in extra {
            writer.write_chunk(png::chunk::ChunkType(**kind), body).unwrap();
        }
        writer.write_image_data(data).unwrap();
    }

    #[test]
    fn probe_rules() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        let rgb = vec![10u8; 4 * 4 * 3];
        write_png(&p, 4, 4, png::ColorType::Rgb, &rgb, &[]);
        assert_eq!(probe(&p).unwrap(), Probe::Convertible);
        write_png(&p, 4, 4, png::ColorType::Rgb, &rgb, &[(b"sRGB", &[0]), (b"gAMA", &45455u32.to_be_bytes())]);
        assert_eq!(probe(&p).unwrap(), Probe::Convertible);
        write_png(&p, 4, 4, png::ColorType::Rgb, &rgb, &[(b"gAMA", &100_000u32.to_be_bytes())]);
        assert_eq!(probe(&p).unwrap(), Probe::Unsupported("non-sRGB gamma"));
        write_png(&p, 4, 4, png::ColorType::Rgb, &rgb, &[(b"cICP", &[9, 16, 0, 1])]);
        assert_eq!(probe(&p).unwrap(), Probe::Unsupported("HDR colour"));
        fs::write(&p, b"not a png").unwrap();
        assert_eq!(probe(&p).unwrap(), Probe::Unsupported("not a PNG file"));
    }

    #[test]
    fn decode_drops_an_opaque_alpha_and_keeps_text() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        let rgba: Vec<u8> = (0..4 * 2).flat_map(|i| [i as u8, 2, 3, 255]).collect();
        write_png(&p, 4, 2, png::ColorType::Rgba, &rgba, &[(b"tEXt", b"Software\0grim")]);
        let d = decode(&p).unwrap();
        assert!(!d.alpha);
        assert_eq!(d.pixels.len(), 4 * 2 * 3);
        assert_eq!(&d.pixels[3..6], &[1, 2, 3]);
        assert_eq!(d.text, [("Software".to_owned(), "grim".to_owned())]);

        let translucent: Vec<u8> = (0..4 * 2).flat_map(|i| [i as u8, 2, 3, 128]).collect();
        write_png(&p, 4, 2, png::ColorType::Rgba, &translucent, &[]);
        let d = decode(&p).unwrap();
        assert!(d.alpha);
        assert_eq!(d.pixels, translucent);
    }

    #[test]
    fn kept_marker_is_found_by_the_probe_and_keeps_pixels_and_times() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        let rgb: Vec<u8> = (0..16 * 3).map(|i| i as u8).collect();
        write_png(&p, 4, 4, png::ColorType::Rgb, &rgb, &[]);
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
        mark_kept(&p).unwrap();
        assert_eq!(probe(&p).unwrap(), Probe::Kept);
        assert_eq!(decode(&p).unwrap().pixels, rgb);
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), old);
    }
}
