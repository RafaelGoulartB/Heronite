//! WebP side of the compression: the lossless encode (libwebp), the extended
//! container that carries the colour profile, EXIF and our XMP marker, and
//! the cheap read of that marker.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::raw::c_int;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use libwebp_sys::*;

use super::marker::{self, Marker};

/// `cwebp -z` level. On game captures 5 frees as much as 9 (within 1%) in a
/// tenth of the time (~0.8 s for 2560x1080 on one core instead of ~8 s).
const LOSSLESS_LEVEL: c_int = 5;
/// WebP's limit on each side.
pub const MAX_SIDE: u32 = 16383;
/// Larger XMP chunks are not ours (we only read the marker).
const MAX_XMP: u32 = 1 << 20;

#[derive(Debug)]
pub enum EncodeError {
    /// The cancel flag was raised while encoding.
    Cancelled,
    Failed(String),
}

/// `libwebp 1.6.0`.
pub fn encoder_name() -> String {
    // SAFETY: a plain getter without arguments.
    let v = unsafe { WebPGetEncoderVersion() };
    format!("libwebp {}.{}.{} lossless", (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff)
}

/// Stops the encode when the flag in `user_data` is raised.
unsafe extern "C" fn progress(_percent: c_int, picture: *const WebPPicture) -> c_int {
    // SAFETY: `user_data` points at the `AtomicBool` borrowed by `encode`
    // for the whole `WebPEncode` call that invokes this hook.
    let cancel = unsafe { &*((*picture).user_data as *const AtomicBool) };
    c_int::from(!cancel.load(Ordering::Relaxed))
}

/// Lossless WebP (a simple `RIFF/WEBP/VP8L` file) of 8-bit RGB or RGBA rows.
/// `exact` keeps the colour under fully transparent pixels, so every pixel
/// decodes back to the same bytes. Single-threaded on purpose: it runs on a
/// background thread and must stay light.
pub fn encode(width: u32, height: u32, pixels: &[u8], alpha: bool, cancel: &AtomicBool) -> Result<Vec<u8>, EncodeError> {
    let channels = if alpha { 4 } else { 3 };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(EncodeError::Failed(format!("{width}x{height} is outside WebP's limits")));
    }
    if pixels.len() != width as usize * height as usize * channels {
        return Err(EncodeError::Failed("pixel buffer of the wrong size".into()));
    }
    let mut config = WebPConfig::new().map_err(|_| EncodeError::Failed("WebPConfigInit failed".into()))?;
    let mut picture = WebPPicture::new().map_err(|_| EncodeError::Failed("WebPPictureInit failed".into()))?;
    // SAFETY: `config` and `picture` are initialised above; `picture` only
    // borrows `pixels` (copied by the import), `writer` and `cancel` during
    // this block, and is freed before they go away.
    unsafe {
        if WebPConfigLosslessPreset(&mut config, LOSSLESS_LEVEL) == 0 {
            return Err(EncodeError::Failed("WebPConfigLosslessPreset failed".into()));
        }
        config.exact = 1;
        config.thread_level = 0;
        if WebPValidateConfig(&config) == 0 {
            return Err(EncodeError::Failed("invalid WebP configuration".into()));
        }
        picture.use_argb = 1;
        picture.width = width as c_int;
        picture.height = height as c_int;
        let stride = (width as usize * channels) as c_int;
        let imported = if alpha {
            WebPPictureImportRGBA(&mut picture, pixels.as_ptr(), stride)
        } else {
            WebPPictureImportRGB(&mut picture, pixels.as_ptr(), stride)
        };
        if imported == 0 {
            WebPPictureFree(&mut picture);
            return Err(EncodeError::Failed("out of memory importing the pixels".into()));
        }
        let mut writer: WebPMemoryWriter = std::mem::zeroed();
        WebPMemoryWriterInit(&mut writer);
        picture.writer = Some(WebPMemoryWrite);
        picture.custom_ptr = (&mut writer as *mut WebPMemoryWriter).cast();
        picture.progress_hook = Some(progress);
        picture.user_data = (cancel as *const AtomicBool).cast_mut().cast();
        let ok = WebPEncode(&config, &mut picture) != 0;
        let error = picture.error_code;
        WebPPictureFree(&mut picture);
        let out = if ok && !writer.mem.is_null() {
            Ok(std::slice::from_raw_parts(writer.mem, writer.size).to_vec())
        } else if error == WebPEncodingError::VP8_ENC_ERROR_USER_ABORT {
            Err(EncodeError::Cancelled)
        } else {
            Err(EncodeError::Failed(format!("WebP encoding failed ({error:?})")))
        };
        WebPMemoryWriterClear(&mut writer);
        out
    }
}

