//! The grid view's body and its drawer: the selected game in a glass sheet on
//! the right, with the art on top, the cover next to the title, the controls
//! of the focus panel's left column, the play numbers and the details in one
//! scrolling column. "Open game page" goes back to the Focus view.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_images::Variant;

use super::grid::{drawer_width, DRAWER_INSET};
use super::vm::FocusVm;
use super::{focus, CollectionView};
use crate::theme;
use crate::ui::controls::{button, close_button, ButtonKind};
use heronite_i18n::tr;

const PAD: f32 = 22.;
const COVER_W: f32 = 112.;
/// How far the cover reaches up into the art.
const COVER_OVERLAP: f32 = 96.;

impl CollectionView {
    /// The grid across the body, with the drawer over its right side.
    pub(super) fn render_grid_body(&mut self, grid_w: f32, vw: f32, vh: f32, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let list = self.render_list_column(grid_w, cx);
        let drawer = if self.drawer_open() { self.render_grid_drawer(drawer_width(vw), vh, scale, cx) } else { None };
        div().relative().size_full().flex().child(list).children(drawer).into_any_element()
    }

    fn render_grid_drawer(&mut self, width: f32, vh: f32, scale: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let vm = self.with_folder_rows(self.focus.clone()?);
        let inner_w = width - 2. * PAD;
        // Shorter art on short windows, so the controls stay in sight.
        let hero_h = (vh * 0.22).clamp(150., 220.);
        let head = self.drawer_head(&vm, width, hero_h, scale, cx);

        let epic = self.epic_focus(&vm.key, vm.installed, cx);
        let play = self.play_area(&vm, epic, cx);
        let status = self.focus_status_row(&vm, vh, cx);
        let numbers = self.drawer_numbers(&vm);
        let rating = self.rating_row(&vm.key, cx);
        let facts = self.facts_card(&vm, cx);

        let covers = self.focus_covers(&vm, scale);
        let html_images = self.focus_html_images(&vm, inner_w, scale);
        let screenshots = self.screenshots_section(cx);
        let timeline = self.timeline_section(&vm.key, inner_w, cx);
        let extras = self.focus_extras(&vm.key, scale);
        let details = focus::stacked_details(
            focus::StackedProps {
                vm,
                cover_for: Rc::new(move |k| covers.get(k).cloned()),
                html_images: Rc::new(move |u| html_images.get(u).cloned()),
                width: inner_w,
                active_facets: self.active_facets(),
                screenshots,
                timeline,
                extras,
            },
            self.focus_actions(cx),
        );
        let open_page = button("grid-open-page", ButtonKind::Glass, tr!("Open game page"))
            .on_click(cx.listener(|this, _, _, cx| this.set_grid_view(false, cx)));

        let scroll = self.focus_scroll.clone();
        let scrollbar = crate::ui::scrollbar_draggable(
            "grid-drawer-scrollbar",
            scroll.offset().y,
            scroll.max_offset().y,
            scroll.bounds().size.height,
            {
                let (scroll, weak) = (scroll.clone(), cx.entity().downgrade());
                move |distance, _, cx| {
                    scroll.set_offset(point(px(0.), px(-distance)));
                    weak.update(cx, |_, cx| cx.notify()).ok();
                }
            },
            None,
        );
        let body = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("grid-drawer-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .px(px(PAD))
                    .pt(px(18.))
                    .pb(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(play)
                            .child(status)
                            .child(numbers)
                            .child(rating)
                            .child(facts)
                            .child(div().mt(px(14.)).child(details))
                            .child(div().mt(px(10.)).flex().justify_center().child(open_page)),
                    ),
            )
            .children(scrollbar);

        let panel = crate::ui::glass::glass_panel(theme::R_DRAWER)
            .id("grid-drawer")
            .occlude()
            .absolute()
            .top_0()
            .right(px(DRAWER_INSET))
            .bottom(px(DRAWER_INSET))
            .w(px(width))
            .flex()
            .flex_col()
            .text_color(theme::text_primary())
            .child(head)
            .child(body);
        Some(if theme::animations() {
            panel
                .with_animation(
                    "grid-drawer-in",
                    Animation::new(std::time::Duration::from_millis(220)).with_easing(ease_in_out),
                    |el, t| el.opacity(t).right(px(DRAWER_INSET - 28. * (1. - t))),
                )
                .into_any_element()
        } else {
            panel.into_any_element()
        })
    }

