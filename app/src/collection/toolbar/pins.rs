//! The pinned smart groups: a track like the quick controls' after them (or
//! first in the lane while the controls are off), one segment per group in
//! its colour. A click shows only that group's games and a second click
//! shows every game again; a right click opens its editor. Nothing is drawn
//! while no group is pinned, and the pins that do not fit wait off screen as
//! the controls do (`layout::fit`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{segment, text_width, Widths, DOT, ICON_GAP, PRESET_LABEL_MAX, SEG_GAP, SEG_PAD, TRACK_PAD};
use crate::collection::smart_groups::group_colour;
use crate::collection::CollectionView;
use crate::theme;
use crate::ui::tooltip;

impl CollectionView {
    /// Each pinned group's segment width, in its order.
    pub(super) fn pin_widths(&self, window: &Window) -> Vec<f32> {
        self.smart_groups.pinned().map(|g| SEG_PAD + DOT + ICON_GAP + text_width(&g.name, window).min(PRESET_LABEL_MAX) + SEG_PAD).collect()
    }

    /// The first `shown` pinned groups.
    pub(in crate::collection) fn pinned_groups(
        &self,
        shown: usize,
        widths: &Widths,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if shown == 0 {
            return None;
        }
        let alone = (self.ui.smart_groups.len() == 1).then(|| self.ui.smart_groups.first()).flatten();
        let segments = self.smart_groups.pinned().zip(&widths.pins).take(shown).enumerate().map(|(i, (g, w))| {
            let on = alone == Some(&g.id);
            let (id, edit) = (g.id.clone(), g.id.clone());
            segment(("pinned-group", i), *w, on, false)
                .pl(px(SEG_PAD))
                .child(div().flex_none().size(px(DOT)).rounded_full().bg(group_colour(g)))
                .child(div().flex_1().min_w_0().ml(px(ICON_GAP)).mr(px(SEG_PAD)).truncate().child(g.name.clone()))
                .tooltip(tooltip(if on {
                    tr!("Show every game again").to_string()
                } else {
                    tr!("Show only “{name}”. Right click to edit it.", name = g.name.as_str()).to_string()
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.show_only_group(&id, cx)))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, window, cx| this.open_group_editor(Some(&edit), None, window, cx)),
                )
        });
        Some(
            div()
                .id("collection-pinned-groups")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(SEG_GAP))
                .p(px(TRACK_PAD))
                .rounded_full()
                .bg(theme::ink_a(0.06))
                .children(segments),
        )
    }
}
