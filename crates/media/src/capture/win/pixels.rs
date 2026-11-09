//! What becomes of a captured frame before it is written: Desktop
//! Duplication's BGRA or scRGB (HDR) rows to 8-bit sRGB, the rotation of a
//! portrait monitor, and the PNG. No Windows calls here, so it is all tested.
//!
//! # HDR
//! On a monitor with HDR on, the desktop is composed in scRGB: linear light,
//! sRGB primaries, half floats, 1.0 = 80 nits. The frame is scaled so the
//! monitor's SDR white level (Settings → Display → HDR → "SDR content
//! brightness") is white, which keeps the desktop and SDR games as they look.
//! What is brighter than that (HDR highlights) is compressed into the range
//! above a knee with an extended Reinhard curve on the largest channel, so
//! hues stay and the brightest part of the frame (its 99.9th percentile)
//! lands on white instead of clipping a whole area. Colours outside sRGB
//! (negative channels) are clamped. The PNG is 8-bit sRGB like every other
//! capture, so it is listed, previewed and compressed the same way.

use std::sync::OnceLock;

/// The pixel formats a frame comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `DXGI_FORMAT_B8G8R8A8_UNORM` (SDR desktops, GDI).
    Bgra8,
    /// `DXGI_FORMAT_R16G16B16A16_FLOAT`, scRGB (HDR desktops).
    Rgba16Float,
}

/// Rows of a frame as the GPU (or GDI) handed them over.
pub struct Frame<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// Bytes from one row to the next (at least `width` pixels).
    pub pitch: usize,
    pub format: Format,
}

/// An 8-bit sRGB image, 3 bytes a pixel, rows packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgb {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Format {
    fn bytes_per_pixel(self) -> usize {
        match self {
            Format::Bgra8 => 4,
            Format::Rgba16Float => 8,
        }
    }
}

/// The frame as 8-bit sRGB. `sdr_white` is the SDR white level in scRGB
/// units (nits / 80), only used for [`Format::Rgba16Float`].
pub fn to_rgb(frame: &Frame, sdr_white: f32) -> Result<Rgb, String> {
    let bpp = frame.format.bytes_per_pixel();
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w == 0 || h == 0 {
        return Err("the captured frame is empty".into());
    }
    if frame.pitch < w * bpp || frame.data.len() < frame.pitch * (h - 1) + w * bpp {
        return Err("the captured frame is smaller than its size".into());
    }
    let rows = (0..h).map(|y| &frame.data[y * frame.pitch..y * frame.pitch + w * bpp]);
    let mut pixels = Vec::with_capacity(w * h * 3);
    match frame.format {
        Format::Bgra8 => {
            for row in rows {
                for p in row.as_chunks::<4>().0 {
                    pixels.extend_from_slice(&[p[2], p[1], p[0]]);
                }
            }
        }
        Format::Rgba16Float => {
            let white = if sdr_white.is_finite() && sdr_white > 0. { sdr_white } else { 1. };
            let peak = highlight_peak(frame, white);
            let lut = srgb_lut();
            for row in rows {
                for p in row.as_chunks::<8>().0 {
                    let rgb = [half(p, 0) / white, half(p, 1) / white, half(p, 2) / white];
                    pixels.extend_from_slice(&tone_map_pixel(rgb, peak, lut));
                }
            }
        }
    }
    Ok(Rgb { width: frame.width, height: frame.height, pixels })
}

/// The `i`-th half float of a pixel.
fn half(p: &[u8], i: usize) -> f32 {
    half_to_f32(u16::from_le_bytes([p[2 * i], p[2 * i + 1]]))
}

