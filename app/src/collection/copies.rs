//! Copies of one game from several stores (`heronite_data::copies`): the
//! focus panel's Copies card (each copy, Play it, make it the default,
//! the possible copies to link, ungroup), and the changes that go to every
//! copy of a grouped card (status, favourite, hidden, categories, rating).

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::{copies, CollectionGame};
use heronite_i18n::{tr, trn};

use super::focus::panel;
use super::game_actions::{GameAction, GameActionItem};
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tooltip};

impl CollectionView {
    /// The games a change to `key` goes to: every copy while copies are
    /// grouped, so the card does not change when another copy represents it.
    pub(super) fn with_copies(&self, key: &str) -> Vec<Arc<CollectionGame>> {
        let Some(lib) = self.lib.as_ref() else { return Vec::new() };
        if self.ui.group_copies {
            lib.copies_of(key).into_iter().cloned().collect()
        } else {
            lib.get(key).cloned().into_iter().collect()
        }
    }

    /// The game's other copies in the context menu and palette ("Play with
    /// GOG"), and "Ungroup copies".
    pub(super) fn copies_actions(&self, game: &CollectionGame) -> Vec<GameActionItem> {
        let Some(lib) = self.lib.as_ref().filter(|l| l.copies.get(&game.key).is_some()) else { return Vec::new() };
        let mut v: Vec<GameActionItem> = lib
            .copies_of(&game.key)
            .into_iter()
            .filter(|c| c.key != game.key && !self.playing.contains_key(&c.app_name))
            .map(|c| {
                let (label, ic) = if c.installed {
                    (tr!("Play with {store}", store = c.source_label), Icon::PlayArrow)
                } else {
                    (tr!("Install with {store}", store = c.source_label), Icon::Download)
                };
                GameActionItem::new(
                    GameAction::PlayCopy(c.key.clone()),
                    format!("copy-{}", c.key),
                    label,
                    ic,
                    "copy store launch play install jogar copia loja",
                )
            })
            .collect();
        v.push(GameActionItem::new(
            GameAction::UngroupCopies,
            "ungroup-copies",
            tr!("Ungroup copies"),
            Icon::Close,
            "split separate copies duplicate desagrupar copias separar",
        ));
        v
    }

    /// Favourite (or not) every copy, as the game under `key` is not.
    pub(super) fn toggle_favourite_of(&mut self, key: &str, cx: &mut Context<Self>) {
        let games = self.with_copies(key);
        let Some(on) = games.iter().find(|g| g.key == key).map(|g| !g.favourite) else { return };
        let writes: Vec<_> = games.iter().map(|g| (g.app_name.clone(), g.title.clone(), on)).collect();
        if let Err(err) = heronite_data::config_games::set_favourites(&self.data_paths, &writes) {
            self.show_toast(tr!("Could not update favourites: {err}", err = format!("{err:#}")), true, cx);
        }
        self.reload(cx);
    }

    /// Hides every copy, or shows them all again.
    pub(super) fn toggle_hidden_of(&mut self, key: &str, cx: &mut Context<Self>) {
        let games = self.with_copies(key);
        let Some(on) = games.iter().find(|g| g.key == key).map(|g| !g.hidden) else { return };
        let writes: Vec<_> = games.iter().map(|g| (g.app_name.clone(), g.title.clone(), on)).collect();
        if let Err(err) = heronite_data::config_games::set_hidden(&self.data_paths, &writes) {
            self.show_toast(format!("{err:#}"), true, cx);
        }
        self.reload(cx);
    }

    /// Plays (or installs) this copy, whichever copy is the default.
    pub(super) fn play_copy(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        if self.refuse_busy_launch(&game, cx) {
            return;
        }
        let result = if game.installed { self.runtime.play(&game) } else { self.runtime.install(&game) };
        if let Err(err) = result {
            self.report_launch_error(err, cx);
        }
    }

