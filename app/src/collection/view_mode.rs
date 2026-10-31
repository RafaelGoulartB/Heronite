//! The header's Focus / Grid switch. A solid pill slides under the active
//! mode, which carries a chevron: clicking it opens the mode's options (the
//! Focus list as covers or rows; the grid's cover size and titles), clicking
//! the other mode switches to it. The choices are kept in `ui-state.json`.

use std::f32::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ui_state::GRID_COVER_W;

use super::header::Menu;
use super::CollectionView;
use crate::console::motion::mix;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::toggle;
use crate::ui::dropdown::{menu_panel, menu_separator};
use crate::ui::icon;
use heronite_i18n::tr;

/// Slightly underdamped: the pill settles in about 250ms with a hint of
/// overshoot. Only the switch requests frames, and only while it moves.
const SLIDE: SpringConfig = SpringConfig::new(520., 40., 1.);
/// Critically damped turn of the chevron (about 150ms).
const TURN: SpringConfig = SpringConfig::new(900., 60., 1.);

const TRACK_PAD: f32 = 3.;
const SEG_GAP: f32 = 2.;
const SEG_H: f32 = 26.;
const SEG_PAD: f32 = 11.;
const ICON: f32 = 15.;
const ICON_GAP: f32 = 6.;
const LABEL: f32 = 12.5;
const CARET: f32 = 12.;
/// The chevron and the 4px before it.
const CARET_SLOT: f32 = 16.;
/// How much wider the active mode is: its chevron, with 8px after it
/// instead of the 11px padding.
const CARET_EXTRA: f32 = CARET_SLOT - SEG_PAD + 8.;
const MENU_W: f32 = 248.;

/// One half of the switch, measured once per render (the label depends on
/// the language and the font).
struct Mode {
    id: &'static str,
    ic: Icon,
    label: &'static str,
    /// Width without the chevron.
    base: f32,
}

impl Mode {
    fn new(id: &'static str, ic: Icon, label: &'static str, window: &Window) -> Self {
        let base = SEG_PAD + ICON + ICON_GAP + label_width(label, window) + SEG_PAD;
        Self { id, ic, label, base }
    }

    /// Width when `a` of the chevron shows (0 = inactive, 1 = active).
    fn width(&self, a: f32) -> f32 {
        self.base + CARET_EXTRA * a.max(0.)
    }
}

/// Width of `text` in the switch's type, a pixel more for the rounding of
/// the laid-out text.
fn label_width(text: &'static str, window: &Window) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: font(theme::font_body()),
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(SharedString::new_static(text), px(LABEL), &[run], None);
    f32::from(line.width).ceil() + 1.
}

impl CollectionView {
    pub(super) fn view_mode_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let grid = self.ui.grid_view;
        let open = self.open_menu == Some(Menu::Display);
        let focus = Mode::new("view-focus", Icon::ViewSidebar, tr!("Focus"), window);
        let grid_mode = Mode::new("view-grid", Icon::GridView, tr!("Grid"), window);
        // The menu lines up with the right edge of the active mode.
        let menu_right = if grid { TRACK_PAD } else { TRACK_PAD + grid_mode.base + SEG_GAP };
        let menu = open.then(|| self.view_options(cx));

        let on_focus = cx.listener(|this, _: &ClickEvent, _, cx| this.pick_view_mode(false, cx));
        let on_grid = cx.listener(|this, _: &ClickEvent, _, cx| this.pick_view_mode(true, cx));
        // A press on the active mode while its menu is open closes it: the
        // menu's click-outside must not close it first (the click would
        // open it again). Capture runs back to front, so before the menu's.
        let press = Rc::new(cx.listener(|this, _: &MouseDownEvent, _, _| this.grid.menu_press = this.open_menu == Some(Menu::Display)));

