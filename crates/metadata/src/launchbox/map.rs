//! A LaunchBox game as a candidate's record and as a search hit: the
//! fields it fills, its score, its game modes, which box art region is
//! shown and which cover family its shape fills; and its pictures as the
//! Covers dialog offers them.

use heronite_data::cover_family::CoverFamily;
use heronite_data::models::CollectionGameMetadata;
use heronite_data::num::JsNum;
use heronite_i18n::Lang;

use super::index::{ImageKind, LbGame, LbImage};
use super::SOURCE;
use crate::picture_size::family_of;
use crate::platforms::{self, Database};
use crate::types::{release_year, ArtKind, RegionImage, SourceHit, WebImage};
use crate::util::{blank_metadata, encode_uri_component, unique_names12};

/// The fewest votes a community rating needs to be a score: two thirds of
/// LaunchBox's games have fewer, and below five one voter is the rating.
pub const MIN_VOTES: u32 = 5;

/// The "Box art region" option's automatic value.
pub const AUTO_REGION: &str = "auto";

/// LaunchBox's regions people pick from, as the option's values.
pub const REGIONS: &[&str] =
    &["North America", "Europe", "Japan", "World", "United Kingdom", "Germany", "France", "Spain", "Italy", "Brazil", "Korea", "Australia"];

/// The wider region a country's box art stands in for.
fn parent(region: &str) -> Option<&'static str> {
    Some(match region {
        "United States" | "Canada" => "North America",
        "United Kingdom" | "Germany" | "France" | "Spain" | "Italy" | "The Netherlands" | "Sweden" | "Finland" | "Norway" | "Greece"
        | "Russia" => "Europe",
        "Japan" | "Korea" | "China" | "Hong Kong" => "Asia",
        "Australia" => "Oceania",
        "Brazil" => "South America",
        _ => return None,
    })
}

/// The region of the interface language's main country, as `age_rating`
/// picks a board from it.
fn region_of(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "North America",
        Lang::PtBr => "Brazil",
        Lang::Es => "Spain",
        Lang::De => "Germany",
    }
}

/// The regions box art is taken from, best first: the one chosen (or the
/// interface language's), the wider region it belongs to, then art made
/// for everyone (`World`, no region), then the big releases; any other
/// region after them.
pub fn region_order(preference: &str, lang: Lang) -> Vec<String> {
    let first = if preference == AUTO_REGION || preference.is_empty() { region_of(lang) } else { preference };
    let mut out: Vec<String> = Vec::new();
    let mut push = |r: &str| {
        if !out.iter().any(|o| o == r) {
            out.push(r.to_owned());
        }
    };
    push(first);
    if first == "North America" {
        push("United States");
        push("Canada");
    }
    if let Some(p) = parent(first) {
        push(p);
    }
    for r in ["World", "", "North America", "United States", "Europe", "Japan"] {
        push(r);
    }
    out
}

/// The picture of `kinds` (the first kind before the next) in the best
/// region of `order` (any region after those).
fn pick<'a>(images: &'a [LbImage], kinds: &[ImageKind], order: &[String]) -> Option<&'a LbImage> {
    let rank = |img: &LbImage| order.iter().position(|r| *r == img.region).unwrap_or(order.len());
    kinds.iter().find_map(|kind| images.iter().filter(|i| i.kind == *kind).min_by_key(|i| (rank(i), i.file.as_str())))
}

/// The box art of `game` in the best region of `order` (a box made up
/// from scans when no region has a real one).
pub fn box_front<'a>(game: &'a LbGame, order: &[String]) -> Option<&'a LbImage> {
    pick(&game.images, &[ImageKind::BoxFront, ImageKind::BoxFrontReconstructed], order)
}

/// LaunchBox's picture address of `file`.
pub(crate) fn image_url(base: &str, file: &str) -> String {
    let path: Vec<String> = file.split('/').map(encode_uri_component).collect();
    format!("{}/{}", base.trim_end_matches('/'), path.join("/"))
}

/// A 0..=5 community rating as a 0..=100 score, when enough people voted.
pub fn score(rating: Option<f64>, votes: u32) -> Option<JsNum> {
    let r = rating.filter(|r| (0.0..=5.0).contains(r) && *r > 0.0)?;
    (votes >= MIN_VOTES).then(|| JsNum((r * 20.0).round()))
}

