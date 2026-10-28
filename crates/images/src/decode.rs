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
    if scaled {
        let cs = cover_scale(sw, sh, tw.max(1), th.max(1));
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
    match info.pixel_format {
        RGB24 => Ok(Some(Raster { w, h, ch: Channels::Rgb, opaque: true, data: pixels })),
        L8 => {
            let mut data = Vec::with_capacity(pixels.len() * 3);
            for &v in &pixels {
                data.extend_from_slice(&[v, v, v]);
            }
            Ok(Some(Raster { w, h, ch: Channels::Rgb, opaque: true, data }))
        }
        _ => Ok(None),
    }
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
