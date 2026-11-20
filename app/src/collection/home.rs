//! The picture behind the Collection while no game is selected (after Esc, or
//! at start with "Open with the last game selected" off): Settings →
//! Appearance → "With no game selected". A picture of your own, or a game's
//! art or one of your screenshots picked at random, a new one every
//! [`SLIDESHOW_SECS`] and each time the selection is let go.

use std::hash::BuildHasher;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use heronite_data::appearance::{Appearance, BackgroundSource, HomeBackground, SLIDESHOW_SECS};
use heronite_images::ImageRef;

use super::CollectionView;
use crate::theme;

#[derive(Default)]
pub(super) struct HomeState {
    /// The random choice shown at the last render (`None`: a game is
    /// selected, or the choice is not a random one).
    mode: Option<HomeBackground>,
    /// The game whose art shows (`RandomArt`, and `RandomScreenshot` while
    /// there are no screenshots).
    art: Option<SharedString>,
    /// How many games had art when `art` was last looked for.
    art_pool: usize,
    /// Screenshots of every game, listed each time the Home shows.
    shots: Vec<PathBuf>,
    shot: Option<PathBuf>,
    listing: Option<Task<()>>,
    /// Moves the random picture along.
    rotate: Option<Task<()>>,
}

impl CollectionView {
    /// Runs at the start of each render (`sync_look`): picks a random picture
    /// when the Home shows, and keeps one coming.
    pub(super) fn sync_home(&mut self, a: Appearance, cx: &mut Context<Self>) {
        let random = matches!(a.home_background, HomeBackground::RandomArt | HomeBackground::RandomScreenshot);
        let mode = (random && self.selected.is_none() && self.stage_enabled).then_some(a.home_background);
        let entered = mode.is_some() && self.home.mode != mode;
        self.home.mode = mode;
        let Some(mode) = mode else {
            self.home.rotate = None;
            self.home.listing = None;
            return;
        };
        if entered {
            self.home_pick();
            if mode == HomeBackground::RandomScreenshot {
                self.list_home_shots(cx);
            }
        }
        // The library came in (or got its first art) after the Home showed.
        let pool = self.stage_chains.len();
        if self.home.art_pool != pool && !self.home.art.as_ref().is_some_and(|k| self.has_stage_art(k)) {
            self.home.art_pool = pool;
            self.home_pick();
        }
        if self.home.rotate.is_none() {
            self.home.rotate = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(Duration::from_secs(SLIDESHOW_SECS)).await;
                let alive = this
                    .update(cx, |this, cx| {
                        this.home_pick();
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }));
        }
    }

    fn has_stage_art(&self, key: &SharedString) -> bool {
        self.stage_chains.get(key).is_some_and(|c| !c.is_empty())
    }

    /// Another game's art and another screenshot than the ones on screen.
    fn home_pick(&mut self) {
        let keys: Vec<&SharedString> = self.stage_chains.iter().filter(|(_, c)| !c.is_empty()).map(|(k, _)| k).collect();
        self.home.art_pool = self.stage_chains.len();
        self.home.art = pick(&keys, self.home.art.as_ref().as_ref()).map(|k| (*k).clone());
        self.home.shot = pick(&self.home.shots, self.home.shot.as_ref()).cloned();
    }

    /// Lists the screenshots of every game in the background, then picks one.
    fn list_home_shots(&mut self, cx: &mut Context<Self>) {
        let folder = self.lib.as_ref().map(|lib| lib.settings.screenshots.folder.clone()).unwrap_or_default();
        if folder.is_empty() {
            self.home.shots.clear();
            self.home.shot = None;
            return;
        }
        self.home.listing = Some(cx.spawn(async move |this, cx| {
            let shots = cx.background_spawn(async move { heronite_media::list_all_screenshots(std::path::Path::new(&folder)) }).await;
            this.update(cx, |this, cx| {
                this.home.shot = pick(&shots, this.home.shot.as_ref()).cloned();
                this.home.shots = shots;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The picture behind the Collection while no game is selected: an id
    /// for the texture cache and the images to try. `None`: no picture.
    pub(super) fn home_source(&self) -> Option<(String, Vec<ImageRef>)> {
        let a = theme::appearance();
        let file = |id: &str, path: PathBuf| {
            path.exists().then(|| (format!("{id}{}", theme::custom_background_epoch()), vec![ImageRef::File(path)]))
        };
        match a.home_background {
            HomeBackground::Auto if a.background == BackgroundSource::Custom => file("custom", self.app_paths.custom_background()),
            HomeBackground::Auto | HomeBackground::Solid => None,
            HomeBackground::Custom => file("home", self.app_paths.home_background()),
            HomeBackground::RandomScreenshot if self.home.shot.is_some() => {
                let path = self.home.shot.clone()?;
                Some((format!("shot:{}", path.display()), vec![ImageRef::File(path)]))
            }
            HomeBackground::RandomArt | HomeBackground::RandomScreenshot => {
                let key = self.home.art.as_ref()?;
                Some((format!("art:{key}"), self.stage_chains.get(key).cloned().unwrap_or_default()))
            }
        }
    }

    /// The stage shows the panels' colour: the background is Solid for the
    /// selected game, or for the Home.
    pub(super) fn stage_is_solid(&self) -> bool {
        let a = theme::appearance();
        match (&self.selected, a.home_background) {
            (Some(_), _) | (None, HomeBackground::Auto) => a.background == BackgroundSource::Solid,
            (None, h) => h == HomeBackground::Solid,
        }
    }
}

/// A random item of `items`, another one than `current` when there are two
/// or more.
fn pick<'a, T: PartialEq>(items: &'a [T], current: Option<&T>) -> Option<&'a T> {
    if items.is_empty() {
        return None;
    }
    // Each `RandomState` is seeded from the system's random source.
    let mut i = (std::collections::hash_map::RandomState::new().hash_one(items.len()) % items.len() as u64) as usize;
    if items.len() > 1 && current == Some(&items[i]) {
        i = (i + 1) % items.len();
    }
    items.get(i)
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn pick_avoids_the_current_one() {
        let items = [1, 2];
        for _ in 0..50 {
            assert_eq!(pick(&items, Some(&1)), Some(&2));
        }
        assert_eq!(pick(&[7], Some(&7)), Some(&7));
        assert_eq!(pick::<i32>(&[], None), None);
    }
}