/// IEEE 754 binary16 to binary32.
pub fn half_to_f32(h: u16) -> f32 {
    let negative = h & 0x8000 != 0;
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1f);
    let mant = u32::from(h & 0x3ff);
    let bits = match (exp, mant) {
        (0, 0) => sign,
        // Subnormal: mant * 2^-24.
        (0, m) => {
            let v = m as f32 * f32::powi(2., -24);
            return if negative { -v } else { v };
        }
        (0x1f, m) => sign | 0x7f80_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

/// Linear light below this (in SDR-white units) is kept as it is when the
/// frame has highlights to compress.
const KNEE: f32 = 0.6;

/// How bright the brightest part of the frame is (the 99.9th percentile of
/// the largest channel, in SDR-white units): what the tone curve brings to
/// white. 1.0 or less means nothing needs compressing.
fn highlight_peak(frame: &Frame, white: f32) -> f32 {
    const STRIDE: usize = 7;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut samples = Vec::with_capacity(w * h / STRIDE + 1);
    for i in (0..w * h).step_by(STRIDE) {
        let at = (i / w) * frame.pitch + (i % w) * 8;
        let p = &frame.data[at..at + 8];
        let m = half(p, 0).max(half(p, 1)).max(half(p, 2)) / white;
        if m.is_finite() {
            samples.push(m);
        }
    }
    if samples.is_empty() {
        return 1.;
    }
    let k = ((samples.len() as f64 * 0.999) as usize).min(samples.len() - 1);
    let (_, p, _) = samples.select_nth_unstable_by(k, f32::total_cmp);
    p.clamp(1., 100.)
}

/// The largest channel `l` after the tone curve: unchanged up to [`KNEE`],
/// then an extended Reinhard curve (same slope at the knee) that reaches 1
/// at `peak`.
pub fn tone_map(l: f32, peak: f32) -> f32 {
    if peak <= 1. || l <= KNEE {
        return l.min(1.);
    }
    let x = (l - KNEE) / (1. - KNEE);
    let xw = (peak - KNEE) / (1. - KNEE);
    let f = (x * (1. + x / (xw * xw)) / (1. + x)).min(1.);
    KNEE + (1. - KNEE) * f
}

/// One linear scRGB pixel (in SDR-white units) to 8-bit sRGB.
fn tone_map_pixel(rgb: [f32; 3], peak: f32, lut: &[u8]) -> [u8; 3] {
    let rgb = rgb.map(|c| if c.is_finite() { c.max(0.) } else { 0. });
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    let scale = if m > 0. { tone_map(m, peak) / m } else { 0. };
    rgb.map(|c| lut[((c * scale).clamp(0., 1.) * (LUT_SIZE - 1) as f32).round() as usize])
}

const LUT_SIZE: usize = 1 << 16;

/// Linear [0, 1] (in [`LUT_SIZE`] steps) to 8-bit sRGB.
fn srgb_lut() -> &'static [u8] {
    static LUT: OnceLock<Vec<u8>> = OnceLock::new();
    LUT.get_or_init(|| (0..LUT_SIZE).map(|i| (srgb_encode(i as f32 / (LUT_SIZE - 1) as f32) * 255.).round() as u8).collect())
}

/// The sRGB transfer function.
pub fn srgb_encode(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1. / 2.4) - 0.055
    }
}

