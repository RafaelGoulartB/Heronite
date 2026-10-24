//! What a console tab shows: its games (`TabSpec::source`) and, for the
//! layouts that have them, the theme's shelves (`ConsoleTheme::shelves`).
//! Both are rebuilt from the library on each call (cheap: a filter and a
//! sort over the games) so a theme edit shows at once.

use std::sync::Arc;

use gpui_kit::*;
use heronite_data::console_theme::ShelfSpec;
use heronite_data::organize::Organize;
use heronite_data::{CollectionGame, Library};
use heronite_i18n::{tr, trc};

use super::super::CollectionView;

/// One row of covers.
pub(crate) struct Shelf {
    /// The `ShelfSpec::kind` it comes from.
    pub kind: String,
    pub title: SharedString,
    pub keys: Vec<SharedString>,
}

pub(super) fn by_last_played(mut v: Vec<&CollectionGame>) -> Vec<&CollectionGame> {
    v.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower)));
    v
}

pub(super) fn by_title(mut v: Vec<&CollectionGame>) -> Vec<&CollectionGame> {
    v.sort_by(|a, b| a.title_lower.cmp(&b.title_lower));
    v
}

pub(super) fn keys(v: Vec<&CollectionGame>) -> Vec<SharedString> {
    v.into_iter().map(|g| SharedString::from(g.key.clone())).collect()
}

/// The status of `id` by its slug or id (`plan-to-play`, `beaten`...).
fn status_is(lib: &Library, g: &CollectionGame, slug: &str) -> bool {
    g.status_id == slug || lib.statuses.iter().any(|s| s.id == g.status_id && s.slug == slug)
}

/// A day number for "YYYY-MM-DD..." (enough to compare ages).
fn day_number(date: &str) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(date.get(..10)?, "%Y-%m-%d").ok()?;
    Some(i64::from(d.num_days_from_ce()))
}

use chrono::Datelike as _;

