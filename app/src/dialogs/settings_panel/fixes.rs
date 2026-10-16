//! "Game fixes" in the Wine tab: the umu id that picks the protonfixes of a
//! game (`heronite_launch::umu_ids`). It is found by itself; the field only
//! replaces it, and suggestions from the umu database fill it in one click.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::umu_ids::{self, Entry, GameKeys, Resolved, Source};
use serde_json::Value;

use super::logic::RunKind;
use super::parts::{heading, help, rows};
use super::SettingsPanel;
use crate::ui::controls::{action_button, ButtonKind};

/// The umu database and Heroic's cached ids, read off the UI thread.
#[derive(Default)]
pub struct UmuData {
    pub db: Arc<Vec<Entry>>,
    pub heroic_cache: Option<Value>,
}

/// Suggestions shown when nothing was found.
const SUGGESTIONS: usize = 3;

impl SettingsPanel {
    pub(super) fn load_umu(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let data = cx
                .background_spawn(async move {
                    let cache = heronite_data::json_store::read_value(&paths.store_cache("umu")).ok().filter(|v| !v.is_null());
                    UmuData { db: Arc::new(umu_ids::load(&paths)), heroic_cache: cache }
                })
                .await;
            this.update(cx, |this, cx| {
                this.umu = Some(data);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Whether the game runs through umu: a Windows game with Proton and umu on.
    pub(super) fn shows_game_fixes(&self) -> bool {
        self.facts().is_some_and(|f| f.run_kind == RunKind::Wine) && self.wine_version().kind == "proton" && !self.flag("disableUMU", false)
    }

    /// What Heronite finds without the user's id.
    pub(super) fn umu_automatic(&self) -> Option<Resolved> {
        let (f, data) = (self.facts()?, self.umu.as_ref()?);
        let keys =
            GameKeys { runner: &f.runner, app_name: &f.app_name, title: &f.title, steam_app_id: f.protondb_id.as_deref(), user_id: "" };
        umu_ids::resolve(&data.db, data.heroic_cache.as_ref(), keys)
    }

    /// The line under the field: what runs now and where it came from.
    pub(super) fn umu_status(&self) -> String {
        let user = umu_ids::normalize(&self.text(umu_ids::KEY));
        let auto = self.umu_automatic();
        let named = |id: &str, title: Option<&str>| match title {
            Some(t) => format!("{id} ({t})"),
            None => id.to_string(),
        };
        if !user.is_empty() {
            let title = self.umu.as_ref().and_then(|d| d.db.iter().find(|e| e.umu_id == user)).map(|e| e.title.as_str());
            let shown = named(&user, title);
            return match auto {
                Some(a) => tr!("Using {id}, set by you. Clear the field to use {auto}, found by Heronite.", id = shown, auto = a.id),
                None => tr!("Using {id}, set by you.", id = shown),
            };
        }
        match auto {
            None if self.umu.as_ref().is_some_and(|d| d.db.is_empty()) => {
                tr!("The umu database has not been downloaded yet. It is downloaded when Heronite starts online.").into()
            }
            None => tr!("No ID found: the game runs without fixes. Set one if the game has fixes.").into(),
            Some(r) => {
                let shown = named(&r.id, r.title.as_deref());
                match r.source {
                    Source::Store => tr!("Using {id}, found in the umu database by the store's ID.", id = shown),
                    Source::HeroicCache => tr!("Using {id}, found by Heroic.", id = shown),
                    Source::SteamId => tr!("Using {id}, from the game's Steam ID.", id = shown),
                    Source::Title => tr!("Using {id}, found by the game's title.", id = shown),
                    Source::User => shown,
                }
            }
        }
    }

    pub(super) fn umu_suggestions(&self) -> Vec<Entry> {
        let (Some(f), Some(data)) = (self.facts(), self.umu.as_ref()) else { return Vec::new() };
        if !self.text(umu_ids::KEY).trim().is_empty() || self.umu_automatic().is_some() {
            return Vec::new();
        }
        umu_ids::suggestions(&data.db, &f.title, SUGGESTIONS).into_iter().cloned().collect()
    }

    pub(super) fn use_umu_id(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_input("umuId", id, window, cx);
        self.commit("umuId", cx);
    }

    pub(super) fn game_fixes_section(&mut self, cx: &mut Context<Self>) -> Div {
        let mut buttons = div().flex().flex_wrap().gap(px(8.)).pt(px(4.));
        for (i, e) in self.umu_suggestions().into_iter().enumerate() {
            let id = e.umu_id.clone();
            buttons = buttons.child(action_button(
                SharedString::from(format!("umu-suggest-{i}")),
                ButtonKind::Glass,
                tr!("Use {id} ({title})", id = e.umu_id, title = e.title),
                true,
                cx.listener(move |this, _, window, cx| this.use_umu_id(&id, window, cx)),
            ));
        }
        buttons =
            buttons.child(self.outline_button("umu-site", tr!("Search the umu database"), true, cx, |_, _, _| {
                crate::ui::open_external(umu_ids::WEBSITE)
            }));
        let card = rows().field_help(
            self.text_row("umuId", Some(tr!("umu ID")), None),
            vec![
                help(&self.umu_status()),
                help(tr!("GE-Proton and other builds with protonfixes apply per-game fixes picked by this ID. Found by itself; set it only if it is missing or wrong.")),
                buttons,
            ],
        );
        div().flex().flex_col().gap(px(6.)).child(heading(tr!("Game fixes"))).child(card.card())
    }
}
