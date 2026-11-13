//! Made-up searches for `HERONITE_DEBUG_DIALOG=covers[:<state>]`, on the
//! selected game: nothing is asked or written (a picture used, a file, a
//! reset or a family kept change only what the dialog shows). The pictures
//! are ones the Collection already has, so they come from the image cache.
//!
//! States: none (the family the desktop draws), `tall`, `square`, `wide`,
//! `background` (that slot picked), `none` (nothing found), `busy` (still
//! searching), `saving` (a picture picked and being downloaded),
//! `error` (the download failed).

use heronite_metadata::{ArtImages, RegionImage, WebImage};

use super::slots::Slot;
use super::{Busy, CoversDialog};

pub(super) struct Fake {
    state: String,
    thumbs: Vec<String>,
}

impl Fake {
    pub(super) fn new(state: &str, thumbs: Vec<String>) -> Self {
        Self { state: state.to_owned(), thumbs }
    }

    /// The slot the state opens on (`None`: the desktop's family).
    pub(super) fn slot(&self) -> Option<Slot> {
        Slot::parse(&self.state)
    }

    /// What the made-up sources found for `slot`: the game's boxes of three
    /// regions from LaunchBox, a few from SteamGridDB and more from the
    /// web, in the slot's sizes.
    pub(super) fn results(&self, slot: Slot) -> ArtImages {
        if self.state == "none" || self.thumbs.is_empty() {
            return ArtImages::default();
        }
        let (w, h) = match slot {
            Slot::Cover(heronite_data::cover_family::CoverFamily::Tall) => (600, 900),
            Slot::Cover(heronite_data::cover_family::CoverFamily::Square) => (1024, 1024),
            Slot::Cover(heronite_data::cover_family::CoverFamily::Wide) => (920, 430),
            Slot::Background => (1920, 620),
        };
        let image = |i: usize, big: bool| {
            let url = self.thumbs[i % self.thumbs.len()].clone();
            let (w, h) = if big { (w, h) } else { (w / 2, h / 2) };
            WebImage { image_url: url.clone(), thumb_url: url, width: w, height: h }
        };
        let launchbox = ["North America", "Europe", "Japan"]
            .iter()
            .enumerate()
            .map(|(i, region)| RegionImage { region: region.to_string(), image: image(22 + i, true) })
            .collect();
        ArtImages { launchbox, steamgrid: (0..4).map(|i| image(i, true)).collect(), web: (4..22).map(|i| image(i, i % 3 != 0)).collect() }
    }
}

impl CoversDialog {
    /// The states that need more than the search (see the module docs).
    pub(super) fn debug_state(&mut self) {
        let Some(state) = self.fake.as_ref().map(|f| f.state.clone()) else { return };
        match state.as_str() {
            "busy" => self.loading = true,
            "saving" => {
                self.selected = Some(1);
                self.busy = Some(Busy::Saving);
            }
            "error" => {
                self.selected = Some(1);
                self.error = "Could not download the picture: the server answered 404 Not Found.".into();
            }
            _ => {}
        }
    }
}
