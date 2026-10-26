//! Small pieces shared by the Epic pages, dialogs and Collection glue.

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{AppPaths, DataPaths};
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
        .bg(theme::ink_a(0.14))
        .overflow_hidden()
        .child(div().absolute().top_0().left_0().bottom_0().w(relative(fill)).rounded_full().bg(color))
}

/// A bar with a light sweeping across, for work that gives no percentage.
pub fn busy_bar(id: impl Into<ElementId>, height: f32) -> AnyElement {
    let track = div().relative().w_full().h(px(height)).rounded_full().bg(theme::ink_a(0.14)).overflow_hidden();
    let sweep = div().absolute().top_0().bottom_0().w(relative(0.3)).rounded_full().bg(theme::accent());
    if !theme::animations() {
        return track.child(sweep.left(relative(0.35))).into_any_element();
    }
    track
        .child(sweep.with_animation(id, Animation::new(std::time::Duration::from_millis(1400)).repeat().with_max_fps(30.), |s, t| {
            s.left(relative(t * 1.3 - 0.3))
        }))
        .into_any_element()
}

/// `1234.5` MiB -> `1.21 GiB`.
pub fn fmt_mib(mib: f64) -> String {
    if mib >= 1024. {
        format!("{} GiB", heronite_i18n::decimal(mib / 1024., 2))
    } else {
        format!("{} MiB", heronite_i18n::decimal(mib, 1))
    }
}

/// `12.3%`.
pub fn fmt_percent(p: f64) -> String {
    format!("{}%", heronite_i18n::decimal(p, 1))
}

/// A page-local texture cache for covers (the Collection has its own).
pub struct Covers {
    store: ImageStore,
}

impl Covers {
    /// `access` finds the `Covers` inside the owner so the background loader
    /// can hand finished images over.
    pub fn new<V: 'static>(data_paths: &DataPaths, app: &AppPaths, cx: &mut Context<V>, access: fn(&mut V) -> &mut Covers) -> Self {
        let (store, mut rx) = ImageStore::new(data_paths.images_cache_dir(), app);
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

    /// Frees every cover texture (the page went out of sight; they reload
    /// from the disk cache when it comes back).
    pub fn release(&mut self, window: &mut Window) {
        self.store.forget_prefix("");
        self.store.drop_evicted(window);
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

    /// Like [`Covers::get`] for a library game's cover chain (`key` names
    /// the game in the cache id).
    pub fn get_chain(&mut self, key: &str, chain: &[ImageRef], w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        if chain.is_empty() {
            return None;
        }
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("c:{key}:{pw}x{ph}").into();
        self.store.get_or_request(&id, || chain.to_vec(), pw, ph, Variant::Cover, 100).map(ImageSource::Render)
    }

    /// Wide art (stage chain) cropped to `w`x`h`, like `object-fit: cover`.
    pub fn get_art(&mut self, key: &str, chain: &[ImageRef], w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        if chain.is_empty() {
            return None;
        }
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("a:{key}:{pw}x{ph}").into();
        self.store.get_or_request(&id, || chain.to_vec(), pw, ph, Variant::Cover, 90).map(ImageSource::Render)
    }

    /// Blurred art for a full-size background (`w`x`h` logical px; the
    /// pipeline renders it small and the GPU scales it up).
    pub fn get_blurred(&mut self, key: &str, chain: &[ImageRef], w: f32, h: f32, blur_px: f32, saturation: f32) -> Option<ImageSource> {
        if chain.is_empty() {
            return None;
        }
        let (pw, ph) = (w.round().max(1.) as u32, h.round().max(1.) as u32);
        let id: SharedString = format!("b:{key}:{pw}x{ph}").into();
        self.store.get_or_request(&id, || chain.to_vec(), pw, ph, Variant::Stage { blur_px, saturation }, 80).map(ImageSource::Render)
    }

    /// An image file (a screenshot) cropped to `w`x`h`; an http(s) address
    /// (a console theme's picture) is fetched instead.
    pub fn get_file(&mut self, path: &std::path::Path, w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("f:{}:{pw}x{ph}", path.display()).into();
        let text = path.to_string_lossy();
        let chain = || {
            if text.starts_with("http://") || text.starts_with("https://") {
                vec![ImageRef::Url(text.to_string())]
            } else {
                vec![ImageRef::File(path.to_path_buf())]
            }
        };
        self.store.get_or_request(&id, chain, pw, ph, Variant::Cover, 70).map(ImageSource::Render)
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
        .bg(theme::ink_a(0.05))
        .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover).rounded(px(5.))))
}
