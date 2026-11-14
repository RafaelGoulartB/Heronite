//! Drawing the palette: rows, filter pills, the crumb of a game's actions,
//! the footer and the panel over its backdrop.

use std::time::Duration;

use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::launcher;
use super::super::rank::{Filter, ItemRef};
use super::super::sources::Act;
use super::preview::{PANE_W, THUMB_H, THUMB_W};
use super::{Mode, PaletteEvent, PaletteView, CAPTION_H, FOOTER_H, INPUT_H, LIST_MAX_H, LIST_PAD, PANEL_W, ROW_H, TOP};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::kbd;
use crate::ui::glass::glass_panel;
use crate::ui::icon;

impl PaletteView {
    /// Whether the preview pane fits beside the list (a narrow window keeps
    /// the list alone).
    fn shows_pane(&self, window: &Window) -> bool {
        self.mode == Mode::Launcher || f32::from(crate::ui::content_bounds(window).size.width) >= PANEL_W + PANE_W + 48.
    }

    /// What Enter does on the selected item, for the footer.
    fn enter_label(&self) -> &'static str {
        let Some(e) = self.selected_entry() else { return tr!("open") };
        if self.game.is_some() && self.mode == Mode::InApp {
            return tr!("run");
        }
        if self.mode == Mode::InApp {
            return tr!("open");
        }
        match (&e.act, &e.game) {
            (Act::SelectGame(_), Some(b)) if b.playing => tr!("stop"),
            (Act::SelectGame(_), Some(b)) if b.installed => tr!("play"),
            (Act::SelectGame(_), _) => tr!("show in Heronite"),
            (Act::GameActions(_), _) => tr!("open"),
            _ if !launcher::runs_alone(e) => tr!("open in Heronite"),
            _ => tr!("run"),
        }
    }

    fn row(&mut self, n: usize, item: ItemRef, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        // A game of the list shows its cover where the others have an icon.
        let cover_key = match &self.entry(item) {
            e if e.game.is_some() => match &e.act {
                Act::SelectGame(k) => Some(k.clone()),
                _ => None,
            },
            _ => None,
        };
        let thumb = cover_key.map(|k| self.cover(&k, THUMB_W, THUMB_H, window.scale_factor(), 100, cx));
        let e = self.entry(item);
        let sel = n == self.selected;
        let off = e.disabled.is_some();
        // A game of the main list opens its actions from the chevron too.
        let chevron = (self.game.is_none() && matches!(e.act, Act::SelectGame(_))).then(|| {
            div()
                .id(SharedString::from(format!("pal-more-{n}")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.1)))
                .child(icon(Icon::ChevronRight, px(16.), if sel { theme::text_primary() } else { theme::text_dim() }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.selected = n;
                    this.open_selected_game(cx);
                }))
        });
        let icon_color = if off {
            theme::text_dim()
        } else if sel {
            theme::accent()
        } else {
            theme::text_muted()
        };
        div()
            .id(SharedString::from(format!("pal-row-{n}")))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(ROW_H))
            .px(px(12.))
            .rounded(px(theme::R_ITEM))
            .when(!off, |d| d.cursor_pointer())
            .when(sel, |d| d.bg(if off { theme::menu_item_hover() } else { theme::accent_tint() }))
            .when(!sel, |d| d.hover(|s| s.bg(theme::menu_item_hover())))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.selected != n {
                    this.selected = n;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = n;
                this.activate(cx);
            }))
            .when(sel && !off, |d| {
                d.child(div().absolute().left(px(0.)).top(px(8.)).bottom(px(8.)).w(px(2.)).rounded_full().bg(theme::accent()))
            })
            .child(match thumb {
                Some(src) => div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(THUMB_W))
                    .h(px(THUMB_H))
                    .my(px(-6.))
                    .rounded(px(4.))
                    .overflow_hidden()
                    .bg(theme::ink_a(0.08))
                    .when(src.is_none(), |d| d.child(icon(e.icon, px(13.), icon_color)))
                    .children(src.map(|src| img(src).size_full().object_fit(ObjectFit::Cover)))
                    .into_any_element(),
                None => icon(e.icon, px(16.), icon_color).into_any_element(),
            })
            .child(
                div()
                    .flex_none()
                    .max_w(px(340.))
                    .truncate()
                    .text_size(px(14.))
                    .text_color(if off { theme::text_dim() } else { theme::text_primary() })
                    .child(e.title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(if off { theme::text_muted() } else { theme::text_dim() })
                    .child(e.disabled.clone().unwrap_or_else(|| e.subtitle.clone())),
            )
            .when(self.mode == Mode::Launcher && !launcher::runs_alone(e), |d| d.child(icon(Icon::OpenInNew, px(13.), theme::text_dim())))
            .children(e.shortcut.map(kbd))
            .children(chevron)
    }

    fn filter_pills(&self, cx: &mut Context<Self>) -> Div {
        let mut bar = div().flex().flex_none().items_center().gap(px(2.)).p(px(2.)).rounded_full().bg(theme::seg_track());
        for f in [Filter::All, Filter::Pages, Filter::Actions, Filter::Games] {
            let on = f == self.filter;
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("pal-filter-{}", f.label())))
                    .px(px(9.))
                    .py(px(2.))
                    .rounded_full()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(if on { theme::text_bright() } else { Hsla::from(theme::seg_text()) })
                    .when(on, |d| d.bg(theme::seg_active()))
                    .when(!on, |d| d.hover(|s| s.text_color(theme::text_bright())))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_filter(f, cx)))
                    .child(f.label()),
            );
        }
        bar
    }

    /// The game whose actions are listed, before the input; a click goes back.
    fn crumb(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let g = self.game.as_ref()?;
        Some(
            div()
                .id("pal-crumb")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .max_w(px(220.))
                .pl(px(10.))
                .pr(px(6.))
                .py(px(3.))
                .rounded_full()
                .cursor_pointer()
                .bg(theme::accent_tint())
                .hover(|s| s.bg(theme::menu_item_hover()))
                .text_size(px(12.5))
                .text_color(theme::text_primary())
                .child(div().min_w_0().truncate().child(g.title.clone()))
                .child(icon(Icon::ChevronRight, px(14.), theme::text_muted()))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.back(window, cx);
                })),
        )
    }
}

