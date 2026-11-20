//! The library's games as palette entries, with what the preview pane shows
//! of each: shared by the Ctrl K palette and the quick launcher.

use std::rc::Rc;

use heronite_data::status::{display_status, status_label};
use heronite_data::Library;

use super::super::data::played_ago;
use super::sources::{game_entry, Entry, GameCard, GameRow};

/// Every game of `lib`, by title. `playing`: whether the game with this
/// `app_name` runs now.
pub(crate) fn library_entries(lib: &Library, playing: &dyn Fn(&str) -> bool) -> Vec<Entry> {
    let mut games: Vec<_> = lib.games.iter().collect();
    games.sort_by(|a, b| a.title_lower.cmp(&b.title_lower).then(a.key.cmp(&b.key)));
    let mut v = Vec::with_capacity(games.len());
    for g in games {
        let playtime = heronite_data::collection::playtime_label(g.playtime_minutes);
        let mut e = game_entry(&GameRow {
            key: &g.key,
            title: &g.title,
            store_title: &g.game.title,
            source: g.source_label,
            playtime: &playtime,
            installed: g.installed,
            has_folder: g.details.install_path.is_some(),
            playing: playing(&g.app_name),
            favourite: g.favourite,
            hidden: g.hidden,
            last_played: g.last_played.as_deref(),
        });
        let card = GameCard {
            title: g.title.clone().into(),
            source: g.source_label.into(),
            playtime: playtime.into(),
            last_played: g.last_played.as_deref().and_then(played_ago).map(Into::into),
            status: display_status(g.status_raw.as_deref(), &lib.statuses)
                .map(|s| (status_label(&s.name).to_string().into(), s.slug.clone())),
        };
        if let Some(bits) = e.game.as_mut() {
            bits.card = Some(Rc::new(card));
        }
        v.push(e);
    }
    v
}