/// The game modes LaunchBox's maximum of players and co-op flag say, as
/// IGDB names them. Nearly every game can be played alone; LaunchBox only
/// says the most.
pub fn game_modes(max_players: Option<u32>, cooperative: bool) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(n) = max_players.filter(|n| *n > 0) {
        out.push("Single player".to_owned());
        if n > 1 {
            out.push("Multiplayer".to_owned());
        }
    }
    if cooperative {
        out.push("Co-operative".to_owned());
    }
    out
}

/// `Action; Platform` as a list.
fn split(list: Option<&str>) -> Vec<String> {
    unique_names12(list.unwrap_or("").split(';').map(|s| Some(s.trim())))
}

/// How the Collection names a LaunchBox platform.
pub fn platform_name(lb: &str) -> String {
    platforms::from_database(Database::LaunchBox, lb).map_or_else(|| lb.to_owned(), |p| p.name.to_owned())
}

/// What the source fills for `game`, with box art of `regions`' order.
/// The box art is the tall cover (as it always was, whatever its shape)
/// and, when `box_family` (the family its size says, when read) is the
/// square or the wide one, that family's cover too: a CD case or a
/// landscape box is the game's real square or wide cover, ahead of the
/// square art LaunchBox has for some games.
pub fn record(
    app_name: &str,
    runner: &str,
    game: &LbGame,
    regions: &[String],
    images_base: &str,
    box_family: Option<CoverFamily>,
) -> CollectionGameMetadata {
    let mut md = blank_metadata(app_name, runner);
    let d = &game.detail;
    md.title = Some(game.name.clone());
    md.description = d.overview.clone();
    md.release_date = game.released.clone();
    md.developers = split(d.developer.as_deref());
    md.publishers = split(d.publisher.as_deref());
    md.genres = split(d.genres.as_deref());
    md.game_modes = game_modes(d.max_players, d.cooperative);
    md.critic_score = score(d.rating, d.votes);
    md.platforms = vec![platform_name(&game.platform)];
    let url = |img: &LbImage| image_url(images_base, &img.file);
    let front = box_front(game, regions);
    md.cover_url = front.map(url);
    md.hero_url = pick(&game.images, &[ImageKind::Background], regions).map(url);
    let shaped = |family: CoverFamily| front.filter(|_| box_family == Some(family)).map(url);
    md.cover_square_url = shaped(CoverFamily::Square).or_else(|| pick(&game.images, &[ImageKind::Square], regions).map(url));
    md.cover_wide_url = shaped(CoverFamily::Wide);
    md
}

/// The pictures of `game` the Covers dialog offers for a picture of
/// `kind`: every region's box art for a cover (and LaunchBox's square art
/// for the square one), the backgrounds for a background.
pub fn pictures_for(game: &LbGame, kind: ArtKind) -> Vec<&LbImage> {
    let kinds: &[ImageKind] = match kind {
        ArtKind::Hero => &[ImageKind::Background],
        ArtKind::CoverSquare => &[ImageKind::BoxFront, ImageKind::BoxFrontReconstructed, ImageKind::Square],
        ArtKind::Cover | ArtKind::CoverWide => &[ImageKind::BoxFront, ImageKind::BoxFrontReconstructed],
    };
    game.images.iter().filter(|i| kinds.contains(&i.kind)).collect()
}

/// [`pictures_for`] as the dialog lists them, with their sizes from `size`
/// (0 x 0 when not read):
/// for a cover, those of the cover's shape first; then by `regions`' order,
/// real boxes before made-up ones.
pub fn choices(
    game: &LbGame,
    kind: ArtKind,
    regions: &[String],
    images_base: &str,
    size: impl Fn(&LbImage) -> Option<(u32, u32)>,
) -> Vec<RegionImage> {
    let rank = |img: &LbImage| regions.iter().position(|r| *r == img.region).unwrap_or(regions.len());
    let mut out: Vec<(bool, usize, ImageKind, RegionImage)> = pictures_for(game, kind)
        .into_iter()
        .map(|img| {
            let (width, height) = size(img).unwrap_or((0, 0));
            let off_shape = kind.family().is_some_and(|f| width == 0 || family_of(width, height) != f);
            let url = image_url(images_base, &img.file);
            let image = WebImage { thumb_url: url.clone(), image_url: url, width, height };
            (off_shape, rank(img), img.kind, RegionImage { region: img.region.clone(), image })
        })
        .collect();
    out.sort_by(|a, b| (a.0, a.1, a.2 as u8, &a.3.image.image_url).cmp(&(b.0, b.1, b.2 as u8, &b.3.image.image_url)));
    out.into_iter().map(|(.., c)| c).collect()
}

