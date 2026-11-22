//! "AntiMicroX profiles" in console mode: one row per player (A picks
//! another profile), then add and remove, like the desktop section.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{grouped, item, Act, Item};
use crate::console::list::Row;
use crate::console::sheet::{Sheet, SheetEvent};
use crate::dialogs::settings_panel::antimicrox::player_label;
use crate::dialogs::settings_panel::companions::file_name;
use crate::dialogs::settings_panel::SettingsPanel;
use crate::icons::Icon;

/// The files the console browser offers.
fn profile_exts() -> Vec<String> {
    vec!["amgp".into()]
}

impl SettingsPanel {
    pub(super) fn c_antimicrox(&self) -> Vec<Item> {
        let list = self.antimicrox_profiles();
        let names: Vec<String> = list.iter().map(|p| file_name(p)).collect();
        let summary = if names.is_empty() { tr!("None").to_string() } else { names.join(", ") };
        let mut v = vec![item(Row::group("grp:antimicrox", tr!("AntiMicroX profiles")).summary(summary), Act::None)];
        for (i, path) in list.iter().enumerate() {
            v.push(
                Self::c_do(&format!("antimicrox:{i}"), &player_label(i), Icon::SportsEsports, true, move |this, window, cx| {
                    let title = tr!("AntiMicroX profile of {player}", player = player_label(i));
                    this.console_pick_file(&title, profile_exts(), window, cx, move |this, path, cx| {
                        this.replace_antimicrox_profile(i, path, cx)
                    })
                })
                .desc_item(&file_name(path)),
            );
        }
        let next = player_label(list.len());
        v.push(Self::c_do("antimicrox-add", tr!("Add a player\u{2026}"), Icon::Add, true, move |this, window, cx| {
            let title = tr!("AntiMicroX profile of {player}", player = next.clone());
            this.console_pick_file(&title, profile_exts(), window, cx, |this, path, cx| this.add_antimicrox_profile(path, cx))
        }));
        if !list.is_empty() {
            v.push(Self::c_do("antimicrox-remove", tr!("Remove a player\u{2026}"), Icon::DeleteOutline, true, |this, window, cx| {
                this.console_antimicrox_remove(window, cx)
            }));
        }
        grouped(v)
    }

    fn console_antimicrox_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self
            .antimicrox_profiles()
            .iter()
            .enumerate()
            .map(|(i, path)| {
                Row::action(SharedString::from(format!("rm:{i}")), player_label(i)).desc(file_name(path)).icon(Icon::DeleteOutline).danger()
            })
            .collect();
        let sheet = cx.new(|_| Sheet::new(tr!("Remove a player"), None, rows));
        let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, _, cx| {
            let SheetEvent::Activate(id) = ev;
            this.console_layer.close();
            if let Some(i) = id.strip_prefix("rm:").and_then(|n| n.parse::<usize>().ok()) {
                this.remove_antimicrox_profile(i, cx);
            }
            cx.notify();
        });
        self.console_layer.open(&sheet, sub);
    }
}
