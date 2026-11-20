//! Image candidate chains: port of `Collection/steamArt.ts`
//! (`collectionCoverSources`, `collectionStageArt`).
//!
//! The TS hands the image component a `src` and a list of fallbacks; here the
//! chain is one list, tried in order until an image loads. Local sources
//! (`localart://`, `file:`, absolute paths) that do not exist on disk are left
//! out of the chain (Heronite's `CachedImage` skips them when they fail).

use std::path::{Path, PathBuf};

use crate::cover_family::CoverFamily;
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

/// Steam's CDN logos of an app (the console themes' title logo), newest scheme first.
pub fn steam_logo_urls(steam_app_id: &str) -> [String; 2] {
    [
        format!("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{steam_app_id}/logo.png"),
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/logo.png"),
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
        value.starts_with("file:") || value.starts_with("localart:") || value.starts_with('/') || Path::new(value).is_absolute()
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
/// - Steam `library_hero_2x.jpg`, then `library_hero.jpg`, on both CDNs
///   (needs `meta.steamAppId`; older games only have the 1x one)
/// - Heroic override `art_cover`
/// - `art_cover`
/// - `art_square`
///
/// The Steam hero (1920 or 3840 px wide, made to be a background) comes
/// before the covers: a game's `art_cover` is often Steam's `header.jpg`
/// (460×215), which a full window would show stretched five times.
///
/// The last ones skip `localart:` values. Empty means no stage.
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

    let background = [present(info.art_background.as_deref())];
    // The hero URLs; the last download URL is Steam's generated page
    // background, which is darkened and blurred: not a hero.
    let steam_id = game.steam_app_id.as_deref().filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()));
    let steam_hero = steam_id.map(steam_hero_download_urls);
    let steam_hero = steam_hero.iter().flat_map(|urls| urls[..4].iter().map(|u| Some(u.as_str())));
    let covers = [
        present(game.overrides.as_ref().and_then(|o| o.art_cover.as_deref())),
        present(info.art_cover.as_deref()),
        present(info.art_square.as_deref()),
    ];
    for value in background.into_iter().chain(steam_hero).chain(covers).flatten().filter(|v| !is_managed_art(v)) {
        if let Some(item) = ctx.resolve(value) {
            push_unique(&mut chain, item);
        }
    }
    chain
}

/// Where the picture a view draws for a cover family comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverOrigin {
    /// The game's own picture of that family (for the tall family: its cover
    /// chain, whatever it holds).
    Own,
    /// The game's background, cropped (a wide view without a wide cover:
    /// what the Banner aspect always showed).
    Background,
    /// The tall cover, cropped.
    Tall,
}

impl CoverOrigin {
    /// Not a picture of the family: shown cropped from another one.
    pub fn is_cropped(self) -> bool {
        self != Self::Own
    }
}

/// The game's own pictures of `family`: the one set by hand (`art.json`),
/// then the fetched one (`metadata.json`); missing local files left out.
/// Empty for [`CoverFamily::Tall`], whose pictures are [`card_cover`].
pub fn family_art(ctx: &ArtContext<'_>, game: &CollectionGame, family: CoverFamily) -> Vec<ImageRef> {
    let (custom, fetched) = match family {
        CoverFamily::Tall => return Vec::new(),
        CoverFamily::Square => (
            game.art.as_deref().and_then(|a| a.cover_square_url.as_deref()),
            game.metadata.as_deref().and_then(|m| m.cover_square_url.as_deref()),
        ),
        CoverFamily::Wide => (
            game.art.as_deref().and_then(|a| a.cover_wide_url.as_deref()),
            game.metadata.as_deref().and_then(|m| m.cover_wide_url.as_deref()),
        ),
    };
    let mut chain = Vec::new();
    for value in [custom, fetched].into_iter().flatten().filter(|v| !v.is_empty()) {
        let item = if ArtContext::is_local(value) { ctx.resolve(value) } else { Some(format_remote(value, &game.runner)) };
        if let Some(item) = item.filter(|i| *i != ImageRef::Fallback) {
            push_unique(&mut chain, item);
        }
    }
    chain
}

/// What a view of `family` draws, from the parts already resolved: `own`
/// ([`family_art`]), `stage` ([`stage_art`]) and `tall` ([`card_cover`]).
/// Own art goes first, with the rest after it in case it fails to load;
/// without it a wide view takes the background (when there is one) and
/// every other view the tall cover, cropped. The tall family is `tall`.
///
/// Generic over the chain's item so the app can pass the chains it keeps.
pub fn pick_family<T: Clone>(family: CoverFamily, own: &[T], stage: &[T], tall: &[T]) -> (Vec<T>, CoverOrigin) {
    let origin = family_origin(family, !own.is_empty(), !stage.is_empty());
    let behind = if family == CoverFamily::Wide && !stage.is_empty() { stage } else { tall };
    let chain = match origin {
        CoverOrigin::Own if family == CoverFamily::Tall => tall.to_vec(),
        CoverOrigin::Own => own.iter().chain(behind).cloned().collect(),
        CoverOrigin::Background => stage.to_vec(),
        CoverOrigin::Tall => tall.to_vec(),
    };
    (chain, origin)
}