/// `game` as a search hit: its platform shown, its box art as the thumbnail.
pub fn hit(game: &LbGame, regions: &[String], images_base: &str) -> SourceHit {
    SourceHit {
        source: SOURCE.into(),
        id: Some(game.id.to_string()),
        title: Some(game.name.clone()),
        year: release_year(game.released.as_deref()),
        platform: Some(platform_name(&game.platform)),
        thumb_url: box_front(game, regions).map(|i| image_url(images_base, &i.file)),
        matched_by: None,
        alt_title: None,
        thumb_adult: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(kind: ImageKind, region: &str, file: &str) -> LbImage {
        LbImage { kind, region: region.into(), file: file.into() }
    }

    #[test]
    fn regions_follow_the_choice_then_the_language() {
        assert_eq!(region_order("auto", Lang::En)[..4], ["North America", "United States", "Canada", "World"]);
        assert_eq!(region_order("auto", Lang::De)[..4], ["Germany", "Europe", "World", ""]);
        assert_eq!(region_order("auto", Lang::PtBr)[..4], ["Brazil", "South America", "World", ""]);
        assert_eq!(region_order("Japan", Lang::En)[..4], ["Japan", "Asia", "World", ""]);
        let order = region_order("Europe", Lang::En);
        assert_eq!(order.iter().filter(|r| *r == "Europe").count(), 1, "no region twice");
    }

    #[test]
    fn box_art_by_region_then_reconstructed() {
        let images = vec![
            img(ImageKind::BoxFront, "Japan", "jp.jpg"),
            img(ImageKind::BoxFront, "Europe", "eu.jpg"),
            img(ImageKind::BoxFront, "North America", "na.jpg"),
            img(ImageKind::BoxFrontReconstructed, "Germany", "de-rebuilt.jpg"),
        ];
        let first = |pref: &str, lang: Lang| {
            pick(&images, &[ImageKind::BoxFront, ImageKind::BoxFrontReconstructed], &region_order(pref, lang)).unwrap().file.clone()
        };
        assert_eq!(first("auto", Lang::En), "na.jpg");
        assert_eq!(first("auto", Lang::De), "eu.jpg", "Germany's own is only reconstructed: a real European box first");
        assert_eq!(first("Japan", Lang::En), "jp.jpg");
        assert_eq!(first("Korea", Lang::En), "na.jpg", "no Korean or Asian box: the big releases, North America first");
        let only_rebuilt = vec![img(ImageKind::BoxFrontReconstructed, "", "r.jpg")];
        assert_eq!(
            pick(&only_rebuilt, &[ImageKind::BoxFront, ImageKind::BoxFrontReconstructed], &region_order("auto", Lang::En)).unwrap().file,
            "r.jpg"
        );
    }

    #[test]
    fn scores_need_votes() {
        assert_eq!(score(Some(4.69331395348837), 1032), Some(JsNum(94.0)));
        assert_eq!(score(Some(3.0), 4), None, "too few votes");
        assert_eq!(score(Some(3.0), MIN_VOTES), Some(JsNum(60.0)));
        assert_eq!(score(None, 100), None);
        assert_eq!(score(Some(0.0), 100), None, "no rating");
    }

    #[test]
    fn modes_from_players() {
        assert_eq!(game_modes(Some(1), false), ["Single player"]);
        assert_eq!(game_modes(Some(2), true), ["Single player", "Multiplayer", "Co-operative"]);
        assert!(game_modes(None, false).is_empty());
    }

    #[test]
    fn image_addresses() {
        assert_eq!(image_url("https://images.launchbox-app.com/", "0f84.png"), "https://images.launchbox-app.com/0f84.png");
        assert_eq!(image_url("http://x", "Super Mario-01.jpg"), "http://x/Super%20Mario-01.jpg");
    }
}
