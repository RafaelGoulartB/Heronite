//! Builds the Collection view models from `heronite-data`.

use std::sync::Arc;

use gpui_kit::{rgb, Rgba, SharedString};
use heronite_data::appearance::CoverLabel;
use heronite_data::collection::Game;
use heronite_data::collection::{self, FacetKind, RelatedKind};
use heronite_data::focus::focus_data;
use heronite_data::hltb::WikiCache;
use heronite_data::organize::{self, Organize};
use heronite_data::status::display_status;
use heronite_data::text::{format_timestamp, DateLocale};
use heronite_data::ui_state::UiState;
use heronite_data::{CollectionGame, GroupBy, Library, SortMode, ViewQuery};

use super::vm::*;
use crate::icons::Icon;
use crate::theme;
use heronite_i18n::{tr, trn};

fn parse_hex(hex: &str) -> Rgba {
    let h = hex.trim_start_matches('#');
    u32::from_str_radix(h, 16).map(rgb).unwrap_or(rgb(0xe5e7eb))
}

/// A card whose caption is `caption` instead of the playtime.
fn card_vm_with(g: &CollectionGame, playing: bool, caption: Option<Caption>) -> Arc<CardVm> {
    Arc::new(CardVm {
        key: g.key.clone().into(),
        title: g.title.clone().into(),
        caption: caption.unwrap_or_else(|| Caption {
            icon: Icon::AccessTime,
            text: collection::playtime_label(g.playtime_minutes).into(),
            muted: false,
            stars: None,
            tint: None,
        }),
        installed: g.installed,
        playing,
        hidden: g.hidden,
        favourite: g.favourite,
        cover_key: g.key.clone().into(),
    })
}

/// The line under a card's cover: the value the list is sorted by, so the
/// order explains itself (the playtime for title, last played and playtime).
fn caption(g: &CollectionGame, sort: SortMode, o: &Organize, locale: DateLocale) -> Option<Caption> {
    let shown = |icon, text: String| Caption { icon, text: text.into(), muted: false, stars: None, tint: None };
    let missing = |icon, text: &str| Caption { icon, text: text.to_string().into(), muted: true, stars: None, tint: None };
    Some(match sort {
        SortMode::Title | SortMode::LastPlayed | SortMode::Playtime => return None,
        SortMode::DateAdded => match o.added(g) {
            Some(d) => shown(Icon::Inbox, format_timestamp(d, locale)),
            None => missing(Icon::Inbox, tr!("Unknown")),
        },
        SortMode::TimesPlayed => match organize::times_played(g) {
            0 => missing(Icon::Replay, tr!("Never played")),
            n => shown(Icon::Replay, trn!(n, "{n} session", "{n} sessions")),
        },
        SortMode::InstallSize => match o.size(g).filter(|_| g.installed) {
            Some(bytes) => shown(Icon::FaHardDrive, heronite_epic::util::file_size_local(bytes as f64)),
            None if g.installed => missing(Icon::FaHardDrive, tr!("Size unknown")),
            None => missing(Icon::FaHardDrive, tr!("Not installed")),
        },
        SortMode::ReleaseDate => match g.details.release_date.as_deref().filter(|d| organize::release_ymd(d).is_some()) {
            Some(d) => shown(Icon::Event, format_timestamp(d, locale)),
            None => missing(Icon::Event, tr!("Unknown")),
        },
        SortMode::Rating => match o.rating(g) {
            Some(stars) => Caption { icon: Icon::Star, text: SharedString::default(), muted: false, stars: Some(stars), tint: None },
            None => missing(Icon::StarBorder, tr!("Not rated")),
        },
        SortMode::CriticScore => match g.details.critic_score.filter(|s| *s > 0.) {
            Some(score) => shown(Icon::BarChart, tr!("Score {score}", score = format!("{score:.0}"))),
            None => missing(Icon::BarChart, tr!("No score")),
        },
    })
}

/// The line under a cover for Settings → Appearance → "Under the cover";
/// `None` keeps the playtime. Title and Nothing hide the line.
fn chosen_caption(g: &CollectionGame, lib: &Library, ui: &UiState, o: &Organize) -> Option<Caption> {
    match theme::appearance().cover_label {
        CoverLabel::Auto => caption(g, ui.sort, o, lib.date_locale),
        CoverLabel::Playtime | CoverLabel::Title | CoverLabel::Nothing => None,
        CoverLabel::LastPlayed => Some(match g.last_played.as_deref().and_then(days_since) {
            Some(days) => Caption { icon: Icon::History, text: days_ago(days).into(), muted: false, stars: None, tint: None },
            None => Caption { icon: Icon::History, text: tr!("Never played").into(), muted: true, stars: None, tint: None },
        }),
        CoverLabel::Status => display_status(g.status_raw.as_deref(), &lib.statuses).map(|st| Caption {
            icon: Icon::Flag,
            text: heronite_data::status::status_label(&st.name).to_string().into(),
            muted: false,
            stars: None,
            tint: Some(theme::status_color(&st.slug).into()),
        }),
    }
}

