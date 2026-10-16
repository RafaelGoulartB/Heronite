//! Console rows of the "GPU" tab (see `settings_panel::gpu`).

use std::rc::Rc;

use gpui_kit::*;

use super::super::gpu::GpuRow;
use super::{item, Act, Item, SettingsPanel};
use crate::console::list::Row;

impl SettingsPanel {
    /// `steam`: a Steam game, whose rows follow the launch options' lock.
    pub(super) fn c_gpu(&self, steam: bool) -> Vec<Item> {
        let disabled = steam && !self.steam_fields_editable();
        self.gpu_rows(disabled)
            .into_iter()
            .map(|r| match r {
                GpuRow::Header(text) => Self::c_header(text),
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

    pub(super) fn c_steam_gpu(&self) -> Vec<Item> {
        let mut v = self.c_steam_status();
        if self.steam.as_ref().is_some_and(|s| s.editable) {
            v.extend(self.c_gpu(true));
        }
        v
    }
}