        let track =
            div().id("view-mode").relative().flex().items_center().gap(px(SEG_GAP)).p(px(TRACK_PAD)).rounded_full().bg(theme::ink_a(0.06));
        // `t`: 0 = Focus, 1 = Grid; past either end while it overshoots.
        let build = move |track: Stateful<Div>, t: f32| {
            let (a_focus, a_grid) = ((1. - t).max(0.), t.max(0.));
            let (w_focus, w_grid) = (focus.width(a_focus), grid_mode.width(a_grid));
            let pill = div()
                .absolute()
                .top(px(TRACK_PAD))
                .left(px(TRACK_PAD + t * (w_focus + SEG_GAP)))
                .w(px(w_focus + (w_grid - w_focus) * t))
                .h(px(SEG_H))
                .rounded_full()
                .bg(theme::solid_pill());
            let segment = |mode: &Mode, a: f32, w: f32, active: bool| {
                let press = press.clone();
                mode_segment(mode, a, w, active, open).when(active, |d| d.capture_any_mouse_down(move |e, w, cx| press(e, w, cx)))
            };
            track
                .child(pill)
                .child(segment(&focus, a_focus, w_focus, !grid).on_click(on_focus))
                .child(segment(&grid_mode, a_grid, w_grid, grid).on_click(on_grid))
        };
        let switch = if theme::animations() {
            track
                .with_spring("view-mode-slide", SpringAnimation::new(SLIDE).to(grid).with_epsilon(0.002), move |el, phase| {
                    build(el, phase.0)
                })
                .into_any_element()
        } else {
            build(track, if grid { 1. } else { 0. }).into_any_element()
        };

        div().relative().flex_none().mr(px(6.)).child(switch).children(menu.map(|menu| {
            deferred(div().absolute().top(px(SEG_H + 2. * TRACK_PAD + 6.)).right(px(menu_right)).occlude().child(menu)).with_priority(10)
        }))
    }

    /// A click on the active mode opens or closes its menu; on the other
    /// mode, switches to it.
    fn pick_view_mode(&mut self, grid: bool, cx: &mut Context<Self>) {
        let open = self.open_menu == Some(Menu::Display);
        if self.ui.grid_view == grid {
            self.open_menu = if open { None } else { Some(Menu::Display) };
            cx.notify();
            return;
        }
        if open {
            self.open_menu = None;
        }
        self.set_grid_view(grid, cx);
    }

    /// The active mode's options, fading in as they drop 6px into place.
    fn view_options(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let body = if self.ui.grid_view { self.grid_options(cx) } else { self.focus_options(cx) };
        let panel = div().relative().child(
            menu_panel(MENU_W)
                .w(px(MENU_W))
                .p(px(12.))
                .gap(px(10.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if std::mem::take(&mut this.grid.menu_press) {
                        return;
                    }
                    if this.open_menu == Some(Menu::Display) {
                        this.open_menu = None;
                        cx.notify();
                    }
                }))
                .child(body),
        );
        if !theme::animations() {
            return panel.into_any_element();
        }
        panel
            .with_animation("view-options-in", Animation::new(Duration::from_millis(180)).with_easing(ease_out_quint()), |el, t| {
                el.opacity(t).top(px(-6. * (1. - t)))
            })
            .into_any_element()
    }

    fn focus_options(&mut self, cx: &mut Context<Self>) -> Div {
        let rows = self.ui.focus_list;
        div().flex().flex_col().gap(px(8.)).child(section_label(tr!("Game list"))).child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    list_option("list-covers", tr!("Covers"), false, !rows)
                        .on_click(cx.listener(|this, _, _, cx| this.set_focus_list(false, cx))),
                )
                .child(
                    list_option("list-rows", tr!("List"), true, rows).on_click(cx.listener(|this, _, _, cx| this.set_focus_list(true, cx))),
                ),
        )
    }

    fn grid_options(&mut self, cx: &mut Context<Self>) -> Div {
        let slider = self.grid_size_slider(cx);
        let titles = self.ui.grid_titles;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section_label(tr!("Cover size")))
                    .child(div().text_size(px(11.5)).text_color(theme::text_muted()).child(format!("{} px", self.ui.grid_cover_w.round()))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(2.))
                    .child(icon(Icon::GridView, px(12.), theme::text_muted()))
                    .child(div().flex_1().child(Slider::new(&slider).bg(theme::accent()).text_color(theme::accent())))
                    .child(icon(Icon::GridView, px(17.), theme::text_muted())),
            )
            .child(menu_separator().mx(px(0.)))
            .child(
                div()
                    .id("grid-titles")
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(2.))
                    .text_size(px(13.5))
                    .text_color(theme::text_primary())
                    .cursor_pointer()
                    .child(tr!("Show titles"))
                    .child(toggle("grid-titles-sw", titles, false))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.grid_titles = !this.ui.grid_titles;
                        this.save_ui();
                        cx.notify();
                    })),
            )
    }

    fn grid_size_slider(&mut self, cx: &mut Context<Self>) -> Entity<SliderState> {
        if let Some(s) = &self.grid.size {
            return s.clone();
        }
        let (min, max, _) = GRID_COVER_W;
        // `max` first: `min` is clamped against the default max of 100.
        let state = cx.new(|_| SliderState::new().max(max).min(min).step(2.).default_value(self.ui.grid_cover_w));
        cx.subscribe(&state, |this, _, ev: &SliderEvent, cx| {
            let (value, release) = match ev {
                SliderEvent::Change(v) => (v, false),
                SliderEvent::Release(v) => (v, true),
            };
            let SliderValue::Single(v) = *value else { return };
            this.ui.grid_cover_w = v;
            if release {
                this.save_ui();
            }
            cx.notify();
        })
        .detach();
        self.grid.size = Some(state.clone());
        state
    }
}