/// The icon a section header of `by` shows instead of the status dot.
pub(super) fn group_icon(by: GroupBy) -> Option<Icon> {
    Some(match by {
        GroupBy::None | GroupBy::Status => return None,
        GroupBy::Category => Icon::Label,
        GroupBy::Store => Icon::Storefront,
        GroupBy::Platform => Icon::SportsEsports,
        GroupBy::Genre => Icon::Palette,
        GroupBy::Developer => Icon::Build,
        GroupBy::Series => Icon::Folder,
        GroupBy::Year => Icon::Event,
        GroupBy::Playtime => Icon::AccessTime,
        GroupBy::InstallSize => Icon::FaHardDrive,
    })
}

/// The list's query: the search and the View menu's filters and sort (the
/// app's own conditions are [`CollectionView::keeps_game`](super::CollectionView::keeps_game)).
pub(super) fn view_query(ui: &UiState, search: &str) -> ViewQuery {
    ViewQuery {
        search: search.to_string(),
        install: ui.install_filter,
        facets: ui.facet_filters.clone(),
        sort: ui.sort,
        descending: ui.sort_descending,
        favourites_only: ui.only.favourites,
        offline_only: ui.only.offline,
    }
}

/// `keep`: the app's own conditions (categories, statuses, games with an update).
pub(super) fn build_groups(
    lib: &Library,
    ui: &UiState,
    search: &str,
    keep: &dyn Fn(&CollectionGame) -> bool,
    is_playing: &dyn Fn(&str) -> bool,
    launch_step: &dyn Fn(&str) -> Option<SharedString>,
    organize: &Organize,
) -> Vec<GroupVm> {
    let filtered = collection::filter_games_organized(&lib.games, &view_query(ui, search), keep, organize);
    let card = |g: &Game| {
        // A launch that is preparing says its step under the cover.
        let step = launch_step(&g.key).map(|text| Caption { icon: Icon::Refresh, text, muted: false, stars: None, tint: None });
        card_vm_with(g, is_playing(&g.key), step.or_else(|| chosen_caption(g, lib, ui, organize)))
    };
    let icon = group_icon(ui.group_by);
    organize::group_games(&filtered, ui.group_by, organize)
        .into_iter()
        .map(|sg| GroupVm {
            collapsed: ui.group_by != GroupBy::None && ui.collapsed_groups.contains(&sg.id),
            show_header: ui.group_by != GroupBy::None,
            id: sg.id.clone().into(),
            name: sg.name.clone().into(),
            color: sg.color.map(parse_hex).unwrap_or(theme::status_color("custom")),
            icon,
            cards: sg.games.iter().map(card).collect(),
        })
        .collect()
}

