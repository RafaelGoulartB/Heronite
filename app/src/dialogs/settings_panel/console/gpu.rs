//! Console rows of the graphics card (see `settings_panel::gpu`).

use std::rc::Rc;

use gpui_kit::*;

use super::super::gpu::{GpuPart, GpuRow};
use super::{item, Act, Item, SettingsPanel};
use crate::console::list::Row;

impl SettingsPanel {
    /// The rows of `part`. `disabled`: a Steam game whose launch options
    /// cannot change now.
    pub(super) fn c_gpu(&self, part: GpuPart, disabled: bool) -> Vec<Item> {
        self.gpu_rows(part, disabled)
            .into_iter()
            .map(|r| match r {
                GpuRow::Toggle { key, label, desc, enabled } => self.c_toggle(key, false, label, Some(&desc), !enabled),
                GpuRow::Info { id, label, value } => Self::c_info(id, &label, value),
                GpuRow::Choice { id, label, desc, options, selected, enabled, pick } => {
                    let names = options
                        .into_iter()
                        .map(|(name, note)| SharedString::from(note.map_or(name.clone(), |n| format!("{name} \u{b7} {n}"))))
                        .collect();
                    item(
                        Row::choice(SharedString::from(format!("c:{id}")), label.to_string(), names, selected).desc(desc).enabled(enabled),
                        Act::Choice(Rc::new(move |this, i, cx| pick(this, i, cx))),
                    )
                }
            })
            .collect()
    }

    pub(super) fn c_steam_graphics(&self, cx: &App) -> Vec<Item> {
        let mut v = self.c_steam_status();
        if self.steam.as_ref().is_some_and(|s| s.editable) {
            v.extend(self.c_graphics(true, cx));
        }
        v
    }
}
