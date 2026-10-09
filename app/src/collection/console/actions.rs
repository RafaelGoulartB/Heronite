//! Console versions of the game actions that open a dialog on the desktop
//! (edit playtime, game settings, add game): sheets and pages driven by the
//! controller.

use gpui_kit::*;
use heronite_data::playtime::{set_playtime_minutes, split_minutes};

use super::super::CollectionView;
use crate::console::list::Row;
use crate::console::sheet::{Sheet, SheetEvent};

impl CollectionView {
    /// Hours and minutes with ◀ ▶, then Save.
    pub(super) fn console_edit_playtime(&mut self, key: &SharedString, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let (h, m) = split_minutes(g.playtime_minutes);
        let rows = vec![
            Row::number("hours", "Hours", h as i64, 0, 99_999, 1, " h"),
            Row::number("minutes", "Minutes", m as i64, 0, 59, 1, " min"),
            Row::action("save", "Save"),
        ];
        let sheet = cx.new(|_| Sheet::new(format!("Playtime \u{b7} {}", g.title), None, rows));
        let app_name = g.app_name.clone();
        let sub = cx.subscribe(&sheet, move |this, sheet, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            if id.as_ref() == "save" {
                let s = sheet.read(cx);
                let minutes = s.number("hours") * 60 + s.number("minutes");
                if let Err(err) = set_playtime_minutes(&this.heroic, &app_name, minutes as f64) {
                    this.show_toast(format!("Could not save the playtime: {err:#}"), true, cx);
                }
                this.close_console_overlay(cx);
                this.reload(cx);
            }
        });
        self.open_console_overlay(&sheet, sub, cx);
    }

    pub(super) fn console_game_settings(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(g)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
        let tab = crate::dialogs::game_settings::Tab::heroic_start(&g);
        let view = cx.new(|cx| crate::dialogs::game_settings::GameSettingsDialog::new(ctx, g, tab, window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn console_add_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_add_game(window, cx);
    }
}
