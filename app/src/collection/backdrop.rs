//! Stage and glass images: the blurred art behind the Collection and behind
//! glass surfaces.

use std::sync::Arc;

use gpui_kit::*;
use heronite_images::Variant;

use super::CollectionView;

impl CollectionView {
    /// Current stage texture: the selected game's, or the previous one while
    /// it loads.
    pub(super) fn stage_image(&mut self, window: &Window) -> Option<(SharedString, ImageSource)> {
        if !self.stage_enabled {
            // Off: the big blurred texture is not kept around.
            if self.shown_stage.take().is_some() {
                self.images.forget_prefix("s:");
            }
            return None;
        }
        let key = self.selected.clone()?;
        let scale = window.scale_factor();
        let size = window.viewport_size();
        // A blurred stage made at half the window's resolution looks the same
        // once the GPU scales it up, for a quarter of the memory and work.
        let blur = self.blur_px * scale;
        let shrink = if blur >= 4. { 2. } else { 1. };
        let pw = (f32::from(size.width) * scale / shrink).round() as u32;
        let ph = (f32::from(size.height) * scale / shrink).round() as u32;
        let blur = blur / shrink;
        let sat = self.saturation;
        let id: SharedString = format!("s:{key}:{pw}x{ph}:{blur}:{sat}").into();
        let chains = &self.stage_chains;
        let ready = self.images.get_or_request(
            &id,
            || chains.get(&key).cloned().unwrap_or_default(),
            pw,
            ph,
            Variant::Stage { blur_px: blur, saturation: sat },
            1000,
        );
        if let Some(img) = ready {
            self.shown_stage = Some((key, img));
        } else if self.stage_chains.get(&key).is_none_or(|c| c.is_empty()) {
            self.shown_stage = None;
        }
        self.shown_stage.as_ref().map(|(k, img)| (k.clone(), ImageSource::Render(img.clone())))
    }

    /// Blurred art that glass surfaces (menus, dialogs, pages) paint behind
    /// their translucent fill: the selected game's art at a strong blur and a
    /// tiny resolution (`Variant::Glass`, ~1/5 of the window), cached like the
    /// stage. Independent of the Collection background: it exists whenever the
    /// Interface is Transparent (and not fully opaque). `None` (no art, Solid
    /// interface) makes the surfaces fall back to a solid fill, and nothing is
    /// generated or kept.
    pub(super) fn glass_image(&mut self, window: &Window) -> Option<Arc<RenderImage>> {
        use crate::ui::glass::{glass_target, GLASS_SATURATION};
        if !crate::theme::glass_shows_art() {
            if self.shown_glass.take().is_some() {
                self.images.forget_prefix("g:");
            }
            return None;
        }
        let key = self.selected.clone()?;
        let scale = window.scale_factor();
        let size = window.viewport_size();
        let (tw, th) = glass_target(f32::from(size.width), f32::from(size.height), scale);
        let blur = crate::theme::glass_blur_px();
        let id: SharedString = format!("g:{key}:{tw}x{th}:{blur}").into();
        let chains = &self.stage_chains;
        let ready = self.images.get_or_request(
            &id,
            || chains.get(&key).cloned().unwrap_or_default(),
            tw,
            th,
            Variant::Glass { blur_px: blur * scale, saturation: GLASS_SATURATION },
            500,
        );
        if let Some(img) = ready {
            self.shown_glass = Some((key, img));
        } else if self.stage_chains.get(&key).is_none_or(|c| c.is_empty()) {
            self.shown_glass = None;
        }
        self.shown_glass.as_ref().map(|(_, img)| img.clone())
    }
}
