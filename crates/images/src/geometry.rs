//! Pure geometry helpers (CSS `object-fit: cover` math and stage scaling).

/// A crop rectangle in source pixels (fractional).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Centered crop of a `sw x sh` source that has the aspect ratio of `tw x th`
/// (what CSS `object-fit: cover; object-position: 50% 50%` shows).
pub fn cover_crop(sw: u32, sh: u32, tw: u32, th: u32) -> CropRect {
    let (sw, sh, tw, th) = (sw as f64, sh as f64, tw as f64, th as f64);
    // Compare sw/sh with tw/th via cross-multiplication (no division error).
    if sw * th > tw * sh {
        // Source is wider than the target: crop left/right.
        let w = sh * tw / th;
        CropRect { x: (sw - w) / 2.0, y: 0.0, w, h: sh }
    } else {
        let h = sw * th / tw;
        CropRect { x: 0.0, y: (sh - h) / 2.0, w: sw, h }
    }
}

/// Uniform scale applied by `cover` (target px per source px).
pub fn cover_scale(sw: u32, sh: u32, tw: u32, th: u32) -> f64 {
    (tw as f64 / sw as f64).max(th as f64 / sh as f64)
}

/// Size of the (reduced-resolution) stage raster for a full-window target.
///
/// A Gaussian blur of `blur_px` (physical px at target resolution) removes all
/// detail finer than ~blur_px, so we render at scale `s` and blur with
/// `blur_px * s`. `s` is chosen so that the blurred raster keeps
/// `sigma_out` px of sigma (default 8): enough samples per sigma for a smooth
/// bilinear GPU upscale, while minimising pixels. Never above `max_w` wide.
pub fn stage_size(tw: u32, th: u32, blur_px: f32, max_w: u32, sigma_out: f32) -> (u32, u32, f32) {
    let mut s = 1.0f32;
    if tw > max_w {
        s = max_w as f32 / tw as f32;
    }
    if blur_px > 0.0 {
        s = s.min((sigma_out / blur_px).max(0.05));
    }
    // Never go below 160 px wide (nor upscale).
    s = s.max((160.0 / tw as f32).min(1.0)).min(1.0);
    let w = ((tw as f32 * s).round() as u32).max(1);
    let h = ((th as f32 * s).round() as u32).max(1);
    // Effective scale after rounding (use the horizontal one; aspect is kept).
    (w, h, w as f32 / tw as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_source_crops_sides() {
        // 1920x620 hero into a 250x375 tile: keep full height, crop width.
        let c = cover_crop(1920, 620, 250, 375);
        assert_eq!(c.h, 620.0);
        assert!((c.w - 620.0 * 250.0 / 375.0).abs() < 1e-9);
        assert!((c.x - (1920.0 - c.w) / 2.0).abs() < 1e-9);
        assert_eq!(c.y, 0.0);
    }

    #[test]
    fn tall_source_crops_top_bottom() {
        let c = cover_crop(600, 900, 400, 400);
        assert_eq!(c.w, 600.0);
        assert_eq!(c.h, 600.0);
        assert_eq!(c.x, 0.0);
        assert_eq!(c.y, 150.0);
    }

    #[test]
    fn same_aspect_is_identity() {
        let c = cover_crop(600, 900, 200, 300);
        assert_eq!(c, CropRect { x: 0.0, y: 0.0, w: 600.0, h: 900.0 });
    }

    #[test]
    fn scale_is_max_axis() {
        assert!((cover_scale(600, 900, 250, 375) - 250.0 / 600.0).abs() < 1e-12);
        assert!((cover_scale(1920, 620, 250, 375) - 375.0 / 620.0).abs() < 1e-12);
    }

    #[test]
    fn stage_scales_with_blur_and_width() {
        let (w, h, s) = stage_size(1920, 1080, 40.0, 960, 8.0);
        assert_eq!((w, h), (384, 216));
        assert!((s - 0.2).abs() < 1e-3);
        // small blur: limited by max width
        let (w, _, _) = stage_size(3840, 2160, 2.0, 960, 8.0);
        assert_eq!(w, 960);
        // no blur at small target: unchanged
        let (w, h, s) = stage_size(800, 600, 0.0, 960, 8.0);
        assert_eq!((w, h, s), (800, 600, 1.0));
    }
}
