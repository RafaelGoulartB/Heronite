use crate::decode::{Channels, Raster};
use crate::error::{ImageError, Result};
use crate::geometry::{cover_crop, fit_size};
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

/// Resampling filter used for the cover resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Sharpest; best for downscaling photos/covers (default).
    Lanczos3,
    /// Slightly softer than Lanczos, less ringing.
    CatmullRom,
    /// Fastest of the convolution filters, softest.
    Bilinear,
}

impl Filter {
    fn alg(self) -> ResizeAlg {
        ResizeAlg::Convolution(match self {
            Filter::Lanczos3 => FilterType::Lanczos3,
            Filter::CatmullRom => FilterType::CatmullRom,
            Filter::Bilinear => FilterType::Bilinear,
        })
    }
    pub(crate) fn id(self) -> u8 {
        self as u8
    }
}

/// Resize with object-fit *cover* semantics (centered crop) to exactly
/// `tw x th`. Output is tightly packed straight-alpha RGBA with exact capacity.
pub(crate) fn cover_to_rgba(src: Raster, tw: u32, th: u32, filter: Filter) -> Result<Vec<u8>> {
    if src.w == 0 || src.h == 0 || tw == 0 || th == 0 {
        return Err(ImageError::Decode("empty image".into()));
    }
    let crop = cover_crop(src.w, src.h, tw, th);
    let (w0, h0, ch, opaque) = (src.w, src.h, src.ch, src.opaque);
    let pt = match ch {
        Channels::Rgb => PixelType::U8x3,
        Channels::Rgba => PixelType::U8x4,
    };
    let src_img = Image::from_vec_u8(w0, h0, src.data, pt).map_err(|e| ImageError::Decode(e.to_string()))?;
    let mut dst = Image::new(tw, th, pt);
    let opts = ResizeOptions::new().resize_alg(filter.alg()).crop(crop.x, crop.y, crop.w, crop.h).use_alpha(!opaque);
    Resizer::new().resize(&src_img, &mut dst, &opts).map_err(|e| ImageError::Decode(e.to_string()))?;
    drop(src_img); // free the (possibly large) source before expanding
    let small = dst.into_vec();
    Ok(match ch {
        Channels::Rgba => {
            let mut v = small;
            v.shrink_to_fit();
            v
        }
        Channels::Rgb => {
            let mut out = Vec::with_capacity(tw as usize * th as usize * 4);
            for p in small.as_chunks::<3>().0 {
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            out
        }
    })
}

/// Resize with object-fit *contain* semantics: the whole image, scaled down
/// (never up) to fit inside `tw x th`, keeping its aspect ratio and its
/// transparency. Returns the fitted size and tightly packed straight-alpha
/// RGBA with exact capacity.
pub(crate) fn fit_to_rgba(src: Raster, tw: u32, th: u32, filter: Filter) -> Result<(u32, u32, Vec<u8>)> {
    if src.w == 0 || src.h == 0 || tw == 0 || th == 0 {
        return Err(ImageError::Decode("empty image".into()));
    }
    let (ow, oh) = fit_size(src.w, src.h, tw, th);
    // Straight alpha: the resizer premultiplies while it filters, so the
    // transparent edge of a logo does not bleed a dark or white halo.
    let rgba = match src.ch {
        Channels::Rgba => src,
        Channels::Rgb => {
            let mut data = Vec::with_capacity(src.w as usize * src.h as usize * 4);
            for p in src.data.as_chunks::<3>().0 {
                data.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            Raster { ch: Channels::Rgba, opaque: true, data, ..src }
        }
    };
    let (w0, h0, opaque) = (rgba.w, rgba.h, rgba.opaque);
    let src_img = Image::from_vec_u8(w0, h0, rgba.data, PixelType::U8x4).map_err(|e| ImageError::Decode(e.to_string()))?;
    let mut dst = Image::new(ow, oh, PixelType::U8x4);
    let opts = ResizeOptions::new().resize_alg(filter.alg()).use_alpha(!opaque);
    Resizer::new().resize(&src_img, &mut dst, &opts).map_err(|e| ImageError::Decode(e.to_string()))?;
    drop(src_img);
    let mut px = dst.into_vec();
    px.shrink_to_fit();
    Ok((ow, oh, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster_rgb(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Raster {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&f(x, y));
            }
        }
        Raster { w, h, ch: Channels::Rgb, opaque: true, data }
    }

    #[test]
    fn cover_crops_center_wide_source() {
        // 30x10 source: red edges, green middle (central 10x10 is inside the green band).
        let src = raster_rgb(30, 10, |x, _| {
            if x < 7 {
                [255, 0, 0]
            } else if x < 23 {
                [0, 255, 0]
            } else {
                [0, 0, 255]
            }
        });
        // Square target => crops the central 10x10 => pure green.
        let out = cover_to_rgba(src, 8, 8, Filter::Bilinear).unwrap();
        assert_eq!(out.len(), 8 * 8 * 4);
        for p in out.chunks(4) {
            assert!(p[0] < 8 && p[2] < 8 && p[1] > 247 && p[3] == 255, "{p:?}");
        }
    }

    #[test]
    fn cover_crops_center_tall_source() {
        // 10x30: top red, mid green, bottom blue
        let src = raster_rgb(10, 30, |_, y| {
            if y < 10 {
                [255, 0, 0]
            } else if y < 20 {
                [0, 255, 0]
            } else {
                [0, 0, 255]
            }
        });
        let out = cover_to_rgba(src, 5, 5, Filter::Lanczos3).unwrap();
        for p in out.chunks(4) {
            assert!(p[1] > 240 && p[0] < 16 && p[2] < 16, "{p:?}");
        }
    }

    #[test]
    fn alpha_is_resized_straight() {
        // Half transparent red everywhere stays (255,0,0,128) after resize.
        let data: Vec<u8> = (0..16 * 16).flat_map(|_| [255u8, 0, 0, 128]).collect();
        let src = Raster { w: 16, h: 16, ch: Channels::Rgba, opaque: false, data };
        let out = cover_to_rgba(src, 4, 4, Filter::Lanczos3).unwrap();
        for p in out.chunks(4) {
            assert!(p[0] >= 253 && p[1] <= 2 && (p[3] as i32 - 128).abs() <= 1, "{p:?}");
        }
    }

    #[test]
    fn fit_keeps_whole_image_and_transparency() {
        // 40x20: opaque red left half, fully transparent right half.
        let mut data = Vec::new();
        for _ in 0..20 {
            for x in 0..40 {
                data.extend_from_slice(if x < 20 { &[255, 0, 0, 255] } else { &[0, 0, 0, 0] });
            }
        }
        let src = Raster { w: 40, h: 20, ch: Channels::Rgba, opaque: false, data };
        let (w, h, out) = fit_to_rgba(src, 20, 20, Filter::Bilinear).unwrap();
        assert_eq!((w, h, out.len(), out.capacity()), (20, 10, 20 * 10 * 4, 20 * 10 * 4));
        let px = |x: usize, y: usize| &out[(y * 20 + x) * 4..][..4];
        assert_eq!(px(2, 5), [255, 0, 0, 255]);
        assert_eq!(px(17, 5)[3], 0);
    }

    #[test]
    fn fit_does_not_upscale_and_opaque_sources_get_full_alpha() {
        let src = raster_rgb(8, 4, |_, _| [10, 20, 30]);
        let (w, h, out) = fit_to_rgba(src, 100, 100, Filter::Lanczos3).unwrap();
        assert_eq!((w, h), (8, 4));
        assert!(out.chunks(4).all(|p| p == [10, 20, 30, 255]));
    }
}
