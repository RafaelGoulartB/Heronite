//! Decoding bytes into a packed RGB / RGBA raster, using DCT-domain downscaling
//! for JPEG so memory and time follow the *output* size rather than the source.

use crate::error::{ImageError, Result};
use crate::geometry::cover_scale;
use std::io::Cursor;

/// Hard cap on decoded source pixels (guards decompression bombs).
pub const MAX_SOURCE_PIXELS: u64 = 120_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Channels {
    Rgb,
    /// Straight (non-premultiplied) alpha.
    Rgba,
}

pub(crate) struct Raster {
    pub w: u32,
    pub h: u32,
    pub ch: Channels,
    /// For `Rgba`: true when every alpha is 255 (resize can skip alpha math).
    pub opaque: bool,
    pub data: Vec<u8>,
}

/// Returns true if `bytes` starts with a raster format we can decode.
pub(crate) fn sniff(bytes: &[u8]) -> bool {
    image::guess_format(bytes).is_ok()
}

/// Decode `bytes`. `(tw, th)` is the final cover target (used only to pick a
/// JPEG DCT scale that is never lower-resolution than the target needs).
pub(crate) fn decode(bytes: &[u8], tw: u32, th: u32, jpeg_scaled: bool) -> Result<Raster> {
    let fmt = image::guess_format(bytes).map_err(|e| ImageError::NotAnImage(e.to_string()))?;
    if fmt == image::ImageFormat::Jpeg {
        match decode_jpeg_scaled(bytes, tw, th, jpeg_scaled) {
            Ok(Some(r)) => return Ok(r),
            Ok(None) => {} // unsupported pixel format -> generic path
            Err(e) => {
                log::debug!("jpeg-decoder failed ({e}); falling back to image crate");
            }
        }
    }
    decode_generic(bytes, fmt)
}

fn decode_jpeg_scaled(bytes: &[u8], tw: u32, th: u32, scaled: bool) -> std::result::Result<Option<Raster>, jpeg_decoder::Error> {
    let mut dec = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    dec.read_info()?;
    let info = dec.info().ok_or_else(|| jpeg_decoder::Error::Format("no info".into()))?;
    let (sw, sh) = (info.width as u32, info.height as u32);
    if sw as u64 * sh as u64 > MAX_SOURCE_PIXELS {
        return Err(jpeg_decoder::Error::Format("image too large".into()));
    }
    // Same as `decode_generic`: the EXIF orientation decides what is upright.
    let orientation =
        dec.exif_data().and_then(image::metadata::Orientation::from_exif_chunk).unwrap_or(image::metadata::Orientation::NoTransforms);
    if scaled {
        // The target is for the upright picture, so size the scale by its axes.
        let (uw, uh) = if swaps_axes(orientation) { (sh, sw) } else { (sw, sh) };
        let cs = cover_scale(uw, uh, tw.max(1), th.max(1));
        if cs < 1.0 {
            // Request exactly the cover-scaled size: the decoder then picks the
            // smallest DCT scale (1/8, 1/4, 1/2, 1) whose output is >= that.
            let rw = ((sw as f64 * cs).ceil() as u32).clamp(1, u16::MAX as u32) as u16;
            let rh = ((sh as f64 * cs).ceil() as u32).clamp(1, u16::MAX as u32) as u16;
            dec.scale(rw, rh)?;
        }
    }
    let pixels = dec.decode()?;
    let info = dec.info().unwrap();
    let (w, h) = (info.width as u32, info.height as u32);
    use jpeg_decoder::PixelFormat::*;
    let data = match info.pixel_format {
        RGB24 => pixels,
        L8 => {
            let mut data = Vec::with_capacity(pixels.len() * 3);
            for &v in &pixels {
                data.extend_from_slice(&[v, v, v]);
            }
            data
        }
        _ => return Ok(None),
    };
    Ok(Some(orient_rgb(w, h, data, orientation)))
}

/// True when applying `o` swaps width and height (the 90 degree rotations).
fn swaps_axes(o: image::metadata::Orientation) -> bool {
    use image::metadata::Orientation::*;
    matches!(o, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH)
}