pub(super) fn focus_vm(lib: &Library, wiki: &WikiCache, g: &CollectionGame, playing: bool) -> FocusVm {
    let fd = focus_data(lib, g, wiki);
    let d = &fd.details;
    let mut rows = Vec::new();
    let text = |s: &str| DetailValue::Text(s.to_string().into());
    // Every value: Details shows one line of them and "+N" for the rest.
    let chips = |v: &[String], facet: Option<&'static str>| DetailValue::Chips {
        values: v.iter().map(|s| SharedString::from(s.clone())).collect(),
        facet,
    };

    if !d.developers.is_empty() {
        rows.push(DetailRow { label: tr!("Developer").into(), value: chips(&d.developers, Some("developer")), split: false });
    }
    if !d.publishers.is_empty() {
        rows.push(DetailRow { label: tr!("Publisher").into(), value: chips(&d.publishers, Some("publisher")), split: false });
    }
    if let Some(series) = &d.series {
        rows.push(DetailRow { label: tr!("Series").into(), value: chips(std::slice::from_ref(series), Some("series")), split: false });
    }
    let last_played = match g.last_played.as_deref().and_then(days_since) {
        Some(days) if days < 60 => Some(days_ago(days)),
        _ => fd.last_played.clone(),
    };
    rows.push(DetailRow { label: tr!("Last Played").into(), value: text(last_played.as_deref().unwrap_or(tr!("Never"))), split: false });
    rows.push(DetailRow { label: tr!("Playtime").into(), value: text(&fd.playtime), split: false });
    if !d.features.is_empty() {
        rows.push(DetailRow {
            label: tr!("Features").into(),
            // Steam's features (no metadata) are not a filter.
            value: chips(&d.features, (!g.facets[FacetKind::Feature.index()].is_empty()).then_some("feature")),
            split: true,
        });
    }
    if !d.themes.is_empty() {
        rows.push(DetailRow { label: tr!("Themes").into(), value: chips(&d.themes, Some("theme")), split: true });
    }
    if d.platforms.len() > 1 {
        // The PC ones first: this is a PC launcher.
        let mut platforms = d.platforms.clone();
        platforms.sort_by_key(|p| !is_pc_platform(p));
        rows.push(DetailRow { label: tr!("Platforms").into(), value: chips(&platforms, Some("platform")), split: true });
    }
    if !d.genres.is_empty() {
        rows.push(DetailRow { label: tr!("Tags").into(), value: chips(&d.genres, Some("genre")), split: true });
    }
    if let Some(notes) = d.notes.as_ref().filter(|n| !n.is_empty()) {
        rows.push(DetailRow { label: tr!("Notes").into(), value: text(notes), split: true });
    }

    let related = collection::related_games_for(g, &lib.games)
        .into_iter()
        .filter(|r| !r.games.is_empty())
        .map(|r| RelatedGroupVm {
            title: match r.kind {
                RelatedKind::Series => tr!("Same series").into(),
                RelatedKind::Developer => tr!("Same developer").into(),
                RelatedKind::Similar => tr!("Similar").into(),
            },
            games: r.games.iter().map(|x| related_game_vm(lib, x)).collect(),
        })
        .collect();

    let overview = if !d.description.trim().is_empty() { Some(d.description.clone().into()) } else { None };
    let overview_html = d.description_html.as_deref().map(super::html::parse).filter(|b| !b.is_empty()).map(Arc::new);

    FocusVm {
        key: g.key.clone().into(),
        title: g.title.clone().into(),
        meta_line: fd.meta_line.clone().into(),
        cover_key: g.key.clone().into(),
        overview,
        overview_html,
        related,
        details: rows,
        hltb: fd.hltb.as_ref().filter(|h| h.main_story > 0. || h.main_extra > 0.).map(|h| HltbVm {
            main: Some(h.main_story as f32),
            extra: Some(h.main_extra as f32),
            complete: Some(h.completionist as f32),
            played_hours: g.playtime_minutes as f32 / 60.,
        }),
        status: display_status(g.status_raw.as_deref(), &lib.statuses).map(|st| StatusVm {
            id: st.id.clone().into(),
            name: heronite_data::status::status_label(&st.name).to_string().into(),
            color: theme::status_color(&st.slug),
        }),
        score: d.critic_score.filter(|s| *s > 0.),
        score_source: g.metadata.as_ref().and_then(|m| m.field_sources.get("criticScore")).and_then(|s| match s.as_str() {
            "igdb" => Some("IGDB".into()),
            "steam" => Some("Steam".into()),
            _ => None,
        }),
        library: g.source_label.into(),
        install_size: d.install_size.clone().filter(|s| s != "0").map(Into::into),
        installed: g.installed,
        playing,
        favourite: g.favourite,
        playtime: fd.playtime.clone().into(),
        last_played: last_played.map(Into::into),
    }
}

fn is_pc_platform(name: &str) -> bool {
    let n = name.to_lowercase();
    ["pc", "windows", "linux", "mac", "steamos", "dos"].iter().any(|w| n.split(|c: char| !c.is_alphanumeric()).any(|part| part == *w))
}

fn related_game_vm(lib: &Library, g: &CollectionGame) -> RelatedGameVm {
    let status = display_status(g.status_raw.as_deref(), &lib.statuses);
    let line = if g.playtime_minutes > 0 {
        collection::format_playtime_minutes(g.playtime_minutes)
    } else {
        status.map_or_else(|| tr!("Never played").to_string(), |st| heronite_data::status::status_label(&st.name).to_string())
    };
    RelatedGameVm {
        key: g.key.clone().into(),
        title: g.title.clone().into(),
        cover_key: g.key.clone().into(),
        status_color: status.map(|st| theme::status_color(&st.slug)),
        line: line.into(),
    }
}

/// Whole local days from an RFC 3339 moment to today (0 = today).
pub(super) fn days_since(rfc3339: &str) -> Option<i64> {
    let at = chrono::DateTime::parse_from_rfc3339(rfc3339).ok()?.with_timezone(&chrono::Local);
    Some((chrono::Local::now().date_naive() - at.date_naive()).num_days().max(0))
}

/// "Today", "Yesterday", "3 days ago", "5 weeks ago", "4 months ago".
pub(super) fn days_ago(days: i64) -> String {
    match days {
        ..=0 => tr!("Today").to_string(),
        1 => tr!("Yesterday").to_string(),
        2..=13 => tr!("{days} days ago", days),
        14..=59 => tr!("{weeks} weeks ago", weeks = days / 7),
        _ => tr!("{months} months ago", months = days / 30),
    }
}

/// The day of a "YYYY-MM-DD..." text (a date or the start of a timestamp).
pub(super) fn parse_day(text: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()
}

/// [`days_ago`] for a last-played moment: an RFC 3339 time, or just a day.
pub(super) fn played_ago(last_played: &str) -> Option<String> {
    let days = days_since(last_played).or_else(|| Some((chrono::Local::now().date_naive() - parse_day(last_played)?).num_days().max(0)))?;
    Some(days_ago(days))
}
