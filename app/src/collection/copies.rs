//! Copies of one game from several stores (`heronite_data::copies`): the
//! focus panel's Copies card (each copy, Play it, make it the default, take
//! it out of the group, the possible copies to link, ungroup), the version
//! a group of ROMs asks for on Play, and the changes that go to every copy
//! of a grouped card (status, favourite, hidden, categories, rating).

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::{copies, CollectionGame};
use heronite_i18n::{tr, trn};

use super::focus::panel;
use super::game_actions::{GameAction, GameActionItem};
use super::CollectionView;
use crate::dialogs::pick_rom::PickRomDialog;
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
        let all = lib.copies_of(&game.key);
        let details = copies::copy_details(&all.iter().map(|g| &***g).collect::<Vec<_>>());
        let mut v: Vec<GameActionItem> = all
            .iter()
            .zip(details)
            .filter(|(c, _)| c.key != game.key && !self.playing.contains_key(&c.app_name))
            .map(|(c, detail)| {
                // A ROM goes by its version ("Play Super Metroid (Hack Redux)"):
                // every ROM's store is "Emulator".
                let rom = is_rom(c).then(|| detail.unwrap_or_else(|| copies::own_title(c).to_string()));
                let (label, ic) = match (c.installed, rom) {
                    (true, Some(title)) => (tr!("Play {title}", title = title), Icon::PlayArrow),
                    (true, None) => (tr!("Play with {store}", store = c.source_label), Icon::PlayArrow),
                    (false, _) => (tr!("Install with {store}", store = c.source_label), Icon::Download),
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

    /// Plays (or installs) this copy, whichever copy is the default: what
    /// the Play / Install button does, for this copy (a GOG, Epic or Amazon
    /// copy opens its store's install dialog).
    pub(super) fn play_copy(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        self.install_or_play_on(&game, window, cx);
    }

    /// Play on a group whose installed copies are all ROMs (romhacks,
    /// regions, revisions of one game) asks which version to play, starting
    /// on the one Play would start (the default, else the one played last).
    /// Returns `true` when it asked.
    pub(super) fn ask_which_version(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if !self.ui.group_copies {
            return false;
        }
        let Some(lib) = self.lib.clone() else { return false };
        if lib.copies.get(key).is_none() {
            return false;
        }
        let all = lib.copies_of(key);
        // Play on a group that runs (or starts) stops it.
        if all.iter().any(|g| self.playing.contains_key(&g.app_name) || self.preparing.contains_key(&g.app_name)) {
            return false;
        }
        let details = copies::copy_details(&all.iter().map(|g| &***g).collect::<Vec<_>>());
        let versions: Vec<(&Arc<CollectionGame>, Option<String>)> = all.iter().copied().zip(details).filter(|(g, _)| g.installed).collect();
        if versions.len() < 2 || !versions.iter().all(|(g, _)| is_rom(g)) {
            return false;
        }
        // `copies_of` is in play order: the first installed copy is the one Play starts.
        let first = versions.first().map(|(g, _)| g.key.clone());
        let rows = versions
            .iter()
            .map(|(g, detail)| {
                let name = detail.clone().unwrap_or_else(|| copies::own_title(g).to_string());
                (name, g.key.clone(), format!("{} · {}", copies::store_label(g), g.playtime_label()))
            })
            .collect();
        let title = lib.get(key).map(|g| g.title.clone()).unwrap_or_default();
        self.context_menu = None;
        let weak = cx.entity().downgrade();
        let on_play: Rc<dyn Fn(String, &mut App)> = Rc::new(move |copy, cx| {
            weak.update(cx, |this, cx| {
                this.dialog = None;
                this.dialog_sub = None;
                // Started meanwhile (quick launch): its Play would stop it.
                if !this.is_running(&copy) {
                    this.play_copy_now(&copy, cx);
                }
                cx.notify();
            })
            .ok();
        });
        let view = cx.new(|_| PickRomDialog::versions(title, rows, first, on_play));
        self.open_dialog(view, cx);
        true
    }

    /// Takes the copy `key` out of its group; the other copies stay grouped.
    pub(super) fn detach_copy(&mut self, key: &str, cx: &mut Context<Self>) {
        // The whole group, even with "Group copies of a game" off.
        let Some(members) = self.lib.as_ref().and_then(|l| l.copies.get(key)).map(|g| g.members.clone()) else { return };
        let res = copies::detach(&self.data_paths, key, &members);
        self.after_copies_change(res, Some(tr!("It is a game of its own again").to_string()), cx);
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
            let details = copies::copy_details(&copies.iter().map(|g| &***g).collect::<Vec<_>>());
            // With two copies, taking one out is "Ungroup copies".
            let detachable = copies.len() > 2;
            for (i, (g, detail)) in copies.iter().zip(details).enumerate() {
                let plays = launch.as_deref() == Some(g.key.as_str());
                card = card.child(self.copy_row(g, detail, i == 0, group.chosen, plays, detachable, cx));
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
                // Told apart from the game it is offered for.
                let detail = lib.get(key).and_then(|own| copies::copy_details(&[&**own, &*g]).pop().flatten());
                let (key, other) = (key.to_string(), g.key.clone());
                card = card.child(
                    row_base(&g, detail).id(SharedString::from(format!("copy-link-{}", g.key))).child(
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
    /// Play / "Make default" / "Remove from the group".
    fn copy_row(
        &self,
        g: &CollectionGame,
        detail: Option<String>,
        first: bool,
        chosen: bool,
        plays: bool,
        detachable: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
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
            // Stops this copy when it runs, even if another copy runs too.
            .on_click(cx.listener(move |this, _, window, cx| this.play_copy(&key, window, cx)));
        let detach = detachable.then(|| {
            let key = g.key.clone();
            div()
                .id(SharedString::from(format!("copy-detach-{key}")))
                .flex_none()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.1)))
                .child(icon(Icon::Close, px(14.), theme::ink_a(0.55)))
                .tooltip(tooltip(tr!("Remove from the group")))
                .on_click(cx.listener(move |this, _, _, cx| this.detach_copy(&key, cx)))
        });
        row_base(g, detail).id(SharedString::from(format!("copy-{}", g.key))).child(star).child(play).children(detach)
    }
}

/// An emulated game: its copies are versions of a ROM.
fn is_rom(g: &CollectionGame) -> bool {
    g.launch_kind.as_deref() == Some("emulator")
}

/// A copy's store (its emulator for a ROM), what tells it apart from the
/// other copies (`copies::copy_details`: an edition, a region, a ROM's
/// file) and its playtime. A ROM leads with its version (romhacks of one
/// game share their emulator), wrapped rather than cut, as the hack or
/// region ends the name; its emulator goes under it.
fn row_base(g: &CollectionGame, detail: Option<String>) -> Div {
    let (store, detail) = match detail {
        Some(version) if is_rom(g) => (version, Some(copies::store_label(g).to_string())),
        detail => (copies::store_label(g).to_string(), detail),
    };
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
                    .child(div().min_w_0().text_color(theme::ink_a(0.94)).child(store))
                    .when(g.installed, |d| d.child(div().flex_none().size(px(6.)).rounded_full().bg(theme::success()))),
            )
            .child(div().truncate().text_size(px(11.5)).text_color(theme::ink_a(0.55)).child(match detail {
                Some(t) => format!("{t} · {}", g.playtime_label()),
                None => g.playtime_label(),
            })),
    )
}