/// An opaque RGB raster turned to be upright for the EXIF orientation `o`.
fn orient_rgb(w: u32, h: u32, data: Vec<u8>, o: image::metadata::Orientation) -> Raster {
    if o == image::metadata::Orientation::NoTransforms {
        return Raster { w, h, ch: Channels::Rgb, opaque: true, data };
    }
    let buf = image::RgbImage::from_raw(w, h, data).expect("RGB buffer matches its size");
    let mut img = image::DynamicImage::ImageRgb8(buf);
    img.apply_orientation(o);
    let rgb = img.into_rgb8();
    let (w, h) = rgb.dimensions();
    Raster { w, h, ch: Channels::Rgb, opaque: true, data: rgb.into_raw() }
}

fn decode_generic(bytes: &[u8], fmt: image::ImageFormat) -> Result<Raster> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), fmt);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_SOURCE_PIXELS * 4);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| ImageError::Decode(e.to_string()))?;
    let (w, h) = decoder.dimensions();
    if w as u64 * h as u64 > MAX_SOURCE_PIXELS {
        return Err(ImageError::Decode(format!("image too large ({w}x{h})")));
    }
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| ImageError::Decode(e.to_string()))?;
    img.apply_orientation(orientation);
    if img.color().has_alpha() {
        let rgba = img.into_rgba8();
        let (w, h) = rgba.dimensions();
        let data = rgba.into_raw();
        let opaque = data.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
        Ok(Raster { w, h, ch: Channels::Rgba, opaque, data })
    } else {
        let rgb = img.into_rgb8();
        let (w, h) = rgb.dimensions();
        Ok(Raster { w, h, ch: Channels::Rgb, opaque: true, data: rgb.into_raw() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::metadata::Orientation;

    /// A JPEG of `w` x `h` whose left half is red and right half blue, with an
    /// EXIF APP1 segment that says `orientation`.
    fn jpeg_with_orientation(w: u32, h: u32, orientation: u16) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, _| if x < w / 2 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(&img).unwrap();
        // TIFF header (little endian) + one IFD entry: tag 0x0112, SHORT, count 1.
        let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend_from_slice(&orientation.to_le_bytes());
        exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mut out = jpeg[..2].to_vec(); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&(exif.len() as u16 + 2).to_be_bytes());
        out.extend_from_slice(&exif);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    fn px(r: &Raster, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * r.w + x) * 3) as usize;
        [r.data[i], r.data[i + 1], r.data[i + 2]]
    }

    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) < 40)
    }

    #[test]
    fn both_jpeg_paths_apply_the_exif_orientation() {
        // Orientation 6: the picture must be turned 90 degrees clockwise, so
        // the left (red) half ends up on top and the 40x20 picture is 20x40.
        let bytes = jpeg_with_orientation(40, 20, 6);
        let fast = decode(&bytes, 20, 40, true).unwrap();
        let generic = decode_generic(&bytes, image::ImageFormat::Jpeg).unwrap();
        for r in [&fast, &generic] {
            assert_eq!((r.w, r.h), (20, 40));
            assert!(near(px(r, 10, 5), [255, 0, 0]), "{:?}", px(r, 10, 5));
            assert!(near(px(r, 10, 35), [0, 0, 255]), "{:?}", px(r, 10, 35));
        }
    }

    #[test]
    fn scaled_decode_sizes_the_scale_by_the_upright_picture() {
        // 400x200 stored sideways is a 200x400 picture: a 100x200 target is a
        // 1/2 scale of it, not a 1/4 one chosen against the stored axes.
        let bytes = jpeg_with_orientation(400, 200, 6);
        let r = decode(&bytes, 100, 200, true).unwrap();
        assert!(r.w >= 100 && r.h >= 200, "{}x{}", r.w, r.h);
    }

    #[test]
    fn unrotated_jpeg_is_left_alone() {
        let bytes = jpeg_with_orientation(40, 20, 1);
        let r = decode(&bytes, 40, 20, true).unwrap();
        assert_eq!((r.w, r.h), (40, 20));
        assert!(near(px(&r, 5, 10), [255, 0, 0]));
        assert!(near(px(&r, 35, 10), [0, 0, 255]));
        assert!(!swaps_axes(Orientation::Rotate180) && swaps_axes(Orientation::Rotate270));
    }
}
