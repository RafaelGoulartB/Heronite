//! Small pieces shared by the Epic pages, dialogs and Collection glue.

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{AppPaths, HeroicPaths};
use heronite_images::{ImageRef, Variant};

use crate::images::ImageStore;
use crate::theme;

/// Thin horizontal progress bar. `percent` is 0..=100; `None` draws an empty
/// track (unknown progress).
pub fn progress_bar(percent: Option<f32>, height: f32, color: Rgba) -> Div {
    let fill = percent.unwrap_or(0.).clamp(0., 100.) / 100.;
    div()
        .relative()
        .w_full()
        .h(px(height))
        .rounded_full()
        .bg(theme::white_a(0.14))
        .overflow_hidden()
        .child(div().absolute().top_0().left_0().bottom_0().w(relative(fill)).rounded_full().bg(color))
}

/// `1234.5` MiB -> `1.21 GiB`.
pub fn fmt_mib(mib: f64) -> String {
    if mib >= 1024. {
        format!("{:.2} GiB", mib / 1024.)
    } else {
        format!("{mib:.1} MiB")
    }
}

/// `12.3%`.
pub fn fmt_percent(p: f64) -> String {
    format!("{p:.1}%")
}

/// A page-local texture cache for covers (the Collection has its own).
pub struct Covers {
    store: ImageStore,
}

impl Covers {
    /// `access` finds the `Covers` inside the owner so the background loader
    /// can hand finished images over.
    pub fn new<V: 'static>(heroic: &HeroicPaths, app: &AppPaths, cx: &mut Context<V>, access: fn(&mut V) -> &mut Covers) -> Self {
        let (store, mut rx) = ImageStore::new(heroic.images_cache_dir(), app);
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let alive = this
                    .update(cx, |this, cx| {
                        for item in batch {
                            access(this).store.on_loaded(item);
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
        Self { store }
    }

    /// Texture of the cover at `url`, sized `w`x`h` logical px (requested on
    /// demand; `None` while it loads or when there is no cover).
    pub fn get(&mut self, url: Option<&str>, w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        let url = url.filter(|u| !u.is_empty())?;
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("e:{url}:{pw}x{ph}").into();
        let chain = || vec![ImageRef::Url(url.to_string())];
        self.store.get_or_request(&id, chain, pw, ph, Variant::Cover, 100).map(ImageSource::Render)
    }

    pub fn drop_evicted(&mut self, window: &mut Window) {
        self.store.drop_evicted(window);
    }
}

/// Cover box with a placeholder background while the image loads.
pub fn cover_box(src: Option<ImageSource>, w: f32, h: f32) -> Div {
    div()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded(px(5.))
        .overflow_hidden()
        .bg(theme::white_a(0.05))
        .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover).rounded(px(5.))))
}
