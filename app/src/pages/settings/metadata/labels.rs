//! How a source reads on screen: its tile (logo or monogram), its status
//! words, the games it is for, what it gives, and a line about it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::source_prefs::ids;
use heronite_i18n::{tr, trn};
use heronite_metadata::sources::{extra_fields, AppliesTo, SourceStatus};

use crate::dialogs::metadata::field_label as details_field_label;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Side of a source's tile.
pub(super) const TILE: f32 = 32.;

/// A source's name as shown: proper nouns as they are, the others translated.
pub(super) fn name(raw: &str) -> &str {
    tr(raw)
}

/// The store or brand mark of a source that has one in the app's icons.
fn logo(id: &str) -> Option<Icon> {
    Some(match id {
        "steam" | ids::STEAM_TAGS => Icon::BrandSteam,
        "store" => Icon::Storefront,
        ids::WEB_IMAGES => Icon::Language,
        _ => return None,
    })
}

/// Two letters for a source without a logo: the capitals of its name
/// ("SteamGridDB" → "SG"), else its first two letters.
fn monogram(name: &str) -> String {
    let caps: String = name.chars().filter(|c| c.is_uppercase()).take(2).collect();
    if caps.chars().count() == 2 {
        return caps;
    }
    let mut letters = name.chars().filter(|c| c.is_alphanumeric());
    let first = letters.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let second = letters.next().map(|c| c.to_lowercase().to_string()).unwrap_or_default();
    first + &second
}

/// The source's tile: its logo on a faint well, else its monogram in its colour.
pub(super) fn tile(id: &str, name: &str) -> Div {
    let base = div().flex().flex_none().items_center().justify_center().size(px(TILE)).rounded(px(9.));
    match logo(id) {
        Some(ic) => base.bg(theme::ink_a(0.07)).child(icon(ic, px(17.), theme::text_primary())),
        None => {
            let c: Hsla = theme::source_colour(id).into();
            base.bg(Hsla { a: 0.16, ..c })
                .border_1()
                .border_color(Hsla { a: 0.3, ..c })
                .text_size(px(11.))
                .font_weight(theme::W_BOLD)
                .text_color(theme::readable(c))
                .child(monogram(name))
        }
    }
}

pub(super) fn status_word(s: SourceStatus) -> &'static str {
    match s {
        SourceStatus::Off => tr!("Off"),
        SourceStatus::On => tr!("On"),
        SourceStatus::Connected => tr!("Connected"),
        SourceStatus::NeedsKey => tr!("Add a key"),
        SourceStatus::TestFailed => tr!("Could not connect"),
        SourceStatus::NeedsSignIn => tr!("Sign in"),
        SourceStatus::NeedsDownload => tr!("Needs a download"),
        SourceStatus::Unavailable => tr!("Not on this system"),
        SourceStatus::ByHandOnly => tr!("By hand only"),
    }
}

/// The dot before the status: green when it is asked, amber when something
/// is missing, red when it failed, faint when it is not asked.
pub(super) fn status_colour(s: SourceStatus) -> Hsla {
    match s {
        SourceStatus::On | SourceStatus::Connected => theme::success().into(),
        SourceStatus::ByHandOnly => theme::accent().into(),
        SourceStatus::NeedsKey | SourceStatus::NeedsSignIn | SourceStatus::NeedsDownload => theme::warning().into(),
        SourceStatus::TestFailed => theme::danger().into(),
        SourceStatus::Off | SourceStatus::Unavailable => theme::ink_a(0.3),
    }
}

/// Whether the source is on but cannot be asked yet (its switch shows grey).
pub(super) fn waiting(s: SourceStatus) -> bool {
    matches!(s, SourceStatus::NeedsKey | SourceStatus::NeedsSignIn | SourceStatus::NeedsDownload)
}

/// The dot and the words of a status.
pub(super) fn status(s: SourceStatus) -> Div {
    let c = status_colour(s);
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .min_w_0()
        .child(div().flex_none().size(px(7.)).rounded_full().bg(c))
        .child(div().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(status_word(s)))
}

/// A store id of [`AppliesTo::Stores`] as people know it.
fn store_name(id: &str) -> &str {
    match id {
        "legendary" | "epic" => "Epic Games",
        "gog" => "GOG",
        "nile" | "amazon" => "Amazon Games",
        "steam" => "Steam",
        "sideload" => tr!("Added by hand"),
        other => other,
    }
}