    /// Makes `key` the copy its group plays and shows.
    pub(super) fn make_default_copy(&mut self, key: &str, cx: &mut Context<Self>) {
        let members: Vec<String> = self.with_copies(key).iter().map(|g| g.key.clone()).collect();
        let res = copies::set_preferred(&self.data_paths, key, &members);
        self.after_copies_change(res, None, cx);
    }

    /// Takes the copies of `key` apart again: each a game of its own, never
    /// grouped again by itself. Their records are not touched.
    pub(super) fn ungroup_copies(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(group) = self.lib.as_ref().and_then(|l| l.copies.get(key)).cloned() else { return };
        let res = copies::unlink(&self.data_paths, &group.members);
        let n = group.members.len();
        self.after_copies_change(res, Some(trn!(n, "{n} copy is a game of its own again", "{n} copies are games of their own again")), cx);
    }

    /// Links `keys` (and the groups they are in) as copies of one game.
    pub(super) fn link_copies(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        if keys.len() < 2 {
            return;
        }
        let mut all: Vec<String> = Vec::new();
        for k in &keys {
            for g in self.lib.as_ref().map(|l| l.copies_of(k)).unwrap_or_default() {
                if !all.contains(&g.key) {
                    all.push(g.key.clone());
                }
            }
        }
        let res = copies::link(&self.data_paths, &all);
        let n = all.len();
        self.after_copies_change(res, Some(trn!(n, "Linked as {n} copy of one game", "Linked as {n} copies of one game")), cx);
    }

    /// "Link as copies" on the selected games.
    pub(super) fn link_selected_copies(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.sel.pop = None;
        let keys = self.selected_games().iter().map(|g| g.key.clone()).collect();
        self.link_copies(keys, cx);
    }

    fn after_copies_change(&mut self, res: anyhow::Result<()>, done: Option<String>, cx: &mut Context<Self>) {
        self.context_menu = None;
        match res {
            Err(err) => self.show_toast(tr!("Could not save the copies: {err}", err = format!("{err:#}")), true, cx),
            Ok(()) => {
                if let Some(text) = done {
                    self.show_toast(text, false, cx);
                }
            }
        }
        self.reload(cx);
    }

