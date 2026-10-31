//! The menu of the header's Focus / Grid switch (`view_mode.rs`): the active
//! mode's options, the covers' look (the same settings as Settings →
//! Appearance, which shows them too), a link to Appearance and "Reset to
//! defaults", which puts back what this menu shows.

use std::rc::Rc;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{Appearance, Corners, CoverHover, Density, Section};
use heronite_data::settings::{self, CoverAspect};
use heronite_data::ui_state::{FocusCoverSize, GridLabel, GRID_COVER_W};

use super::header::Menu;
use super::list_view::card_aspect;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::toggle;
use crate::ui::icon;
use heronite_i18n::tr;

pub(super) const MENU_W: f32 = 300.;
pub(super) const MENU_PAD: f32 = 12.;
const SHAPE_GAP: f32 = 6.;

const SHAPES: [(CoverAspect, &str); 6] = [
    (CoverAspect::Steam, "2:3"),
    (CoverAspect::Igdb, "3:4"),
    (CoverAspect::Gog, "11:15"),
    (CoverAspect::Square, "1:1"),
    (CoverAspect::Dvd, "5:7"),
    (CoverAspect::Banner, "16:9"),
];

/// The names Settings → Appearance → "Cover aspect" gives the shapes.
fn shape_name(shape: CoverAspect) -> &'static str {
    match shape {
        CoverAspect::Steam => "Steam (2:3)",
        CoverAspect::Igdb => "IGDB (3:4)",
        CoverAspect::Gog => "GOG (11:15)",
        CoverAspect::Square => tr!("Square (1:1)"),
        CoverAspect::Dvd => tr!("DVD case (5:7)"),
        CoverAspect::Banner => tr!("Banner (16:9)"),
    }
}

fn section_name(s: Section) -> &'static str {
    match s {
        Section::Screenshots => tr!("Screenshots"),
        Section::Overview => tr!("Overview"),
        Section::Details => tr!("Details"),
        Section::Achievements => tr!("Achievements"),
        Section::Timeline => tr!("Timeline"),
        Section::Related => tr!("Related games"),
    }
}

impl CollectionView {
    pub(super) fn view_options_body(&mut self, cx: &mut Context<Self>) -> Div {
        let mode = if self.ui.grid_view { self.grid_options(cx) } else { self.focus_options(cx) };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(mode)
            .child(separator())
            .child(self.cover_options(cx))
            .child(separator())
            .child(self.options_footer(cx))
    }