/// The games a source is for, in a few words.
pub(super) fn audience(a: &AppliesTo) -> String {
    match a {
        AppliesTo::All => tr!("All games").into(),
        AppliesTo::HasSteamId => tr!("Games found on Steam").into(),
        AppliesTo::Emulated => tr!("Emulated games").into(),
        AppliesTo::Stores(list) => {
            let stores = list.iter().map(|s| store_name(s)).collect::<Vec<_>>().join(", ");
            tr!("Games from {stores}", stores)
        }
        AppliesTo::Platforms(list) => trn!(list.len(), "Emulated games of {n} platform", "Emulated games of {n} platforms"),
    }
}

/// A field a source fills, as its chip says it.
pub(super) fn field_name(field: &str) -> &str {
    match field {
        extra_fields::TIME_TO_BEAT => tr!("Time to beat"),
        extra_fields::STEAM_TAGS => tr!("Steam tags"),
        extra_fields::LINUX_RATING => tr!("Linux rating"),
        other => match details_field_label(other) {
            "" => other,
            label => label,
        },
    }
}

/// What a source gives, in kinds rather than fields ("Text · Scores ·
/// Covers"), for the closed row.
pub(super) fn gives(fields: &[String]) -> String {
    const TEXT: [&str; 10] =
        ["description", "releaseDate", "developers", "publishers", "genres", "themes", "gameModes", "platforms", "series", "features"];
    let has = |f: &str| fields.iter().any(|x| x == f);
    let mut parts: Vec<&str> = Vec::new();
    if TEXT.iter().any(|f| has(f)) {
        parts.push(tr!("Text"));
    }
    for (field, label) in
        [("criticScore", tr!("Scores")), ("ageRatings", tr!("Age ratings")), ("cover", tr!("Covers")), ("hero", tr!("Backgrounds"))]
    {
        if has(field) {
            parts.push(label);
        }
    }
    let known = |f: &str| TEXT.contains(&f) || ["criticScore", "ageRatings", "cover", "hero"].contains(&f);
    parts.extend(fields.iter().filter(|f| !known(f)).map(|f| field_name(f)));
    parts.join(" \u{b7} ")
}

/// A few words on what the source is, shown when its row is open.
pub(super) fn about(id: &str) -> Option<&'static str> {
    Some(match id {
        "store" => tr!("What the store a game comes from says about it, read from this PC."),
        ids::STEAMGRIDDB => tr!("Adds SteamGridDB covers and backgrounds to the image search."),
        ids::WEB_IMAGES => tr!("Asked only when you look for a picture by hand."),
        ids::HOW_LONG_TO_BEAT => {
            tr!("How long a game takes to beat, from howlongtobeat.com. Heronite is not affiliated with HowLongToBeat.")
        }
        ids::STEAM_TAGS => tr!(
            "The tags Steam players voted for, for games on Steam (other stores' games too, once their metadata found them on Steam). Filter by them in the View menu; rename or remove one in Library fields. From the Steam Web API; Heronite is not affiliated with Valve."
        ),
        ids::PROTONDB => tr!("How well a game runs on Linux, from protondb.com."),
        _ => return None,
    })
}

/// When the source is asked, for the sources the user cannot tell otherwise.
pub(super) fn when_asked(id: &str) -> Option<&'static str> {
    Some(match id {
        ids::HOW_LONG_TO_BEAT => tr!("Asks HowLongToBeat about the game you select, then again after a month."),
        ids::STEAM_TAGS => tr!("Asks Steam for the tags of your games now and then."),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{gives, monogram};

    #[test]
    fn monograms_take_the_capitals_else_the_first_letters() {
        assert_eq!(monogram("SteamGridDB"), "SG");
        assert_eq!(monogram("HowLongToBeat"), "HL");
        assert_eq!(monogram("IGDB"), "IG");
        assert_eq!(monogram("ProtonDB"), "PD");
        assert_eq!(monogram("Lutris"), "Lu");
        assert_eq!(monogram("vndb"), "Vn");
    }

    #[test]
    fn gives_names_kinds_in_a_fixed_order() {
        let f = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(gives(&f(&["cover", "description", "criticScore", "genres"])), "Text \u{b7} Scores \u{b7} Covers");
        assert_eq!(gives(&f(&["cover", "hero"])), "Covers \u{b7} Backgrounds");
        assert_eq!(gives(&f(&["timeToBeat"])), "Time to beat");
    }
}
