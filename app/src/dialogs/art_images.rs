//! Image textures for dialogs: a small `ImageStore` hosted in its own entity
//! so several dialogs (and nested ones) can show covers, thumbnails and
//! previews without touching the Collection's store.

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::HeroicPaths;
use heronite_images::{ImageRef, Variant};

use super::DialogCtx;
use crate::images::ImageStore;

pub struct DialogImages {
    store: ImageStore,
    scale: f32,
}

impl DialogImages {
    pub fn new(ctx: &DialogCtx, cx: &mut Context<Self>) -> Self {
        let (store, mut rx) = ImageStore::new(ctx.heroic.images_cache_dir(), &ctx.app);
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let alive = this
                    .update(cx, |this, cx| {
                        for item in batch {
                            this.store.on_loaded(item);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        Self { store, scale: 1. }
    }

    /// Call from `render` (needs the window for the scale and GPU cleanup).
    pub fn frame(&mut self, window: &mut Window) {
        self.scale = window.scale_factor();
        self.store.drop_evicted(window);
    }

    /// Texture of `chain` fitted to `w`×`h` logical px (loaded on demand).
    pub fn get(&mut self, id: &str, chain: Vec<ImageRef>, w: f32, h: f32) -> Option<ImageSource> {
        let (pw, ph) = ((w * self.scale).round().max(1.) as u32, (h * self.scale).round().max(1.) as u32);
        let key: SharedString = format!("{id}:{pw}x{ph}").into();
        self.store.get_or_request(&key, || chain, pw, ph, Variant::Cover, 100).map(ImageSource::Render)
    }

}

/// Chain for a stored image URL (`http(s)`, `localart://`, `file:`, path).
pub fn chain_for_url(heroic: &HeroicPaths, screenshots: Option<&std::path::Path>, url: &str) -> Vec<ImageRef> {
    if url.is_empty() {
        return Vec::new();
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return vec![ImageRef::Url(url.to_string())];
    }
    let path = if url.starts_with("localart:") {
        heroic.resolve_localart(url, screenshots)
    } else if let Some(rest) = url.strip_prefix("file://") {
        Some(std::path::PathBuf::from(heronite_data::paths::percent_decode(rest)))
    } else if let Some(rest) = url.strip_prefix("file:") {
        Some(std::path::PathBuf::from(heronite_data::paths::percent_decode(rest)))
    } else if url.starts_with('/') {
        Some(std::path::PathBuf::from(url))
    } else {
        None
    };
    path.filter(|p| p.exists()).map(|p| vec![ImageRef::File(p)]).unwrap_or_default()
}

/// `<img>` filling its box with the texture (cover crop), or a placeholder.
pub fn picture(src: Option<ImageSource>) -> Div {
    let d = div().relative().overflow_hidden().bg(crate::theme::black_a(0.28));
    match src {
        Some(src) => d.child(img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)),
        None => d,
    }
}
