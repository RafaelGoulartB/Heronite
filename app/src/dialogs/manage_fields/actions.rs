//! The writes of the fields manager. Each runs off the UI thread, reloads the
//! names and tells the Collection (`DialogEvent::Changed` + [`NamesChanged`]).

use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use heronite_data::field_names::{self, Field};
use heronite_i18n::{tr, trn};

use super::{ManageFieldsDialog, NamesChanged, Overview};
use crate::dialogs::DialogEvent;
use crate::ui::fields::new_input;

/// One write of the dialog.
enum Op {
    Change { from: Vec<String>, to: Option<String> },
    Forget(String),
    Apart(Vec<String>),
}

impl ManageFieldsDialog {
    /// Names of the keys, as shown.
    fn names_of(&self, keys: &[String]) -> Vec<String> {
        keys.iter().filter_map(|k| self.value(k)).map(|v| v.name.clone()).collect()
    }

    fn run(&mut self, field: Field, op: Op, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let games = match &op {
                        Op::Change { from, to: Some(to) } => field_names::rename(&paths, field, from, to)?,
                        Op::Change { from, to: None } => field_names::remove(&paths, field, from)?,
                        Op::Forget(from) => {
                            field_names::forget(&paths, field, from)?;
                            0
                        }
                        Op::Apart(names) => {
                            field_names::keep_apart(&paths, field, names)?;
                            0
                        }
                    };
                    anyhow::Ok((op, games, Overview::load(&paths)?))
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match res {
                    Ok((op, games, data)) => {
                        this.data = Some(std::sync::Arc::new(data));
                        if let Op::Change { from, to } = op {
                            this.notice = Some(
                                match &to {
                                    Some(to) => trn!(games, "“{to}” is in {n} game now", "“{to}” is in {n} games now", to = to.as_str()),
                                    None => trn!(games, "Removed from {n} game", "Removed from {n} games"),
                                }
                                .into(),
                            );
                            this.selected.clear();
                            this.confirm_remove = None;
                            cx.emit(NamesChanged { field, from, to });
                        }
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Merges the selected names into the name typed under them.
    pub(super) fn merge_selected(&mut self, cx: &mut Context<Self>) {
        let to = self.merge_name.read(cx).value().trim().to_string();
        if self.selected.len() < 2 || to.is_empty() {
            return;
        }
        let from = self.names_of(&self.selected);
        self.run(self.field, Op::Change { from, to: Some(to) }, cx);
    }

    /// Merges a suggested group into its most used name.
    pub(super) fn merge_group(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        let from = self.names_of(&keys);
        let Some(to) = from.first().cloned() else { return };
        self.run(self.field, Op::Change { from, to: Some(to) }, cx);
    }

    /// The suggested group is not one name.
    pub(super) fn keep_apart(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        let names = self.names_of(&keys);
        self.run(self.field, Op::Apart(names), cx);
    }

    /// Selects the names of a suggested group, to merge them under another name.
    pub(super) fn select_group(&mut self, keys: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = keys;
        self.confirm_remove = None;
        self.sync_merge_name(window, cx);
        self.merge_name.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub(super) fn start_rename(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.value(key).map(|v| v.name.clone()) else { return };
        let input = new_input(window, cx, &name, "");
        let k = key.to_owned();
        cx.subscribe_in(&input, window, move |this, state, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                let to = state.read(cx).value().trim().to_string();
                this.finish_rename(&k, to, cx);
            }
        })
        .detach();
        input.update(cx, |s, cx| s.focus(window, cx));
        self.renaming = Some((key.to_owned(), input));
        self.confirm_remove = None;
        cx.notify();
    }

    fn finish_rename(&mut self, key: &str, to: String, cx: &mut Context<Self>) {
        if self.renaming.as_ref().is_none_or(|(k, _)| k != key) {
            return;
        }
        self.renaming = None;
        let Some(from) = self.value(key).map(|v| v.name.clone()) else { return };
        if to.is_empty() || to == from {
            cx.notify();
            return;
        }
        self.run(self.field, Op::Change { from: vec![from], to: Some(to) }, cx);
    }

    pub(super) fn ask_remove(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        self.confirm_remove = Some(keys);
        self.renaming = None;
        cx.notify();
    }

    pub(super) fn remove_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(keys) = self.confirm_remove.take() else { return };
        let from = self.names_of(&keys);
        self.run(self.field, Op::Change { from, to: None }, cx);
    }

    pub(super) fn forget(&mut self, field: Field, from: String, cx: &mut Context<Self>) {
        self.run(field, Op::Forget(from), cx);
    }

    /// Games the keys are in (a game with two of them counts twice).
    pub(super) fn games_of(&self, keys: &[String]) -> usize {
        keys.iter().filter_map(|k| self.value(k)).map(|v| v.games).sum()
    }

    pub(super) fn remove_question(&self, keys: &[String]) -> String {
        let games = self.games_of(keys);
        match self.names_of(keys).as_slice() {
            [one] => trn!(games, "Remove “{name}” from {n} game?", "Remove “{name}” from {n} games?", name = one.as_str()),
            names => tr!("Remove {count} names from their games?", count = names.len()),
        }
    }
}
