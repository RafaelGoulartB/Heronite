//! Plugins tab (Bethesda games): the `.esp` / `.esm` / `.esl` of the enabled
//! mods in load order, each with a switch.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{ModsDialog, View};
use crate::theme;
use crate::ui::controls::{caption, card, row, toggle};

impl ModsDialog {
    pub(super) fn render_plugins(&mut self, v: &View, cx: &mut Context<Self>) -> Div {
        let mut col = div().flex().flex_col().gap(px(4.)).child(
            div().mx(px(6.)).mb(px(6.)).text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(tr!(
                "The plugins of the mods that are on, in load order. They are written to Plugins.txt when the game starts, and the file is put back when it ends."
            )),
        );
        let mut any = false;
        for m in v.g.list.enabled().filter(|m| !m.plugins.is_empty()) {
            any = true;
            let rows = m.plugins.iter().map(|plugin| {
                let off = v.g.list.disabled_plugins.iter().any(|d| d.eq_ignore_ascii_case(plugin));
                let name = plugin.clone();
                let sw = toggle(SharedString::from(format!("mods-plugin-{}-{plugin}", m.id)), !off, false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        let key = this.key.clone();
                        this.service.update(cx, |s, cx| s.set_plugin(&key, &name, off, cx));
                    },
                ));
                row(plugin.clone(), None, sw).into_any_element()
            });
            col = col.child(caption(m.name.clone())).child(card(rows));
        }
        if !any {
            col = col.child(
                div()
                    .py(px(24.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(theme::text_dim())
                    .child(tr!("None of the mods that are on has plugins (.esp, .esm or .esl files).")),
            );
        }
        col
    }
}
