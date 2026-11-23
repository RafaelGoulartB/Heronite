//! Settings → Appearance → "Cover fit" set to "Whole cover" (`CoverFit::Whole`):
//! a cover is never cut nor stretched. Its picture is requested with
//! `Variant::Fit` (the whole image, aspect kept) under the cropped texture's
//! id plus [`ID_SUFFIX`], so the invalidation of a game's `c:{key}:` (or
//! `q:` / `w:` / background) prefix drops both. The view then draws it in
//! a [`Shelf`]'s box, which the ring, shadow and badges follow.
//!
//! Covers side by side look alike: a picture takes the family of its shape
//! (`picture_size::family_of`: tall, square or wide, as the sources file box
//! art), and the covers of one family on a row share one height, the
//! tallest that lets the widest of them fit its place. A row of 2:3 covers
//! in 2:3 places fills them as Crop does; a 3:4 box among them brings the
//! row's tall covers down to its height, like spines on a shelf, and only
//! their widths differ. Another row keeps its own height.

use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_images::Variant;
use heronite_metadata::picture_size::family_of;

use crate::theme;

/// Appended to a cover texture's id when it holds the whole picture.
pub(super) const ID_SUFFIX: &str = ":whole";

/// "Whole cover" is picked in Settings → Appearance.
pub(super) fn on() -> bool {
    theme::appearance().whole_covers()
}

/// The texture variant and id suffix of a cover drawn whole (`whole`) or cropped.
pub(super) fn variant(whole: bool) -> (Variant, &'static str) {
    if whole {
        (Variant::Fit { ink: None }, ID_SUFFIX)
    } else {
        (Variant::Cover, "")
    }
}

/// The heights the covers of one row share, one per family (see the
/// module's doc), for places of `w`×`h` logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shelf {
    w: f32,
    h: f32,
    /// Tall, square, wide; `None` while the row has no loaded picture of it.
    heights: [Option<f32>; 3],
}

impl Shelf {
    /// The shelf of a row of covers of `w`×`h` places.
    pub fn of<'a>(pictures: impl IntoIterator<Item = Option<&'a ImageSource>>, w: f32, h: f32) -> Self {
        Self::of_sizes(pictures.into_iter().filter_map(size_of), w, h)
    }

    fn of_sizes(sizes: impl IntoIterator<Item = (u32, u32)>, w: f32, h: f32) -> Self {
        let mut heights = [None; 3];
        for (iw, ih) in sizes {
            let slot: &mut Option<f32> = &mut heights[family_ix(iw, ih)];
            let fit = fit_height((iw, ih), w, h);
            *slot = Some(slot.map_or(fit, |s: f32| s.min(fit)));
        }
        Self { w, h, heights }
    }

    /// The box `src` is drawn in on this shelf: the whole place while the
    /// picture is not loaded (or is not a decoded texture).
    pub fn cover_box(&self, src: Option<&ImageSource>) -> (f32, f32) {
        size_of(src).map_or((self.w, self.h), |size| self.box_of(size))
    }

    fn box_of(&self, (iw, ih): (u32, u32)) -> (f32, f32) {
        let shape = iw as f32 / ih as f32;
        let height = self.heights[family_ix(iw, ih)].unwrap_or_else(|| fit_height((iw, ih), self.w, self.h));
        // The shelf's height lets the row's widest picture fit; a picture
        // missing from it (not loaded when the shelf was made) still fits.
        if shape * height <= self.w + 0.01 {
            (shape * height, height)
        } else {
            (self.w, self.w / shape)
        }
    }
}

/// The box of `src` alone in a place of `w`×`h`: as large as it fits.
pub(super) fn whole_box(src: Option<&ImageSource>, w: f32, h: f32) -> (f32, f32) {
    Shelf::of([src], w, h).cover_box(src)
}

/// A decoded picture's size, px.
fn size_of(src: Option<&ImageSource>) -> Option<(u32, u32)> {
    match src {
        Some(ImageSource::Render(img)) => {
            let s = img.size(0);
            (s.width.0 > 0 && s.height.0 > 0).then_some((s.width.0 as u32, s.height.0 as u32))
        }
        _ => None,
    }
}

fn family_ix(iw: u32, ih: u32) -> usize {
    match family_of(iw, ih) {
        CoverFamily::Tall => 0,
        CoverFamily::Square => 1,
        CoverFamily::Wide => 2,
    }
}

/// The height of a picture of `iw`×`ih` px as large as it fits in `w`×`h`.
fn fit_height((iw, ih): (u32, u32), w: f32, h: f32) -> f32 {
    h.min(w * ih as f32 / iw as f32)
}

#[cfg(test)]
mod tests {
    use super::Shelf;

    fn near((w, h): (f32, f32), (ew, eh): (f32, f32)) -> bool {
        (w - ew).abs() < 0.01 && (h - eh).abs() < 0.01
    }

    /// The boxes of a row of pictures in 162×243 (2:3) places.
    fn row(sizes: &[(u32, u32)]) -> Vec<(f32, f32)> {
        let shelf = Shelf::of_sizes(sizes.iter().copied(), 162., 243.);
        sizes.iter().map(|s| shelf.box_of(*s)).collect()
    }

    #[test]
    fn a_row_takes_the_cut_of_its_shortest_box_and_families_keep_their_height() {
        for (what, sizes, want) in [
            ("2:3 covers fill their places", &[(600, 900), (600, 900), (300, 450)][..], &[(162., 243.), (162., 243.), (162., 243.)][..]),
            (
                "a 3:4 box brings the tall covers to its height",
                &[(600, 900), (300, 400), (500, 700), (440, 720)],
                &[(144., 216.), (162., 216.), (154.29, 216.), (132., 216.)],
            ),
            // 162 / (5/7) = 226.8: the 2:3 covers come down 16 px, not 27.
            ("a 5:7 box among 2:3 ones sets a smaller cut", &[(600, 900), (500, 700)], &[(151.2, 226.8), (162., 226.8)]),
            // Tall, square (PS1, DS), wide (SNES).
            (
                "families keep their own height on one row",
                &[(600, 900), (600, 600), (546, 600), (700, 490)],
                &[(162., 243.), (162., 162.), (147.42, 162.), (162., 113.4)],
            ),
        ] {
            let boxes = row(sizes);
            assert_eq!(boxes.len(), want.len());
            for (b, w) in boxes.iter().zip(want) {
                assert!(near(*b, *w), "{what}: {b:?} vs {w:?}");
            }
        }
    }

    #[test]
    fn a_picture_the_shelf_did_not_measure_still_fits_and_wide_places_cap_the_height() {
        // Loaded after the shelf was made.
        let shelf = Shelf::of_sizes([(600, 900)], 162., 243.);
        assert!(near(shelf.box_of((300, 400)), (162., 216.)));
        // A tall cover in a wide place takes its height.
        let shelf = Shelf::of_sizes([(600, 900), (300, 400)], 288., 162.);
        assert!(near(shelf.box_of((600, 900)), (108., 162.)));
        assert!(near(shelf.box_of((300, 400)), (121.5, 162.)));
    }

    #[test]
    fn nothing_loaded_keeps_the_place() {
        assert_eq!(Shelf::of([None], 200., 300.).cover_box(None), (200., 300.));
    }
}