    fn focus_options(&mut self, cx: &mut Context<Self>) -> Div {
        let rows = self.ui.focus_list;
        let sizes = [(FocusCoverSize::Small, tr!("Small")), (FocusCoverSize::Medium, tr!("Medium")), (FocusCoverSize::Large, tr!("Large"))];
        let sections = theme::appearance().sections;
        let chips = sections
            .order()
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let on = sections.shown(s);
                div()
                    .id(("view-section", i))
                    .flex()
                    .items_center()
                    .h(px(24.))
                    .px(px(9.))
                    .rounded_full()
                    .border_1()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .border_color(if on { theme::accent_tint_border() } else { theme::ink_a(0.12) })
                    .when(on, |d| d.bg(theme::accent_tint()).text_color(theme::text_bright()))
                    .when(!on, |d| d.text_color(theme::text_muted()).hover(|st| st.text_color(theme::text_bright())))
                    .child(section_name(s))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_look(|a| a.sections = a.sections.toggled(s), cx)))
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                group(tr!("Game list"))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                list_option("list-covers", tr!("Covers"), false, !rows)
                                    .on_click(cx.listener(|this, _, _, cx| this.set_focus_list(false, cx))),
                            )
                            .child(
                                list_option("list-rows", tr!("List"), true, rows)
                                    .on_click(cx.listener(|this, _, _, cx| this.set_focus_list(true, cx))),
                            ),
                    )
                    .when(!rows, |d| {
                        d.child(option_row(
                            tr!("Cover size"),
                            segments("focus-size", &sizes, self.ui.focus_cover_size, false, cx, |this, size, cx| {
                                this.ui.focus_cover_size = size;
                                this.save_ui();
                                this.relayout_cards(cx);
                            }),
                        ))
                    }),
            )
            .child(group(tr!("Game sections")).child(div().flex().flex_wrap().gap(px(5.)).children(chips)))
    }

    fn grid_options(&mut self, cx: &mut Context<Self>) -> Div {
        let slider = self.grid_size_slider(cx);
        let labels = [
            (GridLabel::Nothing, tr!("Nothing")),
            (GridLabel::Title, tr!("Title")),
            (GridLabel::Info, tr!("Info")),
            (GridLabel::TitleInfo, tr!("Title and info")),
        ];
        let open = self.ui.grid_open_on_select;
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(div().flex().items_center().justify_between().child(section_label(tr!("Cover size"))).child(
                        div().text_size(px(11.5)).text_color(theme::text_muted()).child(format!("{} px", self.ui.grid_cover_w.round())),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(2.))
                            .child(icon(Icon::GridView, px(12.), theme::text_muted()))
                            .child(div().flex_1().child(Slider::new(&slider).bg(theme::accent()).text_color(theme::accent())))
                            .child(icon(Icon::GridView, px(17.), theme::text_muted())),
                    ),
            )
            .child(group(tr!("Under the cover")).child(segments("grid-label", &labels, self.ui.grid_label, true, cx, |this, label, cx| {
                this.ui.grid_label = label;
                this.save_ui();
                this.relayout_cards(cx);
            })))
            .child(
                option_row(tr!("Open details on select"), toggle("grid-open-sw", open, false))
                    .id("grid-open")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.set_open_on_select(!this.ui.grid_open_on_select, cx))),
            )
    }

    /// The covers' look, shared by both modes.
    fn cover_options(&mut self, cx: &mut Context<Self>) -> Div {
        let a = theme::appearance();
        let shape = self.card_shape;
        let tiles = SHAPES
            .iter()
            .enumerate()
            .map(|(i, (s, ratio))| {
                let (s, on) = (*s, *s == shape);
                let sketch_w = (22. / card_aspect(s)).min(40.);
                div()
                    .id(("view-shape", i))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(5.))
                    .flex_1()
                    .py(px(7.))
                    .rounded(px(9.))
                    .border_1()
                    .text_size(px(11.5))
                    .border_color(if on { theme::accent().into() } else { theme::ink_a(0.) })
                    .bg(if on { theme::accent_tint() } else { theme::card_fill() })
                    .text_color(if on { theme::text_bright() } else { theme::text_muted().into() })
                    .when(!on, |d| d.cursor_pointer().hover(|st| st.bg(theme::button_fill())))
                    .tooltip(crate::ui::tooltip(shape_name(s)))
                    .child(div().w(px(sketch_w)).h(px(22.)).rounded(px(3.)).bg(if on {
                        Hsla { a: 0.8, ..theme::accent().into() }
                    } else {
                        theme::ink_a(0.3)
                    }))
                    .child(*ratio)
                    .on_click(cx.listener(move |this, _, _, cx| this.save_card_shape(s, cx)))
            })
            .collect::<Vec<_>>();
        let corners = [(Corners::Square, tr!("Square")), (Corners::Soft, tr!("Soft")), (Corners::Round, tr!("Round"))];
        let density = [(Density::Comfortable, tr!("Comfortable")), (Density::Compact, tr!("Compact"))];
        let hover = [(CoverHover::Nothing, tr!("Nothing")), (CoverHover::Lift, tr!("Lift")), (CoverHover::Glow, tr!("Glow"))];
        group(tr!("Covers"))
            .child(div().flex().flex_col().gap(px(SHAPE_GAP)).children({
                let mut tiles = tiles.into_iter();
                [0, 1].map(|_| div().flex().gap(px(SHAPE_GAP)).children(tiles.by_ref().take(3)))
            }))
            .child(option_row(
                tr!("Corners"),
                segments("cover-corners", &corners, a.cover_corners, false, cx, |this, v, cx| this.set_look(|a| a.cover_corners = v, cx)),
            ))
            .child(option_row(
                tr!("Density"),
                segments("cover-density", &density, a.density, false, cx, |this, v, cx| {
                    this.set_look(|a| a.density = v, cx);
                    this.relayout_cards(cx);
                }),
            ))
            .child(option_row(
                tr!("Under the pointer"),
                segments("cover-hover", &hover, a.cover_hover, false, cx, |this, v, cx| this.set_look(|a| a.cover_hover = v, cx)),
            ))
            .child(
                option_row(tr!("Favourite star"), toggle("cover-badges-sw", a.cover_badges, false))
                    .id("cover-badges")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.set_look(|a| a.cover_badges = !a.cover_badges, cx))),
            )
    }

    fn options_footer(&mut self, cx: &mut Context<Self>) -> Div {
        let can_reset = !self.view_options_at_default();
        div()
            .flex()
            .items_center()
            .justify_between()
            .text_size(px(12.5))
            .child(
                div()
                    .id("view-more")
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .cursor_pointer()
                    .text_color(theme::accent())
                    .hover(|s| s.text_color(theme::text_bright()))
                    .child(tr!("More in Appearance"))
                    .child(icon(Icon::ChevronRight, px(15.), theme::accent()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_menu = None;
                        this.open_settings_section(crate::pages::settings::Section::Appearance, window, cx);
                    })),
            )
            .child(
                div()
                    .id("view-reset")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(8.))
                    .text_color(theme::text_muted())
                    .child(icon(Icon::Refresh, px(14.), theme::text_muted()))
                    .child(tr!("Reset to defaults"))
                    .when(!can_reset, |d| d.opacity(0.4))
                    .when(can_reset, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme::ink_a(0.08)).text_color(theme::text_bright()))
                            .on_click(cx.listener(|this, _, window, cx| this.reset_view_options(window, cx)))
                    }),
            )
    }

    /// Whether what the menu shows for the current mode is all at its default.
    fn view_options_at_default(&self) -> bool {
        let (a, d) = (theme::appearance(), Appearance::DEFAULT);
        let covers = self.card_shape == CoverAspect::default()
            && a.cover_corners == d.cover_corners
            && a.density == d.density
            && a.cover_hover == d.cover_hover
            && a.cover_badges == d.cover_badges;
        let mode = if self.ui.grid_view {
            self.ui.grid_cover_w == GRID_COVER_W.2 && self.ui.grid_label == GridLabel::default() && self.ui.grid_open_on_select
        } else {
            !self.ui.focus_list
                && self.ui.focus_cover_size == FocusCoverSize::default()
                && Section::ALL.iter().all(|s| a.sections.shown(*s))
        };
        covers && mode
    }

    /// "Reset to defaults": the current mode's options and the covers' look.
    /// The order of the game's sections stays (it is set in Appearance).
    fn reset_view_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let grid = self.ui.grid_view;
        if grid {
            self.ui.grid_cover_w = GRID_COVER_W.2;
            if let Some(slider) = &self.grid.size {
                slider.update(cx, |s, cx| s.set_value(GRID_COVER_W.2, window, cx));
            }
            self.ui.grid_label = GridLabel::default();
            self.set_open_on_select(true, cx);
        } else {
            self.ui.focus_list = false;
            self.ui.focus_cover_size = FocusCoverSize::default();
        }
        self.save_ui();
        self.save_card_shape(CoverAspect::default(), cx);
        self.set_look(
            |a| {
                let d = Appearance::DEFAULT;
                (a.cover_corners, a.density, a.cover_hover, a.cover_badges) = (d.cover_corners, d.density, d.cover_hover, d.cover_badges);
                if !grid {
                    for s in Section::ALL {
                        if !a.sections.shown(*s) {
                            a.sections = a.sections.toggled(*s);
                        }
                    }
                }
            },
            cx,
        );
        self.relayout_cards(cx);
    }

    fn set_open_on_select(&mut self, on: bool, cx: &mut Context<Self>) {
        self.ui.grid_open_on_select = on;
        self.grid.drawer_closed = !on;
        self.save_ui();
        cx.notify();
    }

    /// Settings → Appearance → "Cover aspect" (`settings.json`), saved at once.
    fn save_card_shape(&mut self, shape: CoverAspect, cx: &mut Context<Self>) {
        if self.card_shape == shape {
            return;
        }
        self.set_card_shape(shape);
        if let Some(lib) = &self.lib {
            let mut patch = lib.settings.metadata.clone();
            patch.cover_aspect = shape.as_str().to_owned();
            if let Err(err) = settings::save_metadata(&self.data_paths, &patch) {
                log::warn!("saving the cover aspect: {err:#}");
            }
        }
        self.relayout_cards(cx);
    }

    /// Changes Settings → Appearance at once and saves it off the UI thread,
    /// like the Appearance page.
    fn set_look(&mut self, edit: impl FnOnce(&mut Appearance), cx: &mut Context<Self>) {
        let mut look = theme::appearance();
        edit(&mut look);
        theme::set_appearance(look);
        let look = theme::appearance();
        let app = self.app_paths.clone();
        cx.background_spawn(async move {
            if let Err(err) = look.save(&app) {
                log::warn!("saving appearance.json: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// The cards changed height (shape, size, the lines under them): the
    /// list is laid out again, keeping the selected game in view.
    fn relayout_cards(&mut self, cx: &mut Context<Self>) {
        self.rebuild_items();
        self.grid.reveal = true;
        cx.notify();
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

    /// Closes the menu when a press lands outside it (not on the active
    /// mode, whose click closes it itself).
    pub(super) fn view_options_press_out(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.grid.menu_press) {
            return;
        }
        if self.open_menu == Some(Menu::Display) {
            self.open_menu = None;
            cx.notify();
        }
    }
}

fn separator() -> Div {
    div().h(px(1.)).bg(theme::ink_a(0.08))
}

/// Uppercase label of a group of options.
fn section_label(text: &'static str) -> Div {
    div().text_size(px(11.5)).text_color(theme::text_dim()).child(text.to_uppercase())
}

/// A labelled group of options.
fn group(label: &'static str) -> Div {
    div().flex().flex_col().gap(px(8.)).child(section_label(label))
}

/// A label on the left, its control on the right.
fn option_row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .min_h(px(26.))
        .text_size(px(13.))
        .text_color(theme::text_primary())
        .child(label)
        .child(control)
}

/// A small segmented control; `pick` runs with the value clicked.
fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    items: &[(T, &'static str)],
    current: T,
    full: bool,
    cx: &mut Context<CollectionView>,
    pick: impl Fn(&mut CollectionView, T, &mut Context<CollectionView>) + 'static,
) -> Div {
    let pick = Rc::new(pick);
    let buttons = items.iter().enumerate().map(|(i, (value, label))| {
        let (value, on) = (*value, *value == current);
        let pick = pick.clone();
        let fg: Hsla = if on { theme::text_bright() } else { theme::text_muted().into() };
        div()
            .id((id, i))
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(px(24.))
            .px(px(8.))
            .rounded(px(7.))
            .text_size(px(12.))
            .whitespace_nowrap()
            .text_color(fg)
            .when(on, |d| d.bg(theme::ink_a(0.14)))
            .when(!on, |d| d.cursor_pointer().hover(|s| s.text_color(theme::text_bright())))
            .child(*label)
            .on_click(cx.listener(move |this, _, _, cx| pick(this, value, cx)))
    });
    div().flex().flex_none().p(px(2.)).gap(px(2.)).rounded(px(9.)).bg(theme::ink_a(0.06)).when(full, |d| d.w_full()).children(buttons)
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