/// Where [`pick_family`] takes the picture from, without building the
/// chain: `has_own` the game has art of the family, `has_stage` a
/// background.
pub fn family_origin(family: CoverFamily, has_own: bool, has_stage: bool) -> CoverOrigin {
    match family {
        CoverFamily::Tall => CoverOrigin::Own,
        _ if has_own => CoverOrigin::Own,
        CoverFamily::Wide if has_stage => CoverOrigin::Background,
        _ => CoverOrigin::Tall,
    }
}

/// The picture a view of `family` draws for `game`, and where it comes
/// from (see [`pick_family`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyCover {
    pub chain: Vec<ImageRef>,
    pub origin: CoverOrigin,
}

/// [`pick_family`] for one game: every cover request goes through it (the
/// app through the parts it keeps per game).
pub fn family_cover(ctx: &ArtContext<'_>, game: &CollectionGame, family: CoverFamily) -> FamilyCover {
    let own = family_art(ctx, game, family);
    let stage = if family == CoverFamily::Wide { stage_art(ctx, game) } else { Vec::new() };
    let (chain, origin) = pick_family(family, &own, &stage, &card_cover(ctx, game));
    FamilyCover { chain, origin }
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
                // The Steam hero before the covers (#275), 1x when there is no 2x.
                ImageRef::Url(steam_library_hero("42")),
                ImageRef::Url("https://cdn.cloudflare.steamstatic.com/steam/apps/42/library_hero.jpg".into()),
                ImageRef::Url("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/42/library_hero_2x.jpg".into()),
                ImageRef::Url("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/42/library_hero.jpg".into()),
                ImageRef::Url("https://cdn/cover.jpg".into()),
                ImageRef::Url("https://cdn/square.jpg".into()),
            ]
        );
        let bare = game_with("y", "sideload", |_, _, _, _, _| {});
        assert!(stage_art(&ctx, &bare).is_empty());
    }

    /// #275: a game whose covers came from Steam (`art_cover` is the 460x215
    /// `header.jpg`) and that has no hero yet shows the Steam hero, not the
    /// header stretched over the window.
    #[test]
    fn stage_prefers_the_steam_hero_to_the_header() {
        let (_d, paths) = setup();
        let ctx = ArtContext::new(&paths, None);
        let g = game_with("x", "sideload", |info, _, _, meta, overrides| {
            info.art_cover = Some(steam_header_image("357190"));
            info.art_square = Some(steam_library_cover("357190"));
            overrides.art_cover = Some("https://cdn/override".into());
            meta.steam_app_id = Some("357190".into());
        });
        let heroes = steam_hero_download_urls("357190");
        let mut expected: Vec<ImageRef> = heroes[..4].iter().map(|u| ImageRef::Url(u.clone())).collect();
        expected.extend([
            ImageRef::Url("https://cdn/override".into()),
            ImageRef::Url(steam_header_image("357190")),
            ImageRef::Url(steam_library_cover("357190")),
        ]);
        assert_eq!(stage_art(&ctx, &g), expected);
        // No usable Steam id: the covers only.
        for id in ["", "../x"] {
            let g = game_with("x", "sideload", |info, _, _, meta, _| {
                info.art_cover = Some(steam_header_image("357190"));
                meta.steam_app_id = Some(id.into());
            });
            assert_eq!(stage_art(&ctx, &g), vec![ImageRef::Url(steam_header_image("357190"))], "{id:?}");
        }
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
    fn each_family_takes_its_own_art_then_falls_back() {
        let (_d, paths) = setup();
        fs::write(paths.art_dir().join("sideload_x/cover-square-1.png"), b"x").unwrap();
        let ctx = ArtContext::new(&paths, None);
        let tall_only = game_with("x", "sideload", |info, _, metadata, _, _| {
            info.art_background = Some("https://cdn/bg.jpg".into());
            metadata.cover_url = Some("https://cdn/tall.jpg".into());
        });
        let tall = card_cover(&ctx, &tall_only);
        // Only a tall cover: every family draws today's pictures.
        assert_eq!(family_cover(&ctx, &tall_only, CoverFamily::Tall), FamilyCover { chain: tall.clone(), origin: CoverOrigin::Own });
        assert_eq!(family_cover(&ctx, &tall_only, CoverFamily::Square), FamilyCover { chain: tall.clone(), origin: CoverOrigin::Tall });
        let wide = family_cover(&ctx, &tall_only, CoverFamily::Wide);
        assert_eq!((wide.chain, wide.origin), (stage_art(&ctx, &tall_only), CoverOrigin::Background));
        let bare = game_with("y", "sideload", |_, _, metadata, _, _| metadata.cover_url = Some("https://cdn/t.jpg".into()));
        let wide = family_cover(&ctx, &bare, CoverFamily::Wide);
        assert_eq!((wide.chain, wide.origin), (card_cover(&ctx, &bare), CoverOrigin::Tall), "no background: the tall cover");
        assert!(CoverOrigin::Tall.is_cropped() && CoverOrigin::Background.is_cropped() && !CoverOrigin::Own.is_cropped());

        // Custom first, then fetched, then what the view had before.
        let own = game_with("x", "legendary", |info, art, metadata, _, _| {
            info.art_background = Some("https://cdn/bg.jpg".into());
            metadata.cover_url = Some("https://cdn/tall.jpg".into());
            art.cover_square_url = Some("localart://art/sideload_x/cover-square-1.png".into());
            metadata.cover_square_url = Some("https://cdn/sq.jpg".into());
            metadata.cover_wide_url = Some("https://cdn/wide.jpg".into());
        });
        let square = family_cover(&ctx, &own, CoverFamily::Square);
        assert_eq!(square.origin, CoverOrigin::Own);
        assert_eq!(
            square.chain[..2],
            [
                ImageRef::File(paths.art_dir().join("sideload_x/cover-square-1.png")),
                ImageRef::Url("https://cdn/sq.jpg?h=400&resize=1&w=300".into())
            ]
        );
        assert_eq!(square.chain[2..], card_cover(&ctx, &own)[..]);
        let wide = family_cover(&ctx, &own, CoverFamily::Wide);
        assert_eq!(wide.origin, CoverOrigin::Own);
        assert_eq!(wide.chain[0], ImageRef::Url("https://cdn/wide.jpg?h=400&resize=1&w=300".into()));
        assert_eq!(wide.chain[1..], stage_art(&ctx, &own)[..]);
        // A custom file that is gone is not a picture of the family.
        let gone = game_with("z", "sideload", |_, art, _, _, _| art.cover_square_url = Some("localart://art/z/none.png".into()));
        assert_eq!(family_cover(&ctx, &gone, CoverFamily::Square).origin, CoverOrigin::Tall);
    }

    #[test]
    fn pick_family_order() {
        let (own, stage, tall) = (["o"], ["s"], ["t"]);
        assert_eq!(pick_family(CoverFamily::Tall, &own, &stage, &tall), (vec!["t"], CoverOrigin::Own));
        assert_eq!(pick_family(CoverFamily::Square, &own, &stage, &tall), (vec!["o", "t"], CoverOrigin::Own));
        assert_eq!(pick_family(CoverFamily::Wide, &own, &stage, &tall), (vec!["o", "s"], CoverOrigin::Own));
        assert_eq!(pick_family::<&str>(CoverFamily::Wide, &[], &stage, &tall), (vec!["s"], CoverOrigin::Background));
        assert_eq!(pick_family::<&str>(CoverFamily::Wide, &[], &[], &tall), (vec!["t"], CoverOrigin::Tall));
        assert_eq!(pick_family::<&str>(CoverFamily::Wide, &own, &[], &tall), (vec!["o", "t"], CoverOrigin::Own));
        assert_eq!(pick_family::<&str>(CoverFamily::Square, &[], &stage, &tall), (vec!["t"], CoverOrigin::Tall));
    }

    #[test]
    fn resolving_every_family_of_two_thousand_games_is_fast() {
        let (_d, paths) = setup();
        let ctx = ArtContext::new(&paths, None);
        let games: Vec<_> = (0..2000)
            .map(|i| {
                game_with(&format!("g{i}"), "sideload", |info, _, metadata, meta, _| {
                    info.art_square = Some(format!("https://cdn/{i}/square.jpg"));
                    metadata.cover_url = Some(format!("https://cdn/{i}/c.jpg"));
                    if i % 3 == 0 {
                        metadata.cover_square_url = Some(format!("https://cdn/{i}/sq.jpg"));
                    }
                    meta.steam_app_id = Some(i.to_string());
                })
            })
            .collect();
        let start = std::time::Instant::now();
        let mut cropped = 0;
        for g in &games {
            for family in CoverFamily::ALL {
                cropped += usize::from(family_cover(&ctx, g, family).origin.is_cropped());
            }
        }
        let took = start.elapsed();
        assert_eq!(cropped, 2000 + 1333, "every wide one, two square ones in three");
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
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
