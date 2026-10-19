//! Settings → Appearance on the Collection: the picture behind it (and the
//! screenshot slideshow), the accent taken from the game's cover, and the
//! choices that need the cards rebuilt. The theme reads the rest itself.

use std::time::Duration;

use gpui_kit::*;
use heronite_data::appearance::{Accent, Appearance, BackgroundSource, SLIDESHOW_SECS};
use heronite_images::{ImageRef, Variant};

use super::CollectionView;
use crate::theme;

#[derive(Default)]
pub(super) struct LookState {
    /// The appearance the cards were last built with.
    applied: Option<Appearance>,
    /// Count of slideshow steps; picks the screenshot behind the Collection.
    pub slide: usize,
    slideshow: Option<Task<()>>,
}

/// Size the accent is measured on: a cover shrunk to a few pixels.
const ACCENT_SAMPLE: (u32, u32) = (8, 12);

impl CollectionView {
    /// Runs at the start of each render.
    pub(super) fn sync_look(&mut self, cx: &mut Context<Self>) {
        let a = theme::appearance();
        let before = self.look.applied.replace(a);
        if before.is_some_and(|b| b.cover_label != a.cover_label) {
            self.refresh_groups();
        }
        if before.is_some_and(|b| b.font != a.font) {
            theme::apply_component_theme(cx);
        }
        self.sync_game_accent(a);
        self.sync_slideshow(a, cx);
    }

    /// [`Accent::Game`]: the average colour of the selected game's cover,
    /// weighted towards its vivid pixels and made light enough for text.
    fn sync_game_accent(&mut self, a: Appearance) {
        if a.accent != Accent::Game {
            theme::set_game_accent(None);
            return;
        }
        let Some(key) = self.selected.clone() else {
            theme::set_game_accent(None);
            return;
        };
        let (w, h) = ACCENT_SAMPLE;
        let id: SharedString = format!("a:{key}").into();
        let chains = &self.cover_chains;
        let img = self.images.get_or_request(&id, || chains.get(&key).cloned().unwrap_or_default(), w, h, Variant::Cover, 200);
        // Keep the last colour while the next one loads.
        if let Some(rgb) = img.as_ref().and_then(|img| img.as_bytes(0)).and_then(accent_from_bgra) {
            theme::set_game_accent(Some(rgb));
        }
    }

    /// A timer that moves the screenshot behind the Collection along.
    fn sync_slideshow(&mut self, a: Appearance, cx: &mut Context<Self>) {
        let wanted = a.background == BackgroundSource::Screenshot && a.background_slideshow && self.shots.items.len() > 1;
        if !wanted {
            self.look.slideshow = None;
            return;
        }
        if self.look.slideshow.is_some() {
            return;
        }
        self.look.slideshow = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(SLIDESHOW_SECS)).await;
            let alive = this
                .update(cx, |this, cx| {
                    this.look.slide = this.look.slide.wrapping_add(1);
                    cx.notify();
                })
                .is_ok();
            if !alive {
                break;
            }
        }));
    }

    /// The picture behind the Collection for game `key`: an id for the
    /// texture cache and the images to try. `None`: no picture (Solid).
    pub(super) fn stage_source(&self, key: &SharedString) -> Option<(String, Vec<ImageRef>)> {
        let art = || ("art".to_string(), self.stage_chains.get(key).cloned().unwrap_or_default());
        Some(match theme::appearance().background {
            BackgroundSource::GameArt => art(),
            BackgroundSource::Solid => return None,
            BackgroundSource::Cover => ("cover".into(), self.cover_chains.get(key).cloned().unwrap_or_default()),
            BackgroundSource::Screenshot => {
                let shots = &self.shots.items;
                if self.shots.game.as_ref() != Some(key) || shots.is_empty() {
                    return Some(art());
                }
                let i = self.look.slide % shots.len();
                (format!("shot{i}"), vec![ImageRef::File(shots[i].path.clone())])
            }
            BackgroundSource::Custom => {
                (format!("custom{}", theme::custom_background_epoch()), vec![ImageRef::File(self.app_paths.custom_background())])
            }
        })
    }

    /// Settings → Appearance → "Open the Collection in".
    pub(super) fn apply_start_view(&mut self) {
        use heronite_data::appearance::StartView;
        match theme::appearance().start_view {
            StartView::Last => {}
            StartView::Focus => self.ui.grid_view = false,
            StartView::Grid => self.ui.grid_view = true,
        }
    }

    /// Settings → Appearance → "Reset widths".
    pub(super) fn reset_panel_widths(&mut self, cx: &mut Context<Self>) {
        self.ui.list_column_w = None;
        self.ui.grid_drawer_w = None;
        self.save_ui();
        cx.notify();
    }
}

/// Average of the pixels (BGRA) weighted by their saturation, as a light
/// colour (HSL lightness 0.74) so it reads like the default accent. `None`
/// for an empty or grey image.
fn accent_from_bgra(bgra: &[u8]) -> Option<(u8, u8, u8)> {
    let (mut r, mut g, mut b, mut wsum) = (0f32, 0f32, 0f32, 0f32);
    for px in bgra.as_chunks::<4>().0 {
        let (pb, pg, pr) = (px[0] as f32 / 255., px[1] as f32 / 255., px[2] as f32 / 255.);
        let (max, min) = (pr.max(pg).max(pb), pr.min(pg).min(pb));
        // Vivid, not too dark: those pixels make the cover's colour.
        let w = (max - min) * max + 0.02;
        r += pr * w;
        g += pg * w;
        b += pb * w;
        wsum += w;
    }
    if wsum <= 0. {
        return None;
    }
    let (r, g, b) = (r / wsum, g / wsum, b / wsum);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max - min < 0.04 {
        return None;
    }
    let hue = if max == r {
        ((g - b) / (max - min)).rem_euclid(6.)
    } else if max == g {
        (b - r) / (max - min) + 2.
    } else {
        (r - g) / (max - min) + 4.
    } * 60.;
    let c = hsla(hue / 360., 0.75, 0.74, 1.).to_rgb();
    let to = |v: f32| (v * 255.).round().clamp(0., 255.) as u8;
    Some((to(c.r), to(c.g), to(c.b)))
}

#[cfg(test)]
mod tests {
    use super::accent_from_bgra;

    #[test]
    fn accent_follows_the_vivid_pixels() {
        // Two grey pixels and one vivid red: the accent is a light red.
        let px = [[128, 128, 128, 255], [100, 100, 100, 255], [20, 30, 220, 255]].concat();
        let (r, g, b) = accent_from_bgra(&px).unwrap();
        assert!(r > g && r > b, "{r} {g} {b}");
        assert!(g > 120, "light enough for text: {r} {g} {b}");
    }

    #[test]
    fn grey_art_has_no_accent() {
        assert_eq!(accent_from_bgra(&[[90, 90, 90, 255], [200, 200, 200, 255]].concat()), None);
        assert_eq!(accent_from_bgra(&[]), None);
    }
}