/// One mode of the switch, `w` wide, its colours and chevron `a` of the way
/// to active. The chevron turns over while the menu is open.
fn mode_segment(mode: &Mode, a: f32, w: f32, active: bool, open: bool) -> Stateful<Div> {
    let a = a.clamp(0., 1.);
    let fg: Hsla = mix(theme::ink_a(0.68), theme::on_solid_pill(), a).into();
    let turn = active && open;
    let caret = icon(Icon::ExpandMore, px(CARET), fg);
    let caret = if theme::animations() {
        caret
            .with_spring("view-mode-caret", SpringAnimation::new(TURN).to(turn).with_epsilon(0.002), |svg, phase| {
                svg.with_transformation(Transformation::rotate(radians(PI * phase.0)))
            })
            .into_any_element()
    } else {
        caret.with_transformation(Transformation::rotate(radians(if turn { PI } else { 0. }))).into_any_element()
    };
    let hover: Hsla = if active { Hsla { a: 0.08, ..theme::on_solid_pill().into() } } else { theme::ink_a(0.08) };
    div()
        .id(mode.id)
        .flex()
        .flex_none()
        .items_center()
        .w(px(w))
        .h(px(SEG_H))
        .pl(px(SEG_PAD))
        .rounded_full()
        .overflow_hidden()
        .cursor_pointer()
        .text_size(px(LABEL))
        .text_color(fg)
        .whitespace_nowrap()
        .hover(move |s| s.bg(hover))
        .when(active && !open, |d| d.tooltip(crate::ui::tooltip(tr!("View options"))))
        .child(icon(mode.ic, px(ICON), fg))
        .child(div().flex_none().ml(px(ICON_GAP)).child(mode.label))
        .child(div().flex().flex_none().justify_end().items_center().w(px(CARET_SLOT * a)).overflow_hidden().opacity(a).child(caret))
}

/// Uppercase label of a group of options.
fn section_label(text: &'static str) -> Div {
    div().text_size(px(11.5)).text_color(theme::text_dim()).child(text.to_uppercase())
}

/// A way to show the Focus list: a sketch of the list beside the page, and
/// its name; the active one is ringed with the accent.
fn list_option(id: &'static str, label: &'static str, rows: bool, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .p(px(8.))
        .rounded(px(theme::R_ITEM))
        .border_1()
        .border_color(if active { theme::accent().into() } else { theme::card_border() })
        .bg(if active { theme::accent_tint() } else { theme::card_fill() })
        .when(!active, |d| d.cursor_pointer().hover(|s| s.bg(theme::button_fill())))
        .text_size(px(12.5))
        .text_color(if active { theme::text_bright() } else { theme::text_primary().into() })
        .child(list_sketch(rows, active))
        .child(label)
}

fn list_sketch(rows: bool, active: bool) -> Div {
    let mark: Hsla = if active { Hsla { a: 0.8, ..theme::accent().into() } } else { theme::ink_a(0.35) };
    let cell = || div().flex_1().rounded(px(1.)).bg(mark);
    let column = if rows {
        div().flex().flex_col().gap(px(3.)).children((0..4).map(|_| div().h(px(3.)).rounded(px(1.)).bg(mark)))
    } else {
        div().flex().flex_col().gap(px(2.)).children((0..2).map(|_| div().flex().flex_1().gap(px(2.)).child(cell()).child(cell())))
    };
    div()
        .flex()
        .gap(px(3.))
        .w(px(64.))
        .h(px(38.))
        .p(px(4.))
        .rounded(px(5.))
        .bg(theme::ink_a(0.06))
        .child(column.w(px(20.)))
        .child(div().flex_1().rounded(px(3.)).bg(theme::ink_a(0.12)))
}