fn hint(keys: Vec<&'static str>, label: &'static str) -> Div {
    div().flex().items_center().gap(px(6.)).child(div().flex().gap(px(3.)).children(keys.into_iter().map(kbd))).child(label)
}

impl Render for PaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drop_evicted_covers(window);
        let pane = self.shows_pane(window);
        let mut list = div()
            .id("palette-list")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(PANEL_W))
            .px(px(8.))
            .py(px(LIST_PAD))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        // Beside the pane the list keeps its height, so the panel does not
        // change size as the results do.
        list = if pane { list.h(px(LIST_MAX_H)) } else { list.max_h(px(LIST_MAX_H)) };
        if self.results.count() == 0 {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(64.))
                    .text_size(px(13.))
                    .text_color(theme::text_dim())
                    .child(if self.query.trim().is_empty() { tr!("Nothing to show here") } else { tr!("No results") }),
            );
        }
        let mut n = 0;
        let groups: Vec<_> = self.results.groups.iter().map(|g| (g.caption.clone(), g.items.clone())).collect();
        for (caption, items) in groups {
            list = list.child(
                div()
                    .flex()
                    .flex_none()
                    .items_end()
                    .h(px(CAPTION_H))
                    .px(px(12.))
                    .pb(px(4.))
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child(caption),
            );
            for item in items {
                list = list.child(self.row(n, item, window, cx));
                n += 1;
            }
        }

        let input_row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(INPUT_H))
            .px(px(18.))
            .border_b_1()
            .border_color(theme::ink_a(0.08))
            .child(icon(Icon::FaSearch, px(15.), theme::text_muted()))
            .children(self.crumb(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(15.))
                    .text_color(theme::text_primary())
                    .child(Input::new(&self.input).appearance(false)),
            )
            .child(kbd("Esc"));

        let has_game = self.game.is_none() && self.selected_entry().is_some_and(|e| e.game_key().is_some());
        let mut footer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(18.))
            .px(px(16.))
            .h(px(FOOTER_H))
            .border_t_1()
            .border_color(theme::ink_a(0.08))
            .text_size(px(12.))
            .text_color(theme::text_dim())
            .child(hint(vec!["\u{2191}", "\u{2193}"], tr!("move")))
            .child(hint(vec!["\u{21b5}"], self.enter_label()))
            .when(
                self.mode == Mode::Launcher && self.selected_entry().is_some_and(|e| e.game_key().is_some() && launcher::runs_alone(e)),
                |f| f.child(hint(vec!["\u{21e7}", "\u{21b5}"], tr!("show in Heronite"))),
            );
        footer = if self.game.is_some() {
            footer.child(hint(vec!["Esc"], tr!("back"))).child(div().flex_1())
        } else {
            footer
                .when(has_game, |f| f.child(hint(vec!["\u{2192}"], tr!("actions"))))
                .child(hint(vec!["Tab"], tr!("filter")))
                .child(div().flex_1())
                .child(self.filter_pills(cx))
        };

        let preview = if pane { self.preview(window, cx) } else { div() };
        let panel = glass_panel(theme::R_POPOVER)
            .id("palette")
            .occlude()
            .flex()
            .flex_col()
            .w(px(if pane { PANEL_W + PANE_W } else { PANEL_W }))
            .when(self.mode == Mode::InApp, |d| d.max_h(relative(0.9)))
            .when(self.mode == Mode::Launcher, |d| d.size_full().shadow(Vec::new()))
            .font_family(theme::font_body())
            .text_color(theme::text_primary())
            // Clicks on the panel keep the input focused and do not close it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.input.update(cx, |s, cx| s.focus(window, cx));
                }),
            )
            .child(input_row)
            .child(div().flex().flex_none().min_h_0().child(list).when(pane, |d| d.child(preview)))
            .child(footer);

        let slide = self.mode == Mode::InApp;
        let panel = if theme::animations() {
            panel
                .with_animation("palette-open", Animation::new(Duration::from_millis(140)).with_easing(ease_out_quint()), move |el, t| {
                    // The launcher's window is the panel: it fades in place.
                    let el = el.opacity(t);
                    if slide {
                        el.mt(px(-10. * (1. - t)))
                    } else {
                        el
                    }
                })
                .into_any_element()
        } else {
            panel.into_any_element()
        };

        if self.mode == Mode::Launcher {
            // The window is the panel's size: nothing is drawn around it.
            return div().id("palette-backdrop").size_full().child(panel).into_any_element();
        }
        deferred(
            div()
                .id("palette-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(TOP))
                .bg(theme::glass_backdrop_veil())
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
                .child(panel),
        )
        .with_priority(40)
        .into_any_element()
    }
}