/// Turns the image the way the monitor shows it: Desktop Duplication hands
/// a rotated monitor's frame over unrotated. `quarter_turns` clockwise
/// (`DXGI_MODE_ROTATION_ROTATE90` is 1).
pub fn rotate(img: Rgb, quarter_turns: u8) -> Rgb {
    let (w, h) = (img.width as usize, img.height as usize);
    let turns = quarter_turns % 4;
    if turns == 0 {
        return img;
    }
    let (ow, oh) = if turns == 2 { (w, h) } else { (h, w) };
    let mut out = vec![0u8; w * h * 3];
    for y in 0..oh {
        for x in 0..ow {
            // The source pixel that lands at (x, y).
            let (u, v) = match turns {
                1 => (y, h - 1 - x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (w - 1 - y, x),
            };
            let s = (v * w + u) * 3;
            let d = (y * ow + x) * 3;
            out[d..d + 3].copy_from_slice(&img.pixels[s..s + 3]);
        }
    }
    Rgb { width: ow as u32, height: oh as u32, pixels: out }
}

/// The image as an 8-bit RGB PNG marked sRGB.
pub fn encode_png(img: &Rgb) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(img.pixels.len() / 2);
    {
        let mut encoder = png::Encoder::new(&mut out, img.width, img.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&img.pixels).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// f32 to binary16, exact for the values used here.
    fn f16(v: f32) -> [u8; 2] {
        let bits = v.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        if v == 0. {
            return sign.to_le_bytes();
        }
        if v.is_nan() {
            return 0x7e00u16.to_le_bytes();
        }
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let mant = ((bits >> 13) & 0x3ff) as u16;
        (sign | ((exp as u16) << 10) | mant).to_le_bytes()
    }

    fn scrgb(pixels: &[[f32; 3]]) -> Vec<u8> {
        pixels.iter().flat_map(|p| [f16(p[0]), f16(p[1]), f16(p[2]), f16(1.)].concat()).collect()
    }

    #[test]
    fn halves() {
        assert_eq!(half_to_f32(0x0000), 0.);
        assert_eq!(half_to_f32(0x3c00), 1.);
        assert_eq!(half_to_f32(0xc000), -2.);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0x7bff), 65504.);
        assert_eq!(half_to_f32(0x0001), f32::powi(2., -24));
        assert_eq!(half_to_f32(0x8001), -f32::powi(2., -24));
        assert!(half_to_f32(0x7c00).is_infinite());
        assert!(half_to_f32(0x7e00).is_nan());
        for v in [0.25f32, 1.5, 3., 80., -7.] {
            assert_eq!(half_to_f32(u16::from_le_bytes(f16(v))), v);
        }
    }

    #[test]
    fn bgra_rows_with_padding() {
        // 2x2, rows padded to 12 bytes.
        let data = [1, 2, 3, 255, 4, 5, 6, 255, 0, 0, 0, 0, 7, 8, 9, 0, 10, 11, 12, 0, 0, 0, 0, 0];
        let frame = Frame { data: &data, width: 2, height: 2, pitch: 12, format: Format::Bgra8 };
        let img = to_rgb(&frame, 1.).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(img.pixels, [3, 2, 1, 6, 5, 4, 9, 8, 7, 12, 11, 10]);
        let short = Frame { data: &data[..18], width: 2, height: 2, pitch: 12, format: Format::Bgra8 };
        assert!(to_rgb(&short, 1.).is_err());
        let empty = Frame { data: &[], width: 0, height: 0, pitch: 0, format: Format::Bgra8 };
        assert!(to_rgb(&empty, 1.).is_err());
    }

    #[test]
    fn sdr_content_on_an_hdr_desktop_is_unchanged() {
        // SDR white at 200 nits (2.5): white, mid grey, black, red.
        let w = 2.5;
        let data = scrgb(&[[w, w, w], [0.214 * w, 0.214 * w, 0.214 * w], [0., 0., 0.], [w, 0., 0.]]);
        let frame = Frame { data: &data, width: 4, height: 1, pitch: 32, format: Format::Rgba16Float };
        let img = to_rgb(&frame, w).unwrap();
        assert_eq!(img.pixels[..3], [255, 255, 255]);
        // 0.214 linear is sRGB 128 (the half floats round it a little).
        assert!(img.pixels[3..6].iter().all(|&c| (127..=129).contains(&c)), "{:?}", &img.pixels[3..6]);
        assert_eq!(img.pixels[6..12], [0, 0, 0, 255, 0, 0]);
    }

    #[test]
    fn hdr_highlights_are_compressed_not_clipped() {
        // Most of the frame at SDR white, the rest at 4x and 8x it.
        let mut px = vec![[1., 1., 1.]; 2000];
        px.extend(vec![[4., 4., 4.]; 10]);
        px.extend(vec![[8., 4., 2.]; 10]);
        let data = scrgb(&px);
        let n = px.len() as u32;
        let frame = Frame { data: &data, width: n, height: 1, pitch: n as usize * 8, format: Format::Rgba16Float };
        let img = to_rgb(&frame, 1.).unwrap();
        let at = |i: usize| [img.pixels[i * 3], img.pixels[i * 3 + 1], img.pixels[i * 3 + 2]];
        let (white, bright, coloured) = (at(0), at(2000), at(2010));
        // SDR white is below the brighter areas, which stay apart from it.
        assert!(white[0] < bright[0], "{white:?} {bright:?}");
        // The hue survives: red > green > blue.
        assert!(coloured[0] > coloured[1] && coloured[1] > coloured[2], "{coloured:?}");
    }

    #[test]
    fn tone_curve() {
        // Without highlights: the identity, clamped.
        assert_eq!(tone_map(0.5, 1.), 0.5);
        assert_eq!(tone_map(1.5, 1.), 1.);
        // With them: unchanged below the knee, monotonic, white at the peak.
        assert_eq!(tone_map(0.3, 6.), 0.3);
        let mut last = 0.;
        for i in 0..=60 {
            let v = tone_map(i as f32 / 10., 6.);
            assert!(v >= last && v <= 1.);
            last = v;
        }
        assert!((tone_map(6., 6.) - 1.).abs() < 1e-6);
        assert_eq!(tone_map(60., 6.), 1.);
        // Continuous at the knee.
        assert!((tone_map(KNEE + 1e-4, 6.) - KNEE).abs() < 1e-3);
    }

    #[test]
    fn negative_and_broken_values_are_black() {
        let data = scrgb(&[[-1., -0.5, 0.], [f32::NAN, 0., 0.]]);
        let frame = Frame { data: &data, width: 2, height: 1, pitch: 16, format: Format::Rgba16Float };
        assert_eq!(to_rgb(&frame, 1.).unwrap().pixels, [0; 6]);
    }

    #[test]
    fn rotations() {
        // 3x2:  a b c
        //       d e f
        let px: Vec<u8> = (0..6u8).flat_map(|i| [i, i, i]).collect();
        let img = Rgb { width: 3, height: 2, pixels: px };
        let ids = |r: &Rgb| r.pixels.chunks(3).map(|p| p[0]).collect::<Vec<_>>();
        assert_eq!(rotate(img.clone(), 0), img);
        // Clockwise:  d a
        //             e b
        //             f c
        let r = rotate(img.clone(), 1);
        assert_eq!((r.width, r.height), (2, 3));
        assert_eq!(ids(&r), [3, 0, 4, 1, 5, 2]);
        let r = rotate(img.clone(), 2);
        assert_eq!((r.width, r.height), (3, 2));
        assert_eq!(ids(&r), [5, 4, 3, 2, 1, 0]);
        let r = rotate(img.clone(), 3);
        assert_eq!((r.width, r.height), (2, 3));
        assert_eq!(ids(&r), [2, 5, 1, 4, 0, 3]);
        assert_eq!(rotate(rotate(rotate(rotate(img.clone(), 1), 1), 1), 1), img);
    }

    #[test]
    fn png_round_trip() {
        let img = Rgb { width: 2, height: 1, pixels: vec![10, 20, 30, 40, 50, 60] };
        let bytes = encode_png(&img).unwrap();
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap().to_rgb8();
        assert_eq!(decoded.as_raw(), &img.pixels);
        // What the background compression takes as it is: 8-bit, sRGB.
        let reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().unwrap();
        assert_eq!(reader.info().bit_depth, png::BitDepth::Eight);
        assert!(reader.info().srgb.is_some());
    }

    #[test]
    fn srgb_curve() {
        assert_eq!(srgb_encode(0.), 0.);
        assert!((srgb_encode(1.) - 1.).abs() < 1e-6);
        assert!((srgb_encode(0.2159) * 255. - 128.).abs() < 0.1);
        assert_eq!(srgb_lut()[0], 0);
        assert_eq!(srgb_lut()[LUT_SIZE - 1], 255);
    }
}
