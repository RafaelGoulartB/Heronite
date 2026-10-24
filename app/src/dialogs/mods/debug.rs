//! Debug only: `HERONITE_DEBUG_MODS=<steps>` puts the manager in a state
//! that otherwise needs clicks. Steps, `,`-separated: `menu:<n>` (the menu of
//! mod n), `expand:<n>` (its conflicts), `rename:<n>`, `remove:<n>` (the
//! question), `overwrite-name`, `overwrite-clear`, `forget` (Game tab's
//! question), `running`, `thunderstore` (Browse on that site).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{Confirm, ModsDialog, RowMenu, Tab};

impl ModsDialog {
    pub(super) fn debug_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(spec) = crate::env::value("HERONITE_DEBUG_MODS") else { return };
        let Some(v) = self.snapshot(cx) else { return };
        let mod_at = |n: &str| n.parse::<usize>().ok().and_then(|n| v.g.list.mods.get(n)).map(|m| (m.id.clone(), m.name.clone()));
        for step in spec.split(',') {
            match step.split_once(':') {
                Some(("menu", n)) => {
                    if let Some((id, _)) = mod_at(n) {
                        self.menu = Some(RowMenu { id, at: point(px(3300.), px(330.)) });
                    }
                }
                Some(("expand", n)) => {
                    if let Some((id, _)) = mod_at(n) {
                        self.expanded.insert(id);
                    }
                }
                Some(("rename", n)) => {
                    if let Some((id, name)) = mod_at(n) {
                        self.start_rename(&id, &name, window, cx);
                    }
                }
                Some(("remove", n)) => {
                    if let Some((id, _)) = mod_at(n) {
                        self.confirm = Some(Confirm::Remove(id));
                    }
                }
                _ => match step {
                    "overwrite-name" => self.start_naming_overwrite(window, cx),
                    "overwrite-clear" => self.confirm = Some(Confirm::ClearOverwrite),
                    "forget" => {
                        self.tab = Tab::Game;
                        self.confirm = Some(Confirm::ForgetGame);
                    }
                    "running" => self.running = true,
                    "thunderstore" => self.browse.source = super::browse::Source::Thunderstore,
                    _ => {}
                },
            }
        }
    }
}
