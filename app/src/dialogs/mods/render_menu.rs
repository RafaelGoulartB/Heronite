//! The "⋯" menu of a mod, drawn at the root of the dialog where it was
//! clicked (the drawer clips its children, a menu must not be).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{Confirm, ModsDialog, View};
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel, menu_separator};
use crate::ui::icon;

const MENU_W: f32 = 210.;

impl ModsDialog {
    pub(super) fn render_row_menu(&mut self, v: &View, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let (index, m) = v.g.list.mods.iter().enumerate().find(|(_, m)| m.id == menu.id)?;
        let last = v.g.list.mods.len() - 1;
        let id = m.id.clone();
        let name = m.name.clone();
        let item = |key: &'static str, ic: Icon, label: &'static str| {
            menu_item(SharedString::from(format!("mods-menu-{key}")), false)
                .child(icon(ic, px(17.), theme::text_muted()))
                .child(div().flex_1().child(label))
        };
        let mut panel = menu_panel(MENU_W);
        if let Some(url) = m.source.page_url() {
            panel = panel.child(item("page", Icon::OpenInNew, tr!("Open mod page")).on_click(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                this.open_url(url.clone(), cx);
                cx.notify();
            })));
        }
        panel = panel.child({
            let id = id.clone();
            item("folder", Icon::FolderOpen, tr!("Open folder")).on_click(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                let dir = this.mod_dir(&id, cx);
                this.open_dir(dir, cx);
                cx.notify();
            }))
        });
        if m.archive.is_some() {
            let id = id.clone();
            panel = panel.child(item("reinstall", Icon::Refresh, tr!("Reinstall")).on_click(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                let key = this.key.clone();
                this.service.update(cx, |s, cx| s.reinstall(&key, &id, cx));
                cx.notify();
            })));
        }
        panel = panel.child({
            let id = id.clone();
            item("rename", Icon::Edit, tr!("Rename"))
                .on_click(cx.listener(move |this, _, window, cx| this.start_rename(&id, &name, window, cx)))
        });
        panel = panel.child(menu_separator());
        if index > 0 {
            let id = id.clone();
            panel = panel.child(item("up", Icon::ArrowUpward, tr!("Move up")).on_click(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                this.move_to(&id, index - 1, cx);
                cx.notify();
            })));
        }
        if index < last {
            let id = id.clone();
            panel = panel.child(item("down", Icon::ArrowDownward, tr!("Move down")).on_click(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                this.move_to(&id, index + 1, cx);
                cx.notify();
            })));
        }
        panel = panel.child(menu_separator()).child({
            let id = id.clone();
            menu_item("mods-menu-remove", false)
                .text_color(theme::danger_text())
                .child(icon(Icon::DeleteOutline, px(17.), theme::danger_text()))
                .child(div().flex_1().child(tr!("Remove")))
                .on_click(cx.listener(move |this, _, _, cx| this.ask(Confirm::Remove(id.clone()), cx)))
        });

        let at = point(menu.at.x - px(MENU_W - 24.), menu.at.y + px(8.));
        Some(
            deferred(
                div()
                    .id("mods-menu-backdrop")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }),
                    )
                    .child(anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                        div().id("mods-menu").occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(panel),
                    )),
            )
            .with_priority(40)
            .into_any_element(),
        )
    }
}
