//! The replay's switch of one game (Performance tab): as in Settings →
//! Replay, or on or off for this game only. The options are the same for
//! every game. Kept in `replay.json` (not in the game's Heroic settings),
//! through the replay service, which ends or starts the recorder of a game
//! running.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::journal::collection_key;
use heronite_data::AppPaths;
use heronite_i18n::tr;

use super::console::{item, Act, Item};
use super::parts::{heading, rows};
use super::SettingsPanel;
use crate::console::list::Row;
use crate::ui::controls::{row, tabs};

/// The switch's choices: as in Settings, on, off.
const CHOICES: [Option<bool>; 3] = [None, Some(true), Some(false)];

fn label(on: Option<bool>) -> &'static str {
    match on {
        None => tr!("As in Settings"),
        Some(true) => tr!("On"),
        Some(false) => tr!("Off"),
    }
}

impl SettingsPanel {
    /// `journal::collection_key` of the game (none for the defaults).
    fn replay_key(&self) -> Option<String> {
        self.facts().map(|f| collection_key(&f.runner, &f.app_name))
    }

    /// The game's own switch, and what the replay does for it then.
    fn replay_switch(&self, cx: &App) -> Option<(Option<bool>, &'static str)> {
        let key = self.replay_key()?;
        let prefs = crate::services::replay::prefs(cx, &AppPaths::for_data(&self.paths));
        let on = prefs.game(&key);
        let summary = match (on, prefs.enabled) {
            (None, true) => tr!("On, as in Settings → Replay."),
            (None, false) => tr!("Off, as in Settings → Replay."),
            (Some(true), _) => tr!("On for this game, whatever Settings → Replay says."),
            (Some(false), _) => tr!("Off for this game: no memory taken while it runs."),
        };
        Some((on, summary))
    }

    fn set_replay_switch(&mut self, on: Option<bool>, cx: &mut Context<Self>) {
        let Some(key) = self.replay_key() else { return };
        crate::services::replay::change(cx, &AppPaths::for_data(&self.paths), |p| p.set_game(&key, on));
        cx.notify();
    }

    /// "Replay" of the Performance tab, for a game.
    pub(super) fn replay_section(&self, col: Div, cx: &mut Context<Self>) -> Div {
        let Some((on, summary)) = self.replay_switch(cx) else { return col };
        let panel = cx.entity().downgrade();
        let items: Vec<(Option<bool>, &'static str)> = CHOICES.iter().map(|c| (*c, label(*c))).collect();
        let control = tabs("replay-switch", &items, on, move |c, _, cx| {
            panel.update(cx, |this, cx| this.set_replay_switch(c, cx)).ok();
        });
        col.child(heading(tr!("Replay"))).child(rows().child(row(tr!("Keep a replay of this game"), Some(summary), control)).card())
    }

    /// Console rows of [`Self::replay_section`].
    pub(super) fn c_replay(&self, cx: &App) -> Vec<Item> {
        let Some((on, summary)) = self.replay_switch(cx) else { return Vec::new() };
        let choice = Row::choice(
            "replay-switch",
            tr!("Keep a replay of this game"),
            CHOICES.iter().map(|c| SharedString::from(label(*c))).collect(),
            CHOICES.iter().position(|c| *c == on).unwrap_or(0),
        )
        .desc(summary);
        vec![
            Self::c_header(tr!("Replay")),
            item(choice, Act::Choice(Rc::new(|this, i, cx| this.set_replay_switch(CHOICES.get(i).copied().flatten(), cx)))),
        ]
    }
}