    /// The Copies card of the focus panel: shown for a game with other copies
    /// or possible copies.
    pub(super) fn copies_card(&self, key: &SharedString, cx: &mut Context<Self>) -> Option<Div> {
        let lib = self.lib.as_ref()?;
        let group = lib.copies.get(key);
        let suggested: Vec<_> = lib.copies.suggested(key).iter().filter_map(|k| lib.get(k).cloned()).collect();
        if group.is_none() && suggested.is_empty() {
            return None;
        }
        let copies = lib.copies_of(key);
        let mut card = panel(true).gap(px(2.));
        if let Some(group) = group {
            let totals = lib.copy_totals(key);
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py(px(4.))
                    .text_size(px(13.))
                    .child(div().text_color(theme::ink_a(0.66)).child(tr!("Copies")))
                    .child(
                        div()
                            .text_color(theme::ink_a(0.66))
                            .child(tr!("{time} in all", time = format_playtime_minutes(totals.playtime_minutes))),
                    ),
            );
            let launch = lib.launch_copy(key).map(|g| g.key.clone());
            for (i, g) in copies.iter().enumerate() {
                card = card.child(self.copy_row(g, i == 0, group.chosen, launch.as_deref() == Some(g.key.as_str()), cx));
            }
            let key = key.clone();
            card = card.child(
                div()
                    .id("copies-ungroup")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .mt(px(4.))
                    .py(px(5.))
                    .mx(px(-8.))
                    .px(px(8.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::ink_a(0.06)))
                    .text_size(px(12.5))
                    .text_color(theme::ink_a(0.72))
                    .child(icon(Icon::Close, px(14.), theme::ink_a(0.6)))
                    .child(tr!("Ungroup copies"))
                    .on_click(cx.listener(move |this, _, _, cx| this.ungroup_copies(&key, cx))),
            );
        }
        if !suggested.is_empty() {
            card = card.child(
                div()
                    .mt(px(if group.is_some() { 6. } else { 4. }))
                    .py(px(4.))
                    .text_size(px(13.))
                    .text_color(theme::ink_a(0.66))
                    .child(tr!("Possible copies")),
            );
            for g in suggested {
                let (key, other) = (key.to_string(), g.key.clone());
                card = card.child(
                    row_base(&g).id(SharedString::from(format!("copy-link-{}", g.key))).child(
                        div()
                            .id(SharedString::from(format!("copy-link-btn-{}", g.key)))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .bg(theme::ink_a(0.06))
                            .hover(|s| s.bg(theme::ink_a(0.12)))
                            .text_size(px(12.))
                            .child(icon(Icon::Link, px(13.), theme::ink_a(0.7)))
                            .child(tr!("Link"))
                            .on_click(cx.listener(move |this, _, _, cx| this.link_copies(vec![key.clone(), other.clone()], cx))),
                    ),
                );
            }
        }
        Some(card)
    }

    /// One copy: its store, its title when it differs, its own playtime, and
    /// Play / "Make default".
    fn copy_row(&self, g: &CollectionGame, first: bool, chosen: bool, plays: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = g.key.clone();
        let default_tip = if first && chosen {
            tr!("The default copy")
        } else if first {
            tr!("Played first (installed or most played)")
        } else {
            tr!("Make default")
        };
        let star = div()
            .id(SharedString::from(format!("copy-default-{key}")))
            .flex_none()
            .cursor_pointer()
            .child(icon(
                if first { Icon::Star } else { Icon::StarBorder },
                px(15.),
                if first { Hsla::from(theme::accent()) } else { theme::ink_a(0.45) },
            ))
            .tooltip(tooltip(default_tip))
            .when(!(first && chosen), |d| {
                let key = key.clone();
                d.on_click(cx.listener(move |this, _, _, cx| this.make_default_copy(&key, cx)))
            });
        let playing = self.playing.contains_key(&g.app_name);
        let play_icon = if playing {
            Icon::Stop
        } else if g.installed {
            Icon::PlayArrow
        } else {
            Icon::Download
        };
        let play_tip = if g.installed { tr!("Play this copy") } else { tr!("Install this copy") };
        let play = div()
            .id(SharedString::from(format!("copy-play-{key}")))
            .flex_none()
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .when(plays, |d| d.bg(theme::accent_tint()))
            .hover(|s| s.bg(theme::ink_a(0.1)))
            .child(icon(play_icon, px(15.), theme::ink_a(0.85)))
            .tooltip(tooltip(play_tip))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.playing.contains_key(this.lib.as_ref().and_then(|l| l.get(&key)).map_or("", |g| g.app_name.as_str())) {
                    this.primary_action(&key, cx);
                } else {
                    this.play_copy(&key, cx);
                }
            }));
        row_base(g).id(SharedString::from(format!("copy-{}", g.key))).child(star).child(play)
    }
}

/// A copy's store, title (when it is not the Collection's own) and playtime.
fn row_base(g: &CollectionGame) -> Div {
    let store_title = g.game.title.trim();
    let edition = (!store_title.is_empty() && store_title != g.title).then(|| store_title.to_string());
    div().flex().items_center().gap(px(8.)).py(px(4.)).text_size(px(13.)).child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().text_color(theme::ink_a(0.94)).child(g.source_label))
                    .when(g.installed, |d| d.child(div().size(px(6.)).rounded_full().bg(theme::success()))),
            )
            .child(div().truncate().text_size(px(11.5)).text_color(theme::ink_a(0.55)).child(match edition {
                Some(t) => format!("{t} · {}", g.playtime_label()),
                None => g.playtime_label(),
            })),
    )
}
