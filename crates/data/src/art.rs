//! Image candidate chains: port of `Collection/steamArt.ts`
//! (`collectionCoverSources`, `collectionStageArt`).
//!
//! The TS hands the image component a `src` and a list of fallbacks; here the
//! chain is one list, tried in order until an image loads. Local sources
//! (`localart://`, `file:`, absolute paths) that do not exist on disk are left
//! out of the chain (Heronite's `CachedImage` skips them when they fail).

use std::path::{Path, PathBuf};

use crate::library::CollectionGame;
use crate::paths::{percent_decode, DataPaths};

/// One candidate image.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageRef {
    /// A file on disk.
    File(PathBuf),
    /// A remote image.
    Url(String),
    /// The bundled placeholder (`heroic_card.jpg`); always the end of a
    /// cover chain.
    Fallback,
}

impl ImageRef {
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Url(_))
    }
}

pub fn steam_library_cover(steam_app_id: &str) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/library_600x900_2x.jpg")
}

pub fn steam_library_hero(steam_app_id: &str) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/library_hero_2x.jpg")
}

pub fn steam_header_image(steam_app_id: &str) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/header.jpg")
}

/// URLs `cacheSteamHero` tries, in order, to fill `local_library/heroes/{id}.jpg`.
pub fn steam_hero_download_urls(steam_app_id: &str) -> [String; 5] {
    [
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/library_hero_2x.jpg"),
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/library_hero.jpg"),
        format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{steam_app_id}/library_hero_2x.jpg"),
        format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{steam_app_id}/library_hero.jpg"),
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/page_bg_generated_v6b.jpg"),
    ]
}

/// What local files and the screenshots folder resolve against.
#[derive(Debug, Clone, Copy)]
pub struct ArtContext<'a> {
    pub paths: &'a DataPaths,
    /// `settings.screenshots.folder`, for `localart://screenshots/...`.
    pub screenshots_folder: Option<&'a Path>,
}

impl<'a> ArtContext<'a> {
    pub fn new(paths: &'a DataPaths, screenshots_folder: Option<&'a Path>) -> Self {
        Self { paths, screenshots_folder }
    }

    /// `isLocalSrc`: `file:`, `localart:` or an absolute path.
    fn is_local(value: &str) -> bool {
        value.starts_with("file:") || value.starts_with("localart:") || value.starts_with('/')
    }

    /// The file behind a local source, `None` when it cannot be resolved or
    /// does not exist.
    fn local_file(&self, value: &str) -> Option<PathBuf> {
        let path = if value.starts_with("localart:") {
            self.paths.resolve_localart(value, self.screenshots_folder)?
        } else if let Some(rest) = value.strip_prefix("file://") {
            PathBuf::from(percent_decode(rest))
        } else if let Some(rest) = value.strip_prefix("file:") {
            PathBuf::from(percent_decode(rest))
        } else {
            PathBuf::from(value)
        };
        path.exists().then_some(path)
    }

    /// A source of any kind as an [`ImageRef`] (remote URLs verbatim).
    fn resolve(&self, value: &str) -> Option<ImageRef> {
        if Self::is_local(value) {
            self.local_file(value).map(ImageRef::File)
        } else {
            Some(ImageRef::Url(value.to_owned()))
        }
    }
}

fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

fn is_managed_art(value: &str) -> bool {
    value.starts_with("localart:")
}

fn push_unique(chain: &mut Vec<ImageRef>, item: ImageRef) {
    if !chain.contains(&item) {
        chain.push(item);
    }
}

/// `getImageFormatting`: Epic CDN images are requested at card size; the
/// literal `fallback` means the bundled placeholder.
fn format_remote(value: &str, runner: &str) -> ImageRef {
    if value == "fallback" {
        ImageRef::Fallback
    } else if runner == "legendary" {
        ImageRef::Url(format!("{value}?h=400&resize=1&w=300"))
    } else {
        ImageRef::Url(value.to_owned())
    }
}

