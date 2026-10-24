//! Image pipeline for the Heronite Collection screen.
//!
//! * [`load_cover`]  - object-fit *cover* tile art at exact physical size.
//! * [`load_fit`]    - whole image inside a box, aspect and transparency kept (logos).
//! * [`load_stage`]  - full-window blurred + saturated background.
//! * [`load_panel_glass`] - stronger blur variant used behind translucent panels.
//! * [`Loader`]      - bounded, prioritised, de-duplicating background loader.
//! * [`thumbs`]      - persistent resized-image cache (+ [`evict_lru`]).
//!
//! # Pixel format (GPUI)
//! [`Decoded::bgra`] is exactly what `gpui::RenderImage::new` expects: 4 bytes
//! per pixel in **B, G, R, A** order, tightly packed, row-major, **straight
//! (non-premultiplied) alpha** - the same layout GPUI itself produces by
//! `into_rgba8()` + swapping bytes 0 and 2 (`gpui-pre-0.3.8/src/elements/img.rs`
//! and `platform.rs::decode_static_image`); GPUI's shader premultiplies on the
//! GPU. Covers are opaque (A = 255) unless the source PNG/WebP has transparency.
//! Wrap without copying:
//! `image::Frame::new(decoded.into_image_buffer())` (the `image` crate is the
//! same 0.25 line GPUI uses, so the types are identical).

mod decode;
mod error;
mod fetch;
pub mod fx;
pub mod geometry;
mod loader;
mod resize;
pub mod thumbs;

pub use error::{ImageError, Result};
pub use fetch::{cache_path, url_digest};
pub use loader::{LoadResult, Loader, LoaderConfig, Request, RequestKey};
pub use resize::Filter;
pub use thumbs::{evict_downloads, evict_lru, EvictStats};

use std::path::PathBuf;
use std::time::Duration;

/// Default blur (physical px) used behind translucent panels.
pub const PANEL_GLASS_BLUR_PX: f32 = 20.0;
/// Relative luminance (linear light) above which [`Variant::Backdrop`] rolls
/// the art off, and the level it never goes past: white text keeps a 4.5:1
/// contrast over the brightest art once the Collection's scrims are on top.
pub const BACKDROP_KNEE: f32 = 0.15;
pub const BACKDROP_CAP: f32 = 0.26;

/// One place an image can come from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageRef {
    File(PathBuf),
    /// Fetched through Heronite's shared `images-cache` (see [`Opts::images_cache_dir`]).
    Url(String),
    /// The first file called `name` in `dir` or in one of its sub-folders
    /// (Steam keeps a game's art in hashed folders it names itself). Looked
    /// up by the loader's worker, never on the UI thread.
    Find {
        dir: PathBuf,
        name: String,
    },
}

impl ImageRef {
    /// The file behind a local candidate; `None` for a URL or when
    /// [`Find`](Self::Find) finds nothing. Reads the folder: worker threads only.
    pub fn local_path(&self) -> Option<PathBuf> {
        match self {
            ImageRef::File(p) => Some(p.clone()),
            ImageRef::Url(_) => None,
            ImageRef::Find { dir, name } => {
                let direct = dir.join(name);
                if direct.is_file() {
                    return Some(direct);
                }
                std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path().join(name)).filter(|p| p.is_file()).min()
            }
        }
    }
}

/// A decoded, resized image ready to hand to GPUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, BGRA, straight alpha. `capacity == len`.
    pub bgra: Vec<u8>,
}

impl Decoded {
    /// Zero-copy wrap for `image::Frame::new` (data stays BGRA, as GPUI wants).
    pub fn into_image_buffer(self) -> image::RgbaImage {
        image::RgbaImage::from_raw(self.width, self.height, self.bgra).expect("buffer size matches dimensions")
    }

    /// Take ownership out of a shared result without copying when unique.
    pub fn from_shared(a: std::sync::Arc<Decoded>) -> Decoded {
        std::sync::Arc::try_unwrap(a).unwrap_or_else(|a| (*a).clone())
    }

    /// Heap bytes held by the pixel buffer.
    pub fn byte_len(&self) -> usize {
        self.bgra.len()
    }
}

/// What to produce from a source image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Variant {
    /// Plain cover resize to the exact target.
    Cover,
    /// The whole image inside the box, aspect ratio and transparency kept (no
    /// crop, no blur) and never enlarged: a logo. The result is smaller than
    /// the target unless the image fills it on one axis.
    Fit,
    /// [`Variant::Fit`] with the light, colourless parts of the image (the
    /// white of a logo) turned to a near-black ink: a logo on a light
    /// surface, where white would vanish.
    FitInk,
    /// Full-window background: cover + `blur(blur_px) saturate(saturation)`.
    Stage { blur_px: f32, saturation: f32 },
    /// Same as `Stage` but cached separately; meant for a stronger blur.
    Glass { blur_px: f32, saturation: f32 },
    /// `Stage` for art that sits behind text (the Collection background):
    /// its brightest parts are also rolled off ([`fx::compress_highlights`]
    /// with [`BACKDROP_KNEE`] / [`BACKDROP_CAP`]) so white text stays legible
    /// over white or bright art, while darker art is untouched.
    Backdrop { blur_px: f32, saturation: f32 },
}