fn chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// The `VP8L` payload of a simple lossless file from [`encode`].
fn vp8l_payload(simple: &[u8]) -> Option<&[u8]> {
    if simple.len() < 20 || &simple[..4] != b"RIFF" || &simple[8..12] != b"WEBP" || &simple[12..16] != b"VP8L" {
        return None;
    }
    let len = u32::from_le_bytes(simple[16..20].try_into().ok()?) as usize;
    simple.get(20..20 + len)
}

/// Extras of the extended container.
pub struct Extras<'a> {
    pub icc: Option<&'a [u8]>,
    pub exif: Option<&'a [u8]>,
    pub xmp: &'a str,
}

/// Rewraps a simple lossless file as an extended one (`VP8X`, `ICCP`,
/// `VP8L`, `EXIF`, `XMP `: the order the container spec requires).
pub fn with_extras(simple: &[u8], width: u32, height: u32, alpha: bool, extras: &Extras) -> Option<Vec<u8>> {
    let payload = vp8l_payload(simple)?;
    let mut flags = 0x04u8; // XMP
    if extras.icc.is_some() {
        flags |= 0x20;
    }
    if alpha {
        flags |= 0x10;
    }
    if extras.exif.is_some() {
        flags |= 0x08;
    }
    let mut vp8x = [0u8; 10];
    vp8x[0] = flags;
    vp8x[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
    vp8x[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);

    let mut body = Vec::with_capacity(payload.len() + extras.xmp.len() + 64);
    body.extend_from_slice(b"WEBP");
    chunk(&mut body, b"VP8X", &vp8x);
    if let Some(icc) = extras.icc {
        chunk(&mut body, b"ICCP", icc);
    }
    chunk(&mut body, b"VP8L", payload);
    if let Some(exif) = extras.exif {
        chunk(&mut body, b"EXIF", exif);
    }
    chunk(&mut body, b"XMP ", extras.xmp.as_bytes());

    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

/// Our marker in a WebP file: walks the chunk headers (seeking over the
/// image data) to the `XMP ` chunk. `Ok(None)` for any other WebP or file.
pub fn read_marker(path: &Path) -> io::Result<Option<Marker>> {
    let mut f = File::open(path)?;
    let mut head = [0u8; 12];
    if f.read_exact(&mut head).is_err() || &head[..4] != b"RIFF" || &head[8..] != b"WEBP" {
        return Ok(None);
    }
    let mut header = [0u8; 8];
    while f.read_exact(&mut header).is_ok() {
        let size = u32::from_le_bytes(header[4..].try_into().expect("4 bytes"));
        let padded = i64::from(size) + i64::from(size % 2);
        match &header[..4] {
            // Only an extended file has metadata chunks.
            b"VP8 " | b"VP8L" if f.stream_position()? == 20 => return Ok(None),
            b"XMP " if size <= MAX_XMP => {
                let mut xmp = vec![0u8; size as usize];
                f.read_exact(&mut xmp)?;
                return Ok(marker::parse_xmp(&String::from_utf8_lossy(&xmp)));
            }
            _ => {
                f.seek(SeekFrom::Current(padded))?;
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compress::marker::LOSSLESS_WEBP;

    fn gradient(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                out.extend_from_slice(&[(x * 7) as u8, (y * 3) as u8, (x ^ y) as u8]);
                if alpha {
                    // Fully transparent pixels keep their colour (`exact`).
                    out.push(if x % 5 == 0 { 0 } else { (x + y) as u8 });
                }
            }
        }
        out
    }

    #[test]
    fn lossless_round_trip_with_marker() {
        for alpha in [false, true] {
            let (w, h) = (97, 41);
            let px = gradient(w, h, alpha);
            let simple = encode(w, h, &px, alpha, &AtomicBool::new(false)).unwrap();
            let m = Marker { compression: LOSSLESS_WEBP.into(), source_name: "a.png".into(), source_size: 1234 };
            let xmp = marker::to_xmp(&m, &encoder_name(), &[]);
            let icc = [1u8, 2, 3];
            let file = with_extras(&simple, w, h, alpha, &Extras { icc: Some(&icc), exif: Some(b"Exif"), xmp: &xmp }).unwrap();

            let img = image::load_from_memory_with_format(&file, image::ImageFormat::WebP).unwrap();
            let back = if alpha { img.to_rgba8().into_raw() } else { img.to_rgb8().into_raw() };
            assert!(back == px, "alpha={alpha}");

            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("a.webp");
            std::fs::write(&p, &file).unwrap();
            assert_eq!(read_marker(&p).unwrap(), Some(m));
            std::fs::write(&p, &simple).unwrap();
            assert_eq!(read_marker(&p).unwrap(), None);
        }
    }

    #[test]
    fn a_raised_flag_cancels() {
        let px = gradient(64, 64, false);
        assert!(matches!(encode(64, 64, &px, false, &AtomicBool::new(true)), Err(EncodeError::Cancelled)));
    }
}
