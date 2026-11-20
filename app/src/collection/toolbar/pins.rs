//! The pinned categories, manual and smart: a track like the quick
//! controls' after them (or first in the lane while the controls are off),
//! one segment per category in its colour (a smart one with the sparkle). A
//! click shows only that category's games and a second click shows every
//! game again; a right click opens the smart one's editor, or the manager
//! for a manual one. Nothing is drawn while none is pinned, and the pins
//! that do not fit wait off screen as the controls do (`layout::fit`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{segment, text_width, Widths, DOT, ICON_GAP, PRESET_LABEL_MAX, SEG_GAP, SEG_PAD, TRACK_PAD};
use crate::collection::category_kinds::CategoryRef;
use crate::collection::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tooltip};

impl CollectionView {
    /// Each pinned category's segment width, in its order.
    pub(super) fn pin_widths(&self, window: &Window) -> Vec<f32> {
        self.pinned_categories()
            .iter()
            .map(|c| SEG_PAD + DOT + ICON_GAP + text_width(&c.name, window).min(PRESET_LABEL_MAX) + SEG_PAD)
            .collect()
    }

    /// The first `shown` pinned categories.
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
        let segments = self.pinned_categories().into_iter().zip(&widths.pins).take(shown).enumerate().map(|(i, (c, w))| {
            let on = self.category_alone(&c.r);
            let mark = if c.smart() {
                icon(Icon::AutoAwesome, px(DOT + 3.), c.colour).mx(px(-1.5)).into_any_element()
            } else {
                div().flex_none().size(px(DOT)).rounded_full().bg(c.colour).into_any_element()
            };
            let (r, edit) = (c.r.clone(), c.r.clone());
            segment(("pinned-group", i), *w, on, false)
                .pl(px(SEG_PAD))
                .child(mark)
                .child(div().flex_1().min_w_0().ml(px(ICON_GAP)).mr(px(SEG_PAD)).truncate().child(c.name.clone()))
                .tooltip(tooltip(if on {
                    tr!("Show every game again").to_string()
                } else {
                    tr!("Show only “{name}”. Right click to edit it.", name = c.name.as_str()).to_string()
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.show_only_category(&r, cx)))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, window, cx| match &edit {
                        CategoryRef::Smart(id) => this.open_group_editor(Some(id), None, window, cx),
                        CategoryRef::Manual(_) => this.open_manage_categories(window, cx),
                    }),
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