/// Card / Focus cover chain (`collectionCoverSources`), in order:
///
/// - custom cover (`art.json`)
/// - Collection metadata cover
/// - Heroic override `art_square`
/// - `art_square`
/// - `art_cover`
/// - Steam `library_600x900_2x.jpg` (needs `meta.steamAppId`)
/// - Steam `header.jpg` (needs `meta.steamAppId`)
/// - the bundled placeholder
///
/// Remote Epic images get `?h=400&resize=1&w=300`. Never empty.
pub fn card_cover(ctx: &ArtContext<'_>, game: &CollectionGame) -> Vec<ImageRef> {
    let info = &game.game;
    let steam_cover = game.steam_app_id.as_deref().map(steam_library_cover);
    let steam_header = game.steam_app_id.as_deref().map(steam_header_image);
    let defaults = [
        game.overrides.as_ref().and_then(|o| o.art_square.as_deref()),
        info.art_square.as_deref(),
        info.art_cover.as_deref(),
        steam_cover.as_deref(),
        steam_header.as_deref(),
    ];
    // The Steam store header (a hashed URL from the store's API) when the
    // CDN has no art for newer apps.
    let store_header = game.metadata.as_deref().and_then(|m| m.extra.get("headerUrl")).and_then(|v| v.as_str());
    let candidates =
        [game.art.as_deref().and_then(|a| a.cover_url.as_deref()), game.metadata.as_deref().and_then(|m| m.cover_url.as_deref())]
            .into_iter()
            .chain(defaults.into_iter().filter(|v| v.is_some_and(|v| !v.is_empty() && !is_managed_art(v))))
            .chain([store_header]);

    let mut chain = Vec::new();
    for value in candidates.flatten().filter(|v| !v.is_empty()) {
        let item = if ArtContext::is_local(value) {
            match ctx.resolve(value) {
                Some(item) => item,
                None => continue,
            }
        } else {
            format_remote(value, &game.runner)
        };
        push_unique(&mut chain, item);
    }
    if chain.last() != Some(&ImageRef::Fallback) {
        chain.push(ImageRef::Fallback);
    }
    chain
}