    /// The game's art with a close button, then the cover reaching into it
    /// next to the title and the meta line.
    fn drawer_head(&mut self, vm: &FocusVm, width: f32, hero_h: f32, scale: f32, cx: &mut Context<Self>) -> Div {
        let r = px(theme::R_DRAWER);
        let hero = self.drawer_art(&vm.key, width, hero_h, scale);
        let cover_h = COVER_W * self.cover_aspect;
        let cover = self.cover(&vm.cover_key, COVER_W, cover_h, scale, 200);
        let fade = theme::rgb_a(16, 20, 27, 0.96);
        div()
            .relative()
            .flex_none()
            .w_full()
            .h(px(hero_h + cover_h - COVER_OVERLAP))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(hero_h))
                    .rounded_t(r)
                    .bg(theme::white_a(0.04))
                    .children(hero.map(|src| img(src).size_full().rounded_t(r).object_fit(ObjectFit::Cover)))
                    .child(div().absolute().inset_0().rounded_t(r).bg(linear_gradient(
                        180.,
                        linear_color_stop(theme::rgb_a(16, 20, 27, 0.), 0.35),
                        linear_color_stop(fade, 1.),
                    ))),
            )
            .child(close_button("grid-drawer-close").absolute().top(px(12.)).right(px(12.)).bg(theme::black_a(0.45)).on_click(cx.listener(
                |this, _, _, cx| {
                    this.grid.drawer_closed = true;
                    cx.notify();
                },
            )))
            .child(
                div()
                    .absolute()
                    .top(px(hero_h - COVER_OVERLAP))
                    .left(px(PAD))
                    .right(px(PAD))
                    .flex()
                    .items_end()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex_none()
                            .w(px(COVER_W))
                            .h(px(cover_h))
                            .rounded(px(10.))
                            .bg(theme::white_a(0.06))
                            .shadow(vec![BoxShadow {
                                color: theme::black_a(0.55),
                                offset: point(px(0.), px(14.)),
                                blur_radius: px(30.),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                            .children(cover.map(|src| img(src).size_full().rounded(px(10.)).object_fit(ObjectFit::Cover))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .pb(px(2.))
                            .gap(px(6.))
                            .child(
                                div()
                                    .font_family(theme::FONT_HEADING)
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(22.))
                                    .line_height(px(26.))
                                    .line_clamp(3)
                                    .text_color(white())
                                    .child(vm.title.clone()),
                            )
                            .when(!vm.meta_line.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(12.5))
                                        .line_height(px(17.))
                                        .line_clamp(2)
                                        .text_color(theme::white_a(0.7))
                                        .child(vm.meta_line.clone()),
                                )
                            }),
                    ),
            )
    }

    /// The selected game's hero art, sharp, cut to the drawer's top.
    fn drawer_art(&mut self, key: &SharedString, w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("d:{key}:{pw}x{ph}").into();
        let chains = &self.stage_chains;
        self.images
            .get_or_request(&id, || chains.get(key).cloned().unwrap_or_default(), pw, ph, Variant::Cover, 300)
            .map(ImageSource::Render)
    }

    /// Played, last played and sessions, as three tiles.
    fn drawer_numbers(&mut self, vm: &FocusVm) -> Div {
        let sessions = self.timeline_days(&vm.key).map(|days| days.iter().map(|d| d.sessions).sum::<usize>().to_string());
        let last = vm.last_played.clone().unwrap_or_else(|| tr!("Never").into());
        div()
            .flex()
            .gap(px(8.))
            .child(number_tile(tr!("Played"), vm.playtime.clone()))
            .child(number_tile(tr!("Last played"), last))
            .child(number_tile(tr!("Sessions"), sessions.unwrap_or_else(|| "—".into()).into()))
    }
}

fn number_tile(label: &'static str, value: SharedString) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(3.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(12.))
        .bg(theme::white_a(0.05))
        .border_1()
        .border_color(theme::white_a(0.08))
        .child(div().truncate().text_size(px(11.5)).text_color(theme::white_a(0.52)).child(label))
        .child(
            div()
                .truncate()
                .font_family(theme::FONT_HEADING)
                .font_weight(theme::W_BOLD)
                .text_size(px(15.))
                .text_color(white())
                .child(value),
        )
}