impl std::hash::Hash for Variant {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match *self {
            Variant::Cover => 0u8.hash(h),
            Variant::Fit => 4u8.hash(h),
            Variant::FitInk => 5u8.hash(h),
            Variant::Stage { blur_px, saturation } => (1u8, blur_px.to_bits(), saturation.to_bits()).hash(h),
            Variant::Glass { blur_px, saturation } => (2u8, blur_px.to_bits(), saturation.to_bits()).hash(h),
            Variant::Backdrop { blur_px, saturation } => (3u8, blur_px.to_bits(), saturation.to_bits()).hash(h),
        }
    }
}

/// Ink the light parts of a [`Variant::FitInk`] image turn to (RGB).
pub const LOGO_INK: (u8, u8, u8) = (0x15, 0x19, 0x1f);

/// Pipeline configuration. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Opts {
    /// HTTP cache of the data folder (`<data>/images-cache`).
    pub images_cache_dir: PathBuf,
    /// Resized-image cache (`~/.cache/heronite/thumbs`); `None` disables it.
    pub thumbs_dir: Option<PathBuf>,
    /// Allow downloading URLs that are not in `images_cache_dir`.
    pub allow_network: bool,
    pub connect_timeout: Duration,
    /// Whole-request timeout (connect + headers + body).
    pub http_timeout: Duration,
    pub max_download_bytes: u64,
    /// Resampling filter for the cover resize.
    pub filter: Filter,
    /// Decode JPEGs with DCT downscaling (1/2, 1/4, 1/8) when the target is smaller.
    pub jpeg_scaled_decode: bool,
    /// LZ4-compress thumbs on disk (~25-30% smaller, +~50us/250x375 on a warm
    /// read). Incompressible thumbs are stored raw regardless.
    pub thumb_compression: bool,
    /// How long a failed URL is not retried (needs `thumbs_dir`).
    pub negative_ttl: Duration,
    /// Upper bound on stage raster width (it is upscaled by the GPU).
    pub stage_max_width: u32,
    /// Target sigma (px) of the blurred stage raster; smaller => fewer pixels.
    pub stage_sigma_out: f32,
}

impl Opts {
    pub fn new(images_cache_dir: PathBuf, thumbs_dir: Option<PathBuf>) -> Self {
        Self {
            images_cache_dir,
            thumbs_dir,
            allow_network: true,
            connect_timeout: Duration::from_secs(8),
            http_timeout: Duration::from_secs(30),
            max_download_bytes: 32 * 1024 * 1024,
            filter: Filter::Lanczos3,
            jpeg_scaled_decode: true,
            thumb_compression: true,
            negative_ttl: Duration::from_secs(24 * 3600),
            stage_max_width: 960,
            stage_sigma_out: 8.0,
        }
    }
}

/// Resize `chain`'s first loadable image to exactly `target_w x target_h`
/// physical pixels with CSS `object-fit: cover` (centered crop).
pub fn load_cover(chain: &[ImageRef], target_w: u32, target_h: u32, opts: &Opts) -> Result<Decoded> {
    load(chain, target_w, target_h, Variant::Cover, opts)
}

/// The first loadable image of `chain` fitted inside `target_w x target_h`
/// ([`Variant::Fit`]): the result keeps the image's aspect ratio, so its size
/// is the fitted one, not the target.
pub fn load_fit(chain: &[ImageRef], target_w: u32, target_h: u32, opts: &Opts) -> Result<Decoded> {
    load(chain, target_w, target_h, Variant::Fit, opts)
}

/// Full-window background: cover-fit to `target_w x target_h`, Gaussian blur of
/// `blur_px` and CSS `saturate(saturation)`.
///
/// `blur_px` is in **physical pixels at the target resolution** (CSS blur
/// radius x scale factor). The raster is rendered at reduced resolution (see
/// [`geometry::stage_size`]) with the blur scaled accordingly; the returned
/// `Decoded` therefore has the target's aspect ratio but fewer pixels - draw it
/// stretched to the window and let the GPU upscale it.
pub fn load_stage(chain: &[ImageRef], target_w: u32, target_h: u32, blur_px: f32, saturation: f32, opts: &Opts) -> Result<Decoded> {
    load(chain, target_w, target_h, Variant::Stage { blur_px, saturation }, opts)
}

/// [`load_stage`] for art behind text, with its highlights rolled off
/// ([`Variant::Backdrop`]).
pub fn load_backdrop(chain: &[ImageRef], target_w: u32, target_h: u32, blur_px: f32, saturation: f32, opts: &Opts) -> Result<Decoded> {
    load(chain, target_w, target_h, Variant::Backdrop { blur_px, saturation }, opts)
}

/// Like [`load_stage`] with a stronger blur ([`PANEL_GLASS_BLUR_PX`] is a good
/// value) for use behind translucent panels. Cached independently.
pub fn load_panel_glass(chain: &[ImageRef], target_w: u32, target_h: u32, blur_px: f32, saturation: f32, opts: &Opts) -> Result<Decoded> {
    load(chain, target_w, target_h, Variant::Glass { blur_px, saturation }, opts)
}

