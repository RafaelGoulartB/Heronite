//! Covers by family (`heronite_data::cover_family`): every view asks for the
//! family it draws, and [`art::pick_family`] picks the picture: the game's
//! own art of that family, else (wide views) its background, else its tall
//! cover cropped.
//!
//! The tall cover and the background keep their chains (`cover_chains`,
//! `stage_chains`) and their texture ids, so a library with only tall
//! covers draws and caches exactly what it did: a crop of the tall cover is
//! requested as `c:` (the tall cover's id), a background as the view's own
//! prefix. Only a game's own square or wide art has ids of its own (`q:`,
//! `w:`).

use std::collections::HashMap;

use gpui_kit::*;
use heronite_data::art::{self, CoverOrigin};
use heronite_data::cover_family::CoverFamily;
use heronite_data::{ArtContext, CollectionGame};
use heronite_images::{ImageRef, Variant};

use super::CollectionView;
use crate::images::to_chain;

/// The games' own square and wide covers, resolved at load; only games that
/// have some are listed.
#[derive(Default, Clone, PartialEq)]
pub(super) struct FamilyChains {
    pub square: HashMap<SharedString, Vec<ImageRef>>,
    pub wide: HashMap<SharedString, Vec<ImageRef>>,
}

impl FamilyChains {
    /// Adds `game`'s own art (off the UI thread: it checks local files).
    pub fn add(&mut self, ctx: &ArtContext<'_>, key: &SharedString, game: &CollectionGame) {
        for (family, map) in [(CoverFamily::Square, &mut self.square), (CoverFamily::Wide, &mut self.wide)] {
            let own = to_chain(&art::family_art(ctx, game, family));
            if !own.is_empty() {
                map.insert(key.clone(), own);
            }
        }
    }

    fn own(&self, family: CoverFamily, key: &str) -> &[ImageRef] {
        let map = match family {
            CoverFamily::Tall => return &[],
            CoverFamily::Square => &self.square,
            CoverFamily::Wide => &self.wide,
        };
        map.get(key).map_or(&[], Vec::as_slice)
    }

    /// The id prefixes of the games whose own art changed in `next`.
    pub fn stale(&self, next: &FamilyChains) -> Vec<String> {
        let mut out = Vec::new();
        for (prefix, now, then) in [("q", &self.square, &next.square), ("w", &self.wide, &next.wide)] {
            for (key, chain) in now {
                if then.get(key) != Some(chain) {
                    out.push(format!("{prefix}:{key}:"));
                }
            }
        }
        out
    }
}

/// A view's picture of a family.
pub(super) struct FamilyPick {
    pub src: Option<ImageSource>,
    /// Every picture of the chain failed to load.
    pub failed: bool,
}

impl CollectionView {
    /// Where the picture of `family` for `key` comes from (see [`Self::family_art`]).
    pub(super) fn family_origin(&self, key: &str, cover_key: &str, family: CoverFamily) -> CoverOrigin {
        let has_stage = family == CoverFamily::Wide && self.stage_chains.get(key).is_some_and(|c| !c.is_empty());
        art::family_origin(family, !self.family_chains.own(family, cover_key).is_empty(), has_stage)
    }

    /// The picture of `family` for the game `key` at `pw`×`ph` physical px.
    /// `cover_key`: the game whose cover a card shows (another copy's, when
    /// copies are merged); `background`: the texture id prefix the view
    /// gives its background (a wide view without a wide cover).
    pub(super) fn family_art(
        &mut self,
        key: &SharedString,
        cover_key: &SharedString,
        family: CoverFamily,
        (pw, ph): (u32, u32),
        priority: i32,
        background: &str,
    ) -> FamilyPick {
        let own = self.family_chains.own(family, cover_key);
        let stage = match family {
            CoverFamily::Wide => self.stage_chains.get(key).map_or(&[][..], Vec::as_slice),
            _ => &[],
        };
        let tall = self.cover_chains.get(cover_key).map_or(&[][..], Vec::as_slice);
        let id: SharedString = match (art::family_origin(family, !own.is_empty(), !stage.is_empty()), family) {
            (CoverOrigin::Own, CoverFamily::Square) => format!("q:{cover_key}:{pw}x{ph}"),
            (CoverOrigin::Own, CoverFamily::Wide) => format!("w:{cover_key}:{pw}x{ph}"),
            (CoverOrigin::Background, _) => format!("{background}:{key}:{pw}x{ph}"),
            _ => format!("c:{cover_key}:{pw}x{ph}"),
        }
        .into();
        let src = self
            .images
            .get_or_request(&id, || art::pick_family(family, own, stage, tall).0, pw, ph, Variant::Cover, priority)
            .map(ImageSource::Render);
        let failed = src.is_none() && self.images.is_failed(&id);
        FamilyPick { src, failed }
    }
}
