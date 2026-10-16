//! The Launch tab's actions in console mode: the list, then the editor in
//! its place (text on the on-screen keyboard, programs picked with the
//! console browser). Saved through the same code as the desktop editor.

use gpui_kit::*;
use heronite_data::game_actions::ActionKind;
use heronite_i18n::tr;

use super::actions::{play_summary, Editor, KINDS};
use super::GameSettingsDialog;
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::list::{Row, Value};
use crate::console::osk::{Osk, OskEvent};
use crate::icons::Icon;
use crate::ui::game_actions::{icon_of, kind_label, menu_note};

/// Which editor field a keyboard edit goes to.
#[derive(Clone, Copy)]
enum Text {
    Name,
    Target,
    Args,
}

impl GameSettingsDialog {
    /// "Play" for games whose start is not edited here.
    pub(super) fn console_play_row(&self) -> Row {
        Row::info("play", tr!("Play"), tr!("Default")).icon(Icon::PlayArrow).desc(play_summary(&self.game))
    }

    pub(super) fn console_action_rows(&self, cx: &App) -> Vec<Row> {
        let mut rows = vec![Row::header(tr!("Other actions"))];
        let Some(e) = self.actions.editor.as_ref() else {
            for a in &self.actions.list {
                rows.push(Row::action(SharedString::from(format!("ua:{}", a.id)), a.name.clone()).icon(icon_of(a)).desc(menu_note(a)));
            }
            rows.push(
                Row::action("ua-add", tr!("Add action"))
                    .icon(Icon::Add)
                    .desc(tr!("Extra ways to start the game, in the menu next to Play")),
            );
            return rows;
        };
        let kind = KINDS.iter().position(|k| *k == e.kind).unwrap_or(0);
        let target_label = match e.kind {
            ActionKind::Program => tr!("Program"),
            ActionKind::Command => tr!("Command"),
            ActionKind::Open => tr!("Address, file or folder"),
        };
        rows.push(Row::text("ue:name", tr!("Name"), Editor::text(&e.name, cx)).icon(Icon::Edit));
        rows.push(Row::choice("ue:kind", tr!("Type"), KINDS.iter().map(|k| SharedString::from(kind_label(*k))).collect(), kind));
        rows.push(Row::text("ue:target", target_label, Editor::text(&e.target, cx)).icon(if e.kind == ActionKind::Program {
            Icon::FolderOpen
        } else {
            Icon::Edit
        }));
        if e.kind == ActionKind::Program {
            rows.push(Row::text("ue:args", tr!("Arguments"), Editor::text(&e.args, cx)).icon(Icon::Edit));
        }
        if e.kind != ActionKind::Open {
            rows.push(
                Row::toggle("ue:plays", tr!("Counts as playing"), e.plays)
                    .desc(tr!("Starts like Play, with this game's settings; its time counts as playtime")),
            );
        }
        rows.push(Row::action("ue:save", tr!("Save")).icon(Icon::Save).enabled(e.can_save(cx) && !self.actions.saving));
        if let Some(id) = e.id.as_deref() {
            let confirming = self.actions.deleting.as_deref() == Some(id);
            let label = if confirming { tr!("Press A again to delete") } else { tr!("Delete") };
            rows.push(Row::action("ue:delete", label).icon(Icon::DeleteOutline).enabled(!self.actions.saving));
        }
        rows.push(Row::action("ue:cancel", tr!("Cancel")).icon(Icon::Close));
        rows
    }

    /// A pressed row of the actions; `false` when it is not one.
    pub(super) fn console_action_activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match id {
            "ua-add" => self.edit_action(None, window, cx),
            "ue:name" => self.console_action_text(Text::Name, tr!("Name"), window, cx),
            "ue:target" => match self.actions.editor.as_ref().map(|e| e.kind) {
                Some(ActionKind::Program) => self.console_pick_program(window, cx),
                _ => self.console_action_text(Text::Target, tr!("Address, file or folder"), window, cx),
            },
            "ue:args" => self.console_action_text(Text::Args, tr!("Arguments"), window, cx),
            "ue:save" => self.save_action(cx),
            "ue:cancel" => {
                self.close_action_editor(cx);
            }
            "ue:delete" => {
                let Some(id) = self.actions.editor.as_ref().and_then(|e| e.id.clone()) else { return true };
                if self.actions.deleting.as_deref() == Some(id.as_str()) {
                    self.delete_action(&id, cx);
                } else {
                    self.actions.deleting = Some(id);
                }
            }
            id => match id.strip_prefix("ua:") {
                Some(action) => self.edit_action(Some(action.to_string()), window, cx),
                None => return false,
            },
        }
        cx.notify();
        true
    }

    /// A changed row of the editor; `false` when it is not one.
    pub(super) fn console_action_changed(&mut self, id: &str, value: &Value, cx: &mut Context<Self>) -> bool {
        let Some(e) = self.actions.editor.as_mut() else { return false };
        match (id, value) {
            ("ue:kind", Value::Choice { index, .. }) => e.kind = KINDS[*index],
            ("ue:plays", Value::Toggle(on)) => e.plays = *on,
            _ => return false,
        }
        cx.notify();
        true
    }

    /// B in the editor goes back to the list.
    pub(super) fn close_action_editor(&mut self, cx: &mut Context<Self>) -> bool {
        if self.actions.editor.take().is_none() {
            return false;
        }
        self.actions.deleting = None;
        self.console_list.reset();
        cx.notify();
        true
    }

    fn console_action_text(&mut self, field: Text, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.actions.editor.as_ref() else { return };
        let input = match field {
            Text::Name => e.name.clone(),
            Text::Target => e.target.clone(),
            Text::Args => e.args.clone(),
        };
        let current = input.read(cx).value().to_string();
        let osk = cx.new(|_| Osk::new(title.to_string(), &current));
        let sub = cx.subscribe_in(&osk, window, move |this, _, ev: &OskEvent, window, cx| {
            this.layer.close();
            if let OskEvent::Done(text) = ev {
                input.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
            }
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    fn console_pick_program(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.actions.editor.as_ref().map(|e| e.target.clone()) else { return };
        let current = input.read(cx).value().to_string();
        let start = (!current.is_empty()).then(|| std::path::PathBuf::from(&current));
        let browser = cx.new(|cx| Browser::new(tr!("Program"), PickKind::File(Vec::new()), start.as_deref(), cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| {
            this.layer.close();
            if let BrowserEvent::Picked(p) = ev {
                input.update(cx, |s, cx| s.set_value(p.display().to_string(), window, cx));
            }
            cx.notify();
        });
        self.layer.open(&browser, sub);
    }
}
