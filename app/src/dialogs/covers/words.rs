//! How cover families read on screen, shared by the Covers dialog, Game
//! settings, the one-game Metadata dialog, Settings → Metadata, the dock's
//! Metadata menu, "Update metadata" and the aspect notice. One sentence per
//! family: word order differs between languages, so nothing is built from
//! a family's name and a fragment.

use heronite_data::cover_family::{CoverFamily, FamilyState, FamilyUser};
use heronite_i18n::{tr, trc, trn};

/// "Tall", "Square", "Wide".
pub fn family_name(f: CoverFamily) -> &'static str {
    match f {
        CoverFamily::Tall => trc!("shape", "Tall"),
        CoverFamily::Square => trc!("shape", "Square"),
        CoverFamily::Wide => trc!("shape", "Wide"),
    }
}

/// The aspects a family serves.
pub fn aspects(f: CoverFamily) -> &'static str {
    match f {
        CoverFamily::Tall => tr!("2:3, 3:4, 11:15 and 5:7 share one picture"),
        CoverFamily::Square => "1:1",
        CoverFamily::Wide => "16:9",
    }
}

/// "Download square covers".
pub fn download(f: CoverFamily) -> &'static str {
    match f {
        CoverFamily::Tall => tr!("Download tall covers"),
        CoverFamily::Square => tr!("Download square covers"),
        CoverFamily::Wide => tr!("Download wide covers"),
    }
}

/// Where a family shows, for a line about one picture ("used on the
/// desktop"); `None` when no view draws it.
pub fn used_where(state: &FamilyState) -> Option<&'static str> {
    let desktop = state.used_by.iter().any(|u| matches!(u, FamilyUser::DesktopCovers | FamilyUser::DesktopDetails));
    let console = state.used_by.iter().any(|u| matches!(u, FamilyUser::Console { .. }));
    match (desktop, console) {
        (true, true) => Some(tr!("used on the desktop and in Console mode")),
        (true, false) => Some(tr!("used on the desktop")),
        (false, true) => Some(tr!("used in Console mode")),
        (false, false) => None,
    }
}

/// Who draws a family, for Settings → Metadata ("Grid and Focus covers ·
/// Console mode (Pulse)").
pub fn users(state: &FamilyState) -> String {
    if state.used_by.is_empty() {
        return tr!("No view uses it").to_string();
    }
    state
        .used_by
        .iter()
        .map(|u| match u {
            FamilyUser::DesktopCovers => tr!("Grid and Focus covers").to_string(),
            FamilyUser::DesktopDetails => tr!("Game page and lists").to_string(),
            FamilyUser::Console { theme } => tr!("Console mode ({theme})", theme = heronite_i18n::tr(theme)),
        })
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// The aspect notice before a download: "238 games have no square cover…".
pub fn lacking(f: CoverFamily, n: usize) -> String {
    match f {
        CoverFamily::Wide => trn!(
            n,
            "{n} game has no wide cover. It shows its background or tall cover cropped until one is downloaded.",
            "{n} games have no wide cover. They show their background or tall cover cropped until one is downloaded."
        ),
        _ => trn!(
            n,
            "{n} game has no square cover. It shows its tall cover cropped until one is downloaded.",
            "{n} games have no square cover. They show their tall cover cropped until one is downloaded."
        ),
    }
}

/// The aspect notice once the download runs.
pub fn downloading(f: CoverFamily, n: usize) -> String {
    match f {
        CoverFamily::Wide => trn!(n, "Downloading wide covers for {n} game", "Downloading wide covers for {n} games"),
        _ => trn!(n, "Downloading square covers for {n} game", "Downloading square covers for {n} games"),
    }
}

/// Settings → Metadata, under the families: "238 games show a cropped square cover".
pub fn cropped(f: CoverFamily, n: usize) -> String {
    match f {
        CoverFamily::Wide => trn!(n, "{n} game shows a cropped wide cover", "{n} games show a cropped wide cover"),
        _ => trn!(n, "{n} game shows a cropped square cover", "{n} games show a cropped square cover"),
    }
}

/// No source that gives the family's covers can be asked now.
pub fn no_source(f: CoverFamily) -> &'static str {
    match f {
        CoverFamily::Wide => tr!("No source that gives wide covers is on."),
        _ => tr!("No source that gives square covers is on."),
    }
}
