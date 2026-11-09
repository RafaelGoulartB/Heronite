//! Console rows of "Controllers and power" (Other), "Game fixes" (Wine) and
//! "Presets" (Advanced); see `settings_panel::{other, fixes, presets}`.

use std::rc::Rc;

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::umu_ids;

use super::super::logic;
use super::super::presets::summary;
use super::{item, Act, Item, SettingsPanel};
use crate::console::list::Row;
use crate::console::sheet::{Sheet, SheetEvent};
use crate::icons::Icon;

impl SettingsPanel {
    /// GameMode, keeping the computer awake, controllers. `steam`: a Steam
    /// game (Steam keeps the computer awake itself).
    pub(super) fn c_system(&self, steam: bool) -> Vec<Item> {
        let disabled = steam && !self.steam_fields_editable();
        let mut v = vec![
            Self::c_header(tr!("System")),
            self.c_toggle("useGameMode", false, "GameMode", Some(tr!("System tweaks while the game runs (needs GameMode)")), disabled),
        ];
        if !steam {
            v.push(self.c_toggle(
                heronite_launch::keep_awake::KEY,
                true,
                tr!("Keep the computer awake"),
                Some(tr!("A controller does not count as activity, so the computer could sleep mid-game")),
                false,
            ));
        }
        if steam || (!self.native() && self.wine_version().kind == "proton") {
            v.push(self.c_toggle(
                heronite_launch::env::KEY_PREFER_SDL,
                false,
                tr!("Read controllers through SDL"),
                Some(tr!("Fixes a controller the game misses or buttons that repeat; PlayStation extras (gyro, touchpad) stop working")),
                disabled,
            ));
        }
        v
    }

    pub(super) fn c_fixes(&self, cx: &App) -> Vec<Item> {
        if !self.shows_game_fixes() {
            return Vec::new();
        }
        let mut field = self.c_text("umuId", tr!("umu ID"), cx);
        field.row = field.row.desc(self.umu_status());
        let mut v = vec![Self::c_header(tr!("Game fixes")), field];
        for (i, e) in self.umu_suggestions().into_iter().enumerate() {
            let id = e.umu_id.clone();
            v.push(item(
                Row::action(SharedString::from(format!("d:umu-suggest-{i}")), tr!("Use {id} ({title})", id = e.umu_id, title = e.title))
                    .icon(Icon::Check),
                Act::Do(Rc::new(move |this, window, cx| this.use_umu_id(&id, window, cx))),
            ));
        }
        v.push(Self::c_do("umu-site", tr!("Search the umu database"), Icon::OpenInNew, true, |_, _, _| {
            crate::ui::open_external(umu_ids::WEBSITE)
        }));
        v
    }

    pub(super) fn c_presets(&self, steam: bool) -> Vec<Item> {
        let disabled = steam && !self.steam_fields_editable();
        let mut v = vec![Self::c_header(tr!("Presets"))];
        if !self.presets.is_empty() {
            let mut opts: Vec<SharedString> = vec![tr!("Choose a preset\u{2026}").into()];
            opts.extend(self.presets.iter().map(|p| SharedString::from(format!("{} \u{b7} {}", p.name, summary(p)))));
            v.push(item(
                Row::choice("c:preset-apply", tr!("Apply a preset"), opts, 0).enabled(!disabled).desc(if logic::wrappers() {
                    tr!("Adds its variables, wrappers and arguments to this game")
                } else {
                    tr!("Adds its variables and arguments to this game")
                }),
                Act::Choice(Rc::new(|this, i, cx| {
                    if i > 0 {
                        this.apply_preset(i - 1, cx);
                    }
                })),
            ));
        }
        v.push(item(
            Row::action("d:preset-save", tr!("Save as preset\u{2026}")).icon(Icon::Save).enabled(!disabled).desc(if logic::wrappers() {
                tr!("This game's variables, wrappers and arguments, for other games")
            } else {
                tr!("This game's variables and arguments, for other games")
            }),
            Act::Do(Rc::new(|this, window, cx| {
                this.console_osk(tr!("Preset name"), "", window, cx, |this, name, _, cx| this.save_preset(&name, cx))
            })),
        ));
        if !self.presets.is_empty() {
            v.push(Self::c_do("preset-delete", tr!("Delete a preset\u{2026}"), Icon::DeleteOutline, true, |this, window, cx| {
                this.console_preset_delete(window, cx)
            }));
        }
        v
    }

    fn console_preset_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self
            .presets
            .iter()
            .enumerate()
            .map(|(i, p)| Row::action(SharedString::from(format!("rm:{i}")), p.name.clone()).icon(Icon::DeleteOutline).danger())
            .collect();
        let names: Vec<String> = self.presets.iter().map(|p| p.name.clone()).collect();
        let sheet = cx.new(|_| Sheet::new(tr!("Delete a preset"), None, rows));
        let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, _, cx| {
            let SheetEvent::Activate(id) = ev;
            this.console_layer.close();
            if let Some(name) = id.strip_prefix("rm:").and_then(|n| n.parse::<usize>().ok()).and_then(|i| names.get(i)) {
                this.delete_preset(name.clone(), cx);
            }
            cx.notify();
        });
        self.console_layer.open(&sheet, sub);
    }
}