/// Stage / hero chain (`collectionStageArt`, which in the TS picks the first
/// hit and keeps only the Steam hero as fallback; here every step is kept), in
/// order:
///
/// - custom hero (`art.json`)
/// - Collection metadata hero
/// - cached Steam hero (`local_library/heroes/{id}.jpg`)
/// - `art_background`
/// - Heroic override `art_cover`
/// - `art_cover`
/// - Steam `library_hero_2x.jpg` (needs `meta.steamAppId`)
/// - `art_square`
///
/// The last five skip `localart:` values. Empty means no stage.
pub fn stage_art(ctx: &ArtContext<'_>, game: &CollectionGame) -> Vec<ImageRef> {
    let info = &game.game;
    let mut chain = Vec::new();

    let managed = [game.art.as_deref().and_then(|a| a.hero_url.as_deref()), game.metadata.as_deref().and_then(|m| m.hero_url.as_deref())];
    for value in managed.into_iter().flatten().filter(|v| !v.is_empty()) {
        if let Some(item) = ctx.resolve(value) {
            push_unique(&mut chain, item);
        }
    }

    if let Some(id) = game.steam_app_id.as_deref() {
        let cached = ctx.paths.steam_hero(id);
        if cached.exists() {
            push_unique(&mut chain, ImageRef::File(cached));
        }
    }

    let steam_hero = game.steam_app_id.as_deref().map(steam_library_hero);
    let defaults = [
        present(info.art_background.as_deref()),
        present(game.overrides.as_ref().and_then(|o| o.art_cover.as_deref())),
        present(info.art_cover.as_deref()),
        present(steam_hero.as_deref()),
        present(info.art_square.as_deref()),
    ];
    for value in defaults.into_iter().flatten().filter(|v| !is_managed_art(v)) {
        if let Some(item) = ctx.resolve(value) {
            push_unique(&mut chain, item);
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game_with;
    use crate::library::Library;
    use std::fs;

    fn setup() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        fs::create_dir_all(paths.art_dir().join("sideload_x")).unwrap();
        fs::create_dir_all(paths.heroes_dir()).unwrap();
        fs::create_dir_all(paths.metadata_art_dir()).unwrap();
        (dir, paths)
    }

    #[test]
    fn cover_chain_order_and_epic_params() {
        let (_d, paths) = setup();
        fs::write(paths.art_dir().join("sideload_x/cover-1.png"), b"x").unwrap();
        let ctx = ArtContext::new(&paths, None);
        let g = game_with("x", "legendary", |info, art, metadata, meta, overrides| {
            info.art_square = Some("https://cdn/epic-square".into());
            info.art_cover = Some("https://cdn/epic-cover".into());
            art.cover_url = Some("localart://art/sideload_x/cover-1.png".into());
            metadata.cover_url = Some("https://images.igdb.com/c.jpg".into());
            meta.steam_app_id = Some("42".into());
            overrides.art_square = Some("https://cdn/override".into());
        });
        let chain = card_cover(&ctx, &g);
        assert_eq!(
            chain,
            vec![
                ImageRef::File(paths.art_dir().join("sideload_x/cover-1.png")),
                ImageRef::Url("https://images.igdb.com/c.jpg?h=400&resize=1&w=300".into()),
                ImageRef::Url("https://cdn/override?h=400&resize=1&w=300".into()),
                ImageRef::Url("https://cdn/epic-square?h=400&resize=1&w=300".into()),
                ImageRef::Url("https://cdn/epic-cover?h=400&resize=1&w=300".into()),
                ImageRef::Url(steam_library_cover("42") + "?h=400&resize=1&w=300"),
                ImageRef::Url(steam_header_image("42") + "?h=400&resize=1&w=300"),
                ImageRef::Fallback,
            ]
        );
    }

    #[test]
    fn missing_custom_art_and_managed_defaults_are_dropped() {
        let (_d, paths) = setup();
        let ctx = ArtContext::new(&paths, None);
        let g = game_with("x", "sideload", |info, art, _md, _meta, _o| {
            art.cover_url = Some("localart://art/sideload_x/gone.png".into());
            info.art_square = Some("localart://art/other.png".into());
            info.art_cover = Some("https://cdn/c.jpg".into());
        });
        assert_eq!(card_cover(&ctx, &g), vec![ImageRef::Url("https://cdn/c.jpg".into()), ImageRef::Fallback]);
        let bare = game_with("y", "sideload", |_, _, _, _, _| {});
        assert_eq!(card_cover(&ctx, &bare), vec![ImageRef::Fallback]);
    }

    #[test]
    fn stage_chain_prefers_custom_then_metadata_then_cached_hero() {
        let (_d, paths) = setup();
        fs::write(paths.art_dir().join("sideload_x/hero-1.png"), b"x").unwrap();
        fs::write(paths.metadata_art_dir().join("h.jpg"), b"x").unwrap();
        fs::write(paths.heroes_dir().join("42.jpg"), b"x").unwrap();
        let ctx = ArtContext::new(&paths, None);
        let g = game_with("x", "legendary", |info, art, metadata, meta, _o| {
            info.art_background = Some("https://cdn/bg.jpg".into());
            info.art_cover = Some("https://cdn/cover.jpg".into());
            info.art_square = Some("https://cdn/square.jpg".into());
            art.hero_url = Some("localart://art/sideload_x/hero-1.png".into());
            metadata.hero_url = Some("localart://metadata-art/h.jpg".into());
            meta.steam_app_id = Some("42".into());
        });
        assert_eq!(
            stage_art(&ctx, &g),
            vec![
                ImageRef::File(paths.art_dir().join("sideload_x/hero-1.png")),
                ImageRef::File(paths.metadata_art_dir().join("h.jpg")),
                ImageRef::File(paths.heroes_dir().join("42.jpg")),
                // no ?h=400 on the stage, even for Epic
                ImageRef::Url("https://cdn/bg.jpg".into()),
                ImageRef::Url("https://cdn/cover.jpg".into()),
                ImageRef::Url(steam_library_hero("42")),
                ImageRef::Url("https://cdn/square.jpg".into()),
            ]
        );
        let bare = game_with("y", "sideload", |_, _, _, _, _| {});
        assert!(stage_art(&ctx, &bare).is_empty());
    }

    #[test]
    fn absolute_paths_and_file_urls_are_local() {
        let (dir, paths) = setup();
        let img = dir.path().join("my image.png");
        fs::write(&img, b"x").unwrap();
        let ctx = ArtContext::new(&paths, None);
        let g = game_with("x", "sideload", |info, _, _, _, _| {
            info.art_square = Some(img.to_string_lossy().into_owned());
            info.art_cover = Some(format!("file://{}", img.to_string_lossy().replace(' ', "%20")));
        });
        assert_eq!(card_cover(&ctx, &g), vec![ImageRef::File(img), ImageRef::Fallback]);
    }

    #[test]
    fn real_covers_resolve() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE") else { return };
        let paths = DataPaths::at(root);
        let lib = Library::load(&paths).unwrap();
        let ctx = ArtContext::new(&paths, lib.settings.screenshots_folder());
        let mut with_custom_hero = 0;
        for game in &lib.games {
            let cover = card_cover(&ctx, game);
            assert_eq!(cover.last(), Some(&ImageRef::Fallback));
            assert!(cover.len() >= 2 || game.game.art_cover.is_none());
            if stage_art(&ctx, game).first().is_some_and(|i| matches!(i, ImageRef::File(_))) {
                with_custom_hero += 1;
            }
        }
        eprintln!("games whose stage starts with a local file: {with_custom_hero}");
        let epic = lib.games.iter().find(|g| g.runner == "legendary").unwrap();
        assert!(card_cover(&ctx, epic).iter().any(|i| matches!(i, ImageRef::Url(u) if u.ends_with("?h=400&resize=1&w=300"))));
    }
}
