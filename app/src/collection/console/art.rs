//! Art console mode uses beyond covers and the wide art: game logos.
//! Built only while console mode is on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;
use heronite_data::console_art::{logo_sources, LogoSource};
use heronite_data::CollectionGame;
use heronite_images::{ImageRef, Variant};

use super::super::CollectionView;
use crate::console::kit;

/// File Steam keeps a game's logo in, inside its library cache folder.
const STEAM_LOGO_FILE: &str = "logo.png";

/// Request priority of a logo: above the covers around it, below the hero art.
const LOGO_PRIORITY: i32 = 150;

/// A game's logo chain and the library entry (`G`) it was built from.
struct Entry<G> {
    game: Arc<G>,
    chain: Vec<ImageRef>,
}

/// Logo sources found per game (filled lazily, dropped with console mode).
pub(crate) struct LogoCache<G = CollectionGame> {
    games: HashMap<SharedString, Entry<G>>,
    /// The home folder (looked up once); `Some(None)` = there is none.
    home: Option<Option<PathBuf>>,
}

impl<G> Default for LogoCache<G> {
    fn default() -> Self {
        Self { games: HashMap::new(), home: None }
    }
}

impl<G> LogoCache<G> {
    /// The chain of `key`: built on the first request and again when the
    /// library replaced the game (its art may have changed). `sources` lists
    /// the game's places given the home folder. The flag says that an older
    /// chain was swapped for a different one, so the textures made from it
    /// are stale.
    fn chain(&mut self, key: &SharedString, game: &Arc<G>, sources: impl FnOnce(Option<&Path>) -> Vec<LogoSource>) -> (&[ImageRef], bool) {
        if self.games.get(key).is_some_and(|e| Arc::ptr_eq(&e.game, game)) {
            return (&self.games[key].chain, false);
        }
        let home = self.home.get_or_insert_with(dirs::home_dir).as_deref();
        let chain = to_chain(&sources(home));
        let old = self.games.insert(key.clone(), Entry { game: game.clone(), chain });
        let entry = &self.games[key];
        (&entry.chain, old.is_some_and(|o| o.chain != entry.chain))
    }
}

/// The loader's chain for logo sources. Steam's cache is searched by the
/// loader's worker (its folders are hashed), never here.
fn to_chain(sources: &[LogoSource]) -> Vec<ImageRef> {
    sources
        .iter()
        .map(|s| match s {
            LogoSource::Url(u) => ImageRef::Url(u.clone()),
            LogoSource::File(p) => ImageRef::File(p.clone()),
            LogoSource::SteamCache(dir) => ImageRef::Find { dir: dir.clone(), name: STEAM_LOGO_FILE.into() },
        })
        .collect()
}

/// Texture id of a logo at a physical size (one texture per size).
fn logo_id(key: &SharedString, pw: u32, ph: u32) -> SharedString {
    format!("l:{key}:{pw}x{ph}").into()
}

impl CollectionView {
    /// The game's logo fitted inside `w`×`h` design px (transparent around
    /// it, never cropped nor enlarged); `None` while it loads or when the
    /// game has none (callers then show the name as text). The image keeps
    /// the logo's aspect ratio, so it is usually smaller than the box: draw
    /// it with `ObjectFit::Contain` in a box of that size, or let the `img`
    /// take its own size.
    pub(super) fn console_logo(&mut self, key: &SharedString, w: f32, h: f32, window: &Window) -> Option<ImageSource> {
        let game = self.lib.as_ref()?.get(key)?.clone();
        let shots = self.lib.as_ref().and_then(|l| Some(&l.settings.screenshots.folder).filter(|f| !f.is_empty())).map(PathBuf::from);
        let paths = &self.data_paths;
        let (chain, stale) = self.console.logos.chain(key, &game, |home| logo_sources(paths, shots.as_deref(), home, &game));
        if stale {
            self.images.invalidate(&[format!("l:{key}:")]);
        }
        if chain.is_empty() {
            return None;
        }
        let px = kit::scale() * window.scale_factor();
        let (pw, ph) = ((w * px).round().max(1.) as u32, (h * px).round().max(1.) as u32);
        let id = logo_id(key, pw, ph);
        self.images.get_or_request(&id, || chain.to_vec(), pw, ph, Variant::Fit, LOGO_PRIORITY).map(ImageSource::Render)
    }
}

#[cfg(test)]
mod tests {
    use super::{logo_id, to_chain, LogoCache};
    use gpui_kit::SharedString;
    use heronite_data::console_art::LogoSource;
    use heronite_images::ImageRef;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// Stands in for a library game: its logo sources are what the test says.
    fn sources(logo: &str) -> impl FnOnce(Option<&Path>) -> Vec<LogoSource> + '_ {
        move |home| {
            let mut s = vec![LogoSource::Url("https://x/mine.png".into()), LogoSource::Url(logo.into())];
            s.extend(home.map(|h| LogoSource::SteamCache(h.join("steam/570"))));
            s
        }
    }

    fn cache(home: Option<&str>) -> LogoCache<()> {
        LogoCache { games: HashMap::new(), home: Some(home.map(PathBuf::from)) }
    }

    #[test]
    fn steam_cache_is_searched_by_the_loader() {
        let chain = to_chain(&[
            LogoSource::Url("https://x/a.png".into()),
            LogoSource::File("/tmp/b.png".into()),
            LogoSource::SteamCache("/s/appcache/librarycache/1".into()),
        ]);
        assert_eq!(
            chain,
            vec![
                ImageRef::Url("https://x/a.png".into()),
                ImageRef::File("/tmp/b.png".into()),
                ImageRef::Find { dir: "/s/appcache/librarycache/1".into(), name: "logo.png".into() },
            ]
        );
    }

    #[test]
    fn chain_is_built_once_and_rebuilt_when_the_game_changes() {
        let key: SharedString = "epic_Abc".into();
        let mut cache = cache(Some("/home/u"));

        let g = Arc::new(());
        let (chain, stale) = cache.chain(&key, &g, sources("https://x/store.png"));
        assert!(!stale, "the first build is not a change");
        assert_eq!(chain[0], ImageRef::Url("https://x/mine.png".into()));
        assert_eq!(chain[1], ImageRef::Url("https://x/store.png".into()));
        assert!(matches!(chain[2], ImageRef::Find { .. }));

        // The same entry: reused, the sources are not even asked.
        let (chain, stale) = cache.chain(&key, &g, |_| panic!("built again"));
        assert_eq!((chain.len(), stale), (3, false));

        // The library replaced the game with identical art: rebuilt, textures stay.
        assert!(!cache.chain(&key, &Arc::new(()), sources("https://x/store.png")).1);

        // New art: the textures made from the old chain are stale.
        let (chain, stale) = cache.chain(&key, &Arc::new(()), sources("https://x/other.png"));
        assert!(stale);
        assert_eq!(chain[1], ImageRef::Url("https://x/other.png".into()));
    }

    #[test]
    fn a_game_without_logo_sources_has_an_empty_chain() {
        let key: SharedString = "epic_Abc".into();
        let mut cache = cache(None);
        assert!(cache.chain(&key, &Arc::new(()), |_| Vec::new()).0.is_empty());
    }

    #[test]
    fn logo_ids_follow_the_size() {
        let key: SharedString = "epic_Abc".into();
        assert_eq!(logo_id(&key, 400, 120), "l:epic_Abc:400x120");
        assert_ne!(logo_id(&key, 400, 120), logo_id(&key, 800, 240));
    }
}
