//! Image textures for dialogs: a small `ImageStore` hosted in its own entity
//! so several dialogs (and nested ones) can show covers, thumbnails and
//! previews without touching the Collection's store.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_images::{ImageRef, Variant};
use heronite_metadata::picture_size::resolution_label;
use heronite_metadata::{MetadataService, WebImage};

use super::DialogCtx;
use crate::images::ImageStore;

pub struct DialogImages {
    store: ImageStore,
    scale: f32,
}

impl DialogImages {
    pub fn new(ctx: &DialogCtx, cx: &mut Context<Self>) -> Self {
        let (store, mut rx) = ImageStore::new(ctx.data_paths.images_cache_dir(), &ctx.app);
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
pub fn chain_for_url(data_paths: &DataPaths, screenshots: Option<&std::path::Path>, url: &str) -> Vec<ImageRef> {
    if url.is_empty() {
        return Vec::new();
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return vec![ImageRef::Url(url.to_string())];
    }
    let path = if url.starts_with("localart:") {
        data_paths.resolve_localart(url, screenshots)
    } else if let Some(rest) = url.strip_prefix("file://") {
        Some(std::path::PathBuf::from(heronite_data::paths::percent_decode(rest)))
    } else if let Some(rest) = url.strip_prefix("file:") {
        Some(std::path::PathBuf::from(heronite_data::paths::percent_decode(rest)))
    } else if url.starts_with('/') || std::path::Path::new(url).is_absolute() {
        Some(std::path::PathBuf::from(url))
    } else {
        None
    };
    path.filter(|p| p.exists()).map(|p| vec![ImageRef::File(p)]).unwrap_or_default()
}

/// `<img>` filling its box with the texture (cover crop), or a placeholder.
pub fn picture(src: Option<ImageSource>) -> Div {
    let d = div().relative().overflow_hidden().bg(crate::theme::recess_a(0.28));
    match src {
        Some(src) => d.child(img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)),
        None => d,
    }
}

/// What a game shows for each of its pictures, with `art` (its pictures set
/// by hand, maybe changed since the library was read) in place of the ones
/// the library holds: each chain is the picture set by hand, then what the
/// game shows without it. Checks local files: build it on a change, not
/// per frame.
#[derive(Clone, Default)]
pub struct GameChains {
    /// The tall cover (its chain, whatever it holds).
    pub tall: Vec<ImageRef>,
    /// The background.
    pub stage: Vec<ImageRef>,
    /// The game's own square and wide covers (empty: none).
    pub square: Vec<ImageRef>,
    pub wide: Vec<ImageRef>,
}

impl GameChains {
    pub fn of(ctx: &DialogCtx, game: &heronite_data::CollectionGame, art: &heronite_data::models::CollectionGameArt) -> Self {
        use heronite_data::art as data_art;
        use heronite_data::cover_family::CoverFamily;
        use heronite_metadata::ArtKind;
        let shots = ctx.lib.settings.screenshots_folder();
        let art_ctx = heronite_data::ArtContext::new(&ctx.data_paths, shots);
        let url_chain = |a: Option<&heronite_data::models::CollectionGameArt>, kind: ArtKind| {
            a.and_then(|a| kind.of_art(a)).map(|u| chain_for_url(&ctx.data_paths, shots, u)).unwrap_or_default()
        };
        let chain = |kind: ArtKind, defaults: Vec<heronite_data::ImageRef>| {
            let old = url_chain(game.art.as_deref(), kind);
            let mut out = url_chain(Some(art), kind);
            for r in crate::images::to_chain(&defaults) {
                if !old.contains(&r) && !out.contains(&r) {
                    out.push(r);
                }
            }
            out
        };
        Self {
            tall: chain(ArtKind::Cover, data_art::card_cover(&art_ctx, game)),
            stage: chain(ArtKind::Hero, data_art::stage_art(&art_ctx, game)),
            square: chain(ArtKind::CoverSquare, data_art::family_art(&art_ctx, game, CoverFamily::Square)),
            wide: chain(ArtKind::CoverWide, data_art::family_art(&art_ctx, game, CoverFamily::Wide)),
        }
    }

    /// What a view of `family` draws, and where it comes from.
    pub fn family(&self, family: heronite_data::cover_family::CoverFamily) -> (Vec<ImageRef>, heronite_data::art::CoverOrigin) {
        use heronite_data::cover_family::CoverFamily;
        let own = match family {
            CoverFamily::Tall => &[][..],
            CoverFamily::Square => &self.square,
            CoverFamily::Wide => &self.wide,
        };
        heronite_data::art::pick_family(family, own, &self.stage, &self.tall)
    }
}

/// The sizes of the pictures a dialog lists, for the captions under them.
/// A source says the size of most of its pictures; the others are read from
/// the picture's own first bytes (`MetadataService::picture_sizes`, off the
/// UI thread). The texture of a thumbnail is no help: it is made to fit its
/// box, whatever the picture measures.
pub struct PictureSizes {
    service: Arc<MetadataService>,
    /// By address; `None`: asked, and it cannot be read.
    known: HashMap<String, Option<(u32, u32)>>,
    asked: HashSet<String>,
}

impl PictureSizes {
    pub fn new(service: Arc<MetadataService>) -> Self {
        Self { service, known: HashMap::new(), asked: HashSet::new() }
    }

    /// The size read of `url`, once it was.
    pub fn read(&self, url: &str) -> Option<(u32, u32)> {
        self.known.get(url).copied().flatten()
    }

    /// The caption of a found picture: the size its source gave, else the
    /// one read; `None` while neither is known.
    pub fn caption(&self, image: &WebImage) -> Option<String> {
        resolution_label(image.width, image.height).or_else(|| self.read(&image.image_url).and_then(|(w, h)| resolution_label(w, h)))
    }

    /// The caption of a picture of the game (a web address, `localart://`, a
    /// file) once read.
    pub fn caption_of(&self, url: &str) -> Option<String> {
        self.read(url).and_then(|(w, h)| resolution_label(w, h))
    }

    /// Drops what was read of local pictures: a save writes a new one under
    /// the same name.
    pub fn forget_local(&mut self) {
        let web = |u: &String| u.starts_with("http://") || u.starts_with("https://");
        self.known.retain(|u, _| web(u));
        self.asked.retain(web);
    }

    /// Reads the sizes of `urls` not asked yet; each is asked once.
    pub fn ask(&mut self, urls: impl IntoIterator<Item = String>, cx: &mut Context<Self>) {
        let urls: Vec<String> = urls.into_iter().filter(|u| !u.is_empty() && self.asked.insert(u.clone())).collect();
        if urls.is_empty() {
            return;
        }
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let sizes = cx.background_spawn(async move { service.picture_sizes(&urls) }).await;
            this.update(cx, |this, cx| {
                this.known.extend(sizes);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// The address `chain` starts with, as `PictureSizes` asks for it.
pub fn first_address(chain: &[ImageRef]) -> Option<String> {
    match chain.first()? {
        ImageRef::Url(u) => Some(u.clone()),
        ImageRef::File(p) => Some(p.to_string_lossy().into_owned()),
        _ => None,
    }
}