/// Generic entry point behind the `load_*` helpers.
///
/// Chain semantics: candidates are tried in order; for each, the thumb cache
/// is consulted first (a hit returns immediately), then the source is decoded
/// (downloading URLs into the shared cache if needed). The first candidate that
/// yields an image wins and its result is stored in the thumb cache.
pub fn load(chain: &[ImageRef], target_w: u32, target_h: u32, variant: Variant, opts: &Opts) -> Result<Decoded> {
    if chain.is_empty() {
        return Err(ImageError::NoCandidates);
    }
    if target_w == 0 || target_h == 0 {
        return Err(ImageError::Decode("zero-sized target".into()));
    }
    let mut errors = Vec::new();
    for cand in chain {
        let Some(identity) = thumbs::source_identity(cand) else {
            errors.push(format!("{cand:?}: missing"));
            continue;
        };
        let key = opts.thumbs_dir.as_ref().map(|_| thumbs::thumb_key(&identity, variant, target_w, target_h, opts));
        if let (Some(dir), Some(key)) = (&opts.thumbs_dir, &key) {
            if let Some(hit) = thumbs::read(dir, key) {
                return Ok(hit);
            }
            if matches!(cand, ImageRef::Url(_)) && thumbs::is_negative(dir, &identity, opts.negative_ttl) {
                errors.push(format!("{cand:?}: recently failed"));
                continue;
            }
        }
        match produce(cand, target_w, target_h, variant, opts) {
            Ok(d) => {
                if let (Some(dir), Some(key)) = (&opts.thumbs_dir, &key) {
                    if let Err(e) = thumbs::write(dir, key, &d, opts.thumb_compression) {
                        log::warn!("thumb cache write failed: {e}");
                    }
                }
                return Ok(d);
            }
            Err(e) => {
                log::debug!("candidate {cand:?} failed: {e}");
                // Only remember definitive failures of remote sources
                // (404, HTML, undecodable) - not offline/timeouts.
                if let (ImageRef::Url(_), Some(dir)) = (cand, &opts.thumbs_dir) {
                    if is_definitive(&e) {
                        thumbs::mark_negative(dir, &identity);
                    }
                }
                errors.push(format!("{cand:?}: {e}"));
            }
        }
    }
    Err(ImageError::AllFailed(errors))
}

fn is_definitive(e: &ImageError) -> bool {
    match e {
        ImageError::NotAnImage(_) | ImageError::Decode(_) => true,
        ImageError::Http(m) => m.contains("status 4") || m.contains("http status: 4"),
        _ => false,
    }
}

fn produce(cand: &ImageRef, tw: u32, th: u32, variant: Variant, opts: &Opts) -> Result<Decoded> {
    let bytes = match cand {
        ImageRef::File(p) => fetch::read_file(p)?,
        ImageRef::Url(u) => fetch::url_bytes(u, opts)?,
        ImageRef::Find { .. } => fetch::read_file(&cand.local_path().ok_or_else(|| ImageError::Io(format!("{cand:?}: not found")))?)?,
    };
    match variant {
        Variant::Fit | Variant::FitInk => {
            let raster = decode::decode(&bytes, tw, th, opts.jpeg_scaled_decode)?;
            drop(bytes);
            let (w, h, mut px) = resize::fit_to_rgba(raster, tw, th, opts.filter)?;
            if variant == Variant::FitInk {
                fx::ink_light_parts(&mut px, LOGO_INK);
            }
            fx::swap_rb(&mut px);
            Ok(Decoded { width: w, height: h, bgra: px })
        }
        Variant::Cover => {
            let raster = decode::decode(&bytes, tw, th, opts.jpeg_scaled_decode)?;
            drop(bytes);
            let mut px = resize::cover_to_rgba(raster, tw, th, opts.filter)?;
            fx::swap_rb(&mut px);
            Ok(Decoded { width: tw, height: th, bgra: px })
        }
        Variant::Stage { blur_px, saturation } | Variant::Glass { blur_px, saturation } | Variant::Backdrop { blur_px, saturation } => {
            let (ow, oh, scale) = geometry::stage_size(tw, th, blur_px, opts.stage_max_width, opts.stage_sigma_out);
            let raster = decode::decode(&bytes, ow, oh, opts.jpeg_scaled_decode)?;
            drop(bytes);
            let mut px = resize::cover_to_rgba(raster, ow, oh, opts.filter)?;
            fx::flatten_over_black(&mut px);
            fx::gaussian_blur(&mut px, ow as usize, oh as usize, blur_px * scale);
            fx::saturate(&mut px, saturation, false);
            if matches!(variant, Variant::Backdrop { .. }) {
                fx::compress_highlights(&mut px, BACKDROP_KNEE, BACKDROP_CAP);
            }
            fx::swap_rb(&mut px);
            Ok(Decoded { width: ow, height: oh, bgra: px })
        }
    }
}

#[cfg(test)]
mod tests;