impl CollectionView {
    fn organize(&self) -> Organize<'_> {
        Organize { extras: Some(&self.game_extras), categories: Some(&self.categories), ..Default::default() }
    }

    /// Whether `g` belongs to a tab's `source` (see `TabSpec::source`).
    pub(super) fn console_in_source(&self, lib: &Library, g: &CollectionGame, source: &str) -> bool {
        let (kind, value) = source.split_once(':').unwrap_or((source, ""));
        match kind {
            "installed" => g.installed,
            "not-installed" => !g.installed,
            "favourites" => g.favourite,
            "pc" => !self.game_extras.systems.contains_key(&g.key),
            "emulated" => self.game_extras.systems.contains_key(&g.key),
            "status" => status_is(lib, g, value),
            "category" => self.categories.contains(value, &g.runner, &g.app_name),
            "store" => g.source_label == value,
            "platform" => self.organize().platform(g) == value,
            _ => true,
        }
    }

    /// The games of `source`, in library order.
    pub(super) fn console_games(&self, source: &str) -> Vec<Arc<CollectionGame>> {
        let Some(lib) = self.lib.as_ref() else { return Vec::new() };
        lib.games.iter().filter(|g| self.console_in_source(lib, g, source)).cloned().collect()
    }

    /// The theme's shelves over `source`, empty ones left out.
    pub(super) fn console_shelves(&self, source: &str) -> Vec<Shelf> {
        let Some(lib) = self.lib.clone() else { return Vec::new() };
        let theme = crate::console::look::theme();
        let games: Vec<&CollectionGame> =
            lib.games.iter().map(|g| g.as_ref()).filter(|g| self.console_in_source(&lib, g, source)).collect();
        let limit = if theme.shelf_limit == 0 { usize::MAX } else { theme.shelf_limit as usize };
        theme
            .shelves
            .iter()
            .filter(|s| s.on)
            .filter_map(|spec| {
                let mut keys = keys(self.shelf_games(&lib, &games, &spec.kind));
                keys.truncate(limit);
                (!keys.is_empty()).then(|| Shelf { kind: spec.kind.clone(), title: shelf_title(&lib, spec).into(), keys })
            })
            .collect()
    }

    fn shelf_games<'a>(&self, lib: &Library, games: &[&'a CollectionGame], kind: &str) -> Vec<&'a CollectionGame> {
        let (k, value) = kind.split_once(':').unwrap_or((kind, ""));
        let pick = |f: &dyn Fn(&CollectionGame) -> bool| games.iter().copied().filter(|g| f(g)).collect::<Vec<_>>();
        match k {
            "recent" => {
                let mut v = by_last_played(pick(&|g| g.last_played.is_some()));
                v.truncate(20);
                v
            }
            "favourites" => by_last_played(pick(&|g| g.favourite)),
            "installed" => by_last_played(pick(&|g| g.installed)),
            "added" => {
                let org = self.organize();
                let mut v: Vec<(&CollectionGame, &str)> = games.iter().filter_map(|g| Some((*g, org.added(g)?))).collect();
                v.sort_by(|a, b| b.1.cmp(a.1));
                v.into_iter().take(20).map(|(g, _)| g).collect()
            }
            "backlog-short" => {
                let mut v: Vec<(&CollectionGame, f64)> = games
                    .iter()
                    .filter(|g| status_is(lib, g, "plan-to-play") || (g.playtime_minutes == 0 && g.last_played.is_none()))
                    .filter_map(|g| Some((*g, self.wiki.hltb(&g.title, &g.game.title)?.main_story)))
                    .filter(|(_, h)| *h > 0. && *h <= 15.)
                    .collect();
                v.sort_by(|a, b| a.1.total_cmp(&b.1));
                v.into_iter().map(|(g, _)| g).collect()
            }
            "random" => {
                // A different pick each day, the same all day long.
                let day = day_number(&chrono::Local::now().format("%Y-%m-%d").to_string()).unwrap_or(0) as u64;
                let mut v: Vec<(u64, &CollectionGame)> = games
                    .iter()
                    .filter(|g| status_is(lib, g, "plan-to-play") || g.playtime_minutes == 0)
                    .map(|g| (fxhash(&g.key) ^ day.wrapping_mul(0x9e37_79b9_7f4a_7c15), *g))
                    .collect();
                v.sort_by_key(|(h, _)| *h);
                v.into_iter().take(6).map(|(_, g)| g).collect()
            }
            "forgotten" => {
                let today = day_number(&chrono::Local::now().format("%Y-%m-%d").to_string()).unwrap_or(0);
                by_last_played(pick(&|g| g.installed && g.last_played.as_deref().and_then(day_number).is_none_or(|d| today - d > 90)))
            }
            "status" => by_last_played(pick(&|g| status_is(lib, g, value))),
            "store" => by_last_played(pick(&|g| g.source_label == value)),
            "platform" => {
                let org = self.organize();
                by_last_played(pick(&|g| org.platform(g) == value))
            }
            "genre" => by_last_played(pick(&|g| g.metadata.as_ref().is_some_and(|m| m.genres.iter().any(|x| x == value)))),
            "category" => by_last_played(pick(&|g| self.categories.contains(value, &g.runner, &g.app_name))),
            _ => Vec::new(),
        }
    }
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// The name a shelf shows: its own title, else its kind's.
pub(crate) fn shelf_title(lib: &Library, spec: &ShelfSpec) -> String {
    if !spec.title.trim().is_empty() {
        return spec.title.clone();
    }
    kind_label(lib, &spec.kind)
}

/// The name of a shelf kind (also used by the theme editors).
pub(crate) fn kind_label(lib: &Library, kind: &str) -> String {
    let (k, value) = kind.split_once(':').unwrap_or((kind, ""));
    match k {
        "recent" => tr!("Continue playing").into(),
        "favourites" => tr!("Favourites").into(),
        "installed" => trc!("shelf", "Installed").into(),
        "added" => tr!("Recently added").into(),
        "backlog-short" => tr!("Short ones from the backlog").into(),
        "random" => tr!("Pick of the day").into(),
        "forgotten" => tr!("Installed and forgotten").into(),
        "status" => lib
            .statuses
            .iter()
            .find(|s| s.id == value || s.slug == value)
            .map_or_else(|| value.to_string(), |s| heronite_data::status::status_label(&s.name).to_string()),
        "category" => super::super::view_filters::category_label(value),
        _ => value.to_string(),
    }
}
