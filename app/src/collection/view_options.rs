//! The menu of the header's Focus / Grid switch (`view_mode.rs`): the active
//! mode's options, then the covers' look, the picture behind the Collection
//! and the panels as sections that open one at a time, each saying what it
//! is set to while closed (the same settings as Settings → Appearance, which
//! shows them too), a link to Appearance and "Reset to defaults", which puts
//! back what this menu shows.

use std::f32::consts::FRAC_PI_2;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{
    Appearance, BackgroundSource, Corners, CoverHover, Density, InterfaceMode, Section, INTERFACE_BLUR_RANGE, INTERFACE_BLUR_STEP,
    INTERFACE_OPACITY_RANGE,
};
use heronite_data::models::BackgroundSettings;
use heronite_data::settings::{self, CoverAspect, PartialBackground};
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
/// The turn of a section's chevron as it opens or closes.
const TURN: SpringConfig = SpringConfig::new(900., 60., 1.);

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

/// The pictures, with the names Settings → Appearance → "Picture" gives
/// them and the icons of their tiles.
fn pictures() -> [(BackgroundSource, &'static str, Icon); 5] {
    [
        (BackgroundSource::GameArt, tr!("The game's artwork"), Icon::Image),
        (BackgroundSource::Screenshot, tr!("One of your screenshots"), Icon::PhotoCamera),
        (BackgroundSource::Cover, tr!("The cover, blurred"), Icon::ViewCovers),
        (BackgroundSource::Solid, tr!("Solid colour"), Icon::Palette),
        (BackgroundSource::Custom, tr!("Your own picture"), Icon::Add),
    ]
}

/// A section of the menu that opens and closes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LookGroup {
    Covers,
    Background,
    Panels,
}

/// What the menu's sections keep while the app runs.
pub(super) struct LookMenu {
    /// The section open (one at a time); the background's at first.
    open: Option<LookGroup>,
    /// The background's finer sliders (colour intensity, edge shading) show.
    more: bool,
    sliders: Vec<LookSlider>,
}

impl LookMenu {
    /// Opens the covers' section (where the Cover aspect and its notice are).
    pub(super) fn open_covers(&mut self) {
        self.open = Some(LookGroup::Covers);
    }
}

impl Default for LookMenu {
    fn default() -> Self {
        Self { open: Some(LookGroup::Background), more: false, sliders: Vec::new() }
    }
}

/// A slider of the background's and the panels' groups.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LookField {
    /// The Collection art's opacity (`appearance.json`).
    Opacity,
    /// The rest of the background is `local_library/settings.json`.
    Dimming,
    Blur,
    Saturation,
    Scrim,
    /// The glass panels' opacity and blur (`appearance.json`).
    PanelOpacity,
    PanelBlur,
}

impl LookField {
    /// Min, max and step: the ranges of Settings → Appearance.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Self::Opacity => (0., 100., 1.),
            Self::Dimming => (0., 90., 1.),
            Self::Blur => (0., 40., 1.),
            Self::Saturation | Self::Scrim => (0., 150., 1.),
            Self::PanelOpacity => (INTERFACE_OPACITY_RANGE.0, INTERFACE_OPACITY_RANGE.1, 1.),
            Self::PanelBlur => (INTERFACE_BLUR_RANGE.0, INTERFACE_BLUR_RANGE.1, INTERFACE_BLUR_STEP),
        }
    }
}

pub(super) struct LookSlider {
    field: LookField,
    state: Entity<SliderState>,
    /// The value in use when the slider last matched it: when it changes
    /// elsewhere (Settings, a reload, "Reset to defaults"), the slider follows.
    seen: f32,
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
    pub(super) fn view_options_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mode = if self.ui.grid_view { self.grid_options(cx) } else { self.focus_options(cx) };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(mode)
            .child(
                option_row(tr!("Group copies of a game"), toggle("group-copies-sw", self.ui.group_copies, false))
                    .id("group-copies")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.set_group_copies(!this.ui.group_copies, cx))),
            )
            .child(separator())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(self.look_section(LookGroup::Background, window, cx))
                    .child(self.look_section(LookGroup::Covers, window, cx))
                    .child(self.look_section(LookGroup::Panels, window, cx)),
            )
            .child(separator())
            .child(self.options_footer(cx))
    }

    /// A section: its icon, its name and what it is set to, and under them
    /// its options while open. A click on the name opens it and closes the
    /// others; the options fade and slide in (only while they appear, so
    /// nothing animates once they are there).
    fn look_section(&mut self, group: LookGroup, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let open = self.view_look.open == Some(group);
        let a = theme::appearance();
        let (i, glyph, title, summary): (usize, Icon, &str, SharedString) = match group {
            LookGroup::Covers => {
                let shape = SHAPES.iter().find(|(s, _)| *s == self.card_shape).map_or("2:3", |(_, r)| r);
                let corners = match a.cover_corners {
                    Corners::Square => tr!("Square"),
                    Corners::Soft => tr!("Soft"),
                    Corners::Round => tr!("Round"),
                };
                let density = match a.density {
                    Density::Comfortable => tr!("Comfortable"),
                    Density::Compact => tr!("Compact"),
                };
                (0, Icon::ViewCovers, tr!("Covers"), format!("{shape} · {corners} · {density}").into())
            }
            LookGroup::Background => {
                let picture = pictures().into_iter().find(|(p, ..)| *p == a.background).map_or("", |(_, name, _)| name);
                (1, Icon::Image, tr!("Background"), if self.stage_enabled { picture } else { tr!("Off") }.into())
            }
            LookGroup::Panels => {
                let summary = match a.interface_mode {
                    InterfaceMode::Transparent => format!("{} · {}%", tr!("Transparent"), a.interface_opacity.round()).into(),
                    InterfaceMode::Solid => tr!("Solid").into(),
                };
                (2, Icon::ViewSidebar, tr!("Panels"), summary)
            }
        };
        let chevron = icon(Icon::ChevronRight, px(15.), if open { theme::text_bright().into() } else { theme::text_muted() });
        let chevron = if theme::animations() {
            chevron
                .with_spring(("view-look-caret", i), SpringAnimation::new(TURN).to(open).with_epsilon(0.002), |svg, phase| {
                    svg.with_transformation(Transformation::rotate(radians(FRAC_PI_2 * phase.0)))
                })
                .into_any_element()
        } else {
            chevron.when(open, |svg| svg.with_transformation(Transformation::rotate(radians(FRAC_PI_2)))).into_any_element()
        };
        // The icon in an accent tile says "there is more here".
        let badge =
            div().flex().flex_none().items_center().justify_center().size(px(26.)).rounded(px(7.)).bg(theme::accent_tint()).child(icon(
                glyph,
                px(15.),
                theme::accent(),
            ));
        let head = div()
            .id(("view-look", i))
            .flex()
            .items_center()
            .gap(px(9.))
            .pl(px(7.))
            .pr(px(10.))
            .py(px(7.))
            .rounded(px(10.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.06)))
            .child(badge)
            .child(
                div()
                    .flex_none()
                    .text_size(px(13.))
                    .text_color(if open { theme::text_bright() } else { theme::text_primary().into() })
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_right()
                    .text_size(px(11.5))
                    .text_color(theme::text_muted())
                    .child(summary),
            )
            .child(chevron)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.view_look.open = if this.view_look.open == Some(group) { None } else { Some(group) };
                cx.notify();
            }));
        let section = div()
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(if open { theme::ink_a(0.08) } else { theme::ink_a(0.) })
            .bg(theme::ink_a(if open { 0.05 } else { 0.035 }))
            .child(head);
        if !open {
            return section;
        }
        let body = match group {
            LookGroup::Covers => self.cover_options(cx),
            LookGroup::Background => self.background_options(window, cx),
            LookGroup::Panels => self.panel_options(window, cx),
        }
        .relative()
        .px(px(10.))
        .pt(px(2.))
        .pb(px(12.));
        if !theme::animations() {
            return section.child(body);
        }
        // `top` moves the options without laying the menu out again.
        section.child(body.with_animation(
            ("view-look-in", i),
            Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
            |el, t| el.opacity(t).top(px(-6. * (1. - t))),
        ))
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
                chip(("view-section", i), section_name(s), sections.shown(s))
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
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().flex().flex_col().gap(px(SHAPE_GAP)).children({
                let mut tiles = tiles.into_iter();
                [0, 1].map(|_| div().flex().gap(px(SHAPE_GAP)).children(tiles.by_ref().take(3)))
            }))
            .children(crate::dialogs::metadata_update::family_notice::notice("view-notice", cx))
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
            .child(
                option_row(tr!("Store icon"), toggle("cover-stores-sw", a.cover_stores, false))
                    .id("cover-stores")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.set_look(|a| a.cover_stores = !a.cover_stores, cx))),
            )
            // The style is picked in Settings → Appearance (it shows them all).
            .child(
                option_row(tr!("Categories on covers"), toggle("cover-categories-sw", a.cover_categories, false))
                    .id("cover-categories")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.set_look(|a| a.cover_categories = !a.cover_categories, cx))),
            )
    }

    /// The picture behind the Collection: on or off, which one, how much
    /// of it shows.
    fn background_options(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let on = self.stage_enabled;
        let body = div().flex().flex_col().gap(px(10.)).child(
            option_row(tr!("Show"), toggle("view-bg-sw", on, false)).id("view-bg").cursor_pointer().on_click(cx.listener(
                |this, _, _, cx| {
                    this.stage_enabled = !this.stage_enabled;
                    this.save_background(cx);
                    cx.notify();
                },
            )),
        );
        // Off: nothing else applies, so nothing else shows.
        if !on {
            return body;
        }
        let current = theme::appearance().background;
        let tiles = pictures()
            .into_iter()
            .enumerate()
            .map(|(i, (source, name, glyph))| {
                let on = source == current;
                div()
                    .id(("view-picture", i))
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .h(px(34.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if on { theme::accent().into() } else { theme::ink_a(0.1) })
                    .bg(if on { theme::accent_tint() } else { theme::ink_a(0.05) })
                    .when(!on, |d| d.cursor_pointer().hover(|st| st.bg(theme::button_fill())))
                    .tooltip(crate::ui::tooltip(name))
                    .child(icon(glyph, px(17.), if on { theme::accent() } else { theme::text_muted() }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // A picture of your own is chosen in Appearance first.
                        if source == BackgroundSource::Custom && !this.app_paths.custom_background().exists() {
                            this.open_menu = None;
                            this.open_settings_section(crate::pages::settings::Section::Appearance, window, cx);
                            return;
                        }
                        this.set_look(|a| a.background = source, cx);
                    }))
            })
            .collect::<Vec<_>>();
        let name = pictures().into_iter().find(|(p, ..)| *p == current).map_or("", |(_, name, _)| name);
        // The light theme fades the art where the dark one darkens it.
        let dimming = if theme::theme_is_light() { tr!("Fading") } else { tr!("Darkening") };
        let percent = |v: f32| format!("{}%", v.round());
        let more = self.view_look.more;
        let more_link = div()
            .id("view-bg-more")
            .flex()
            .items_center()
            .gap(px(2.))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(theme::accent())
            .hover(|s| s.text_color(theme::text_bright()))
            .child(
                icon(Icon::ChevronRight, px(15.), theme::accent())
                    .when(more, |i| i.with_transformation(Transformation::rotate(percentage(0.25)))),
            )
            .child(if more { tr!("Fewer adjustments") } else { tr!("More adjustments") })
            .on_click(cx.listener(|this, _, _, cx| {
                this.view_look.more = !this.view_look.more;
                cx.notify();
            }));
        body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(div().flex().gap(px(5.)).children(tiles))
                .child(div().text_size(px(11.5)).text_color(theme::text_muted()).child(name)),
        )
        .child(self.slider_block(LookField::Opacity, tr!("Opacity"), percent, window, cx))
        .child(self.slider_block(LookField::Dimming, dimming, percent, window, cx))
        .child(self.slider_block(LookField::Blur, tr!("Blur"), |v| format!("{} px", v.round()), window, cx))
        .child(more_link)
        .when(more, |d| {
            d.child(self.slider_block(LookField::Saturation, tr!("Colour intensity"), percent, window, cx)).child(self.slider_block(
                LookField::Scrim,
                tr!("Edge shading"),
                percent,
                window,
                cx,
            ))
        })
    }

    /// Menus, dialogs and pages: glass (with its opacity and blur) or solid
    /// (its colour is picked in Appearance).
    fn panel_options(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mode = theme::appearance().interface_mode;
        let modes = [(InterfaceMode::Transparent, tr!("Transparent")), (InterfaceMode::Solid, tr!("Solid"))];
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(segments("view-panels", &modes, mode, true, cx, |this, m, cx| this.set_look(|a| a.interface_mode = m, cx)))
            .when(mode == InterfaceMode::Transparent, |d| {
                d.child(self.slider_block(LookField::PanelOpacity, tr!("Opacity"), |v| format!("{}%", v.round()), window, cx))
                    .child(self.slider_block(LookField::PanelBlur, tr!("Blur"), |v| format!("{} px", v.round()), window, cx))
            })
    }

    /// A slider under its name and value.
    fn slider_block(
        &mut self,
        field: LookField,
        label: &'static str,
        value: impl Fn(f32) -> String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.look_slider(field, window, cx);
        let shown = match state.read(cx).value() {
            SliderValue::Single(v) => v,
            _ => self.look_value(field),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(13.))
                    .text_color(theme::text_primary())
                    .child(label)
                    .child(div().text_size(px(11.5)).text_color(theme::text_muted()).child(value(shown))),
            )
            .child(div().px(px(2.)).child(Slider::new(&state).bg(theme::accent()).text_color(theme::accent())))
    }

    /// The value `field` has in use now.
    fn look_value(&self, field: LookField) -> f32 {
        let a = theme::appearance();
        match field {
            LookField::Opacity => a.collection_opacity,
            LookField::Dimming => self.scrim.dim * 100.,
            LookField::Blur => self.blur_px,
            LookField::Saturation => self.saturation * 100.,
            LookField::Scrim => self.scrim.scrim * 100.,
            LookField::PanelOpacity => a.interface_opacity,
            LookField::PanelBlur => a.interface_blur,
        }
    }

    /// `field`'s slider, made on first use and moved to the value in use
    /// when that changed elsewhere.
    fn look_slider(&mut self, field: LookField, window: &mut Window, cx: &mut Context<Self>) -> Entity<SliderState> {
        let value = self.look_value(field);
        if let Some(s) = self.view_look.sliders.iter_mut().find(|s| s.field == field) {
            if (s.seen - value).abs() > 0.01 {
                s.seen = value;
                s.state.update(cx, |st, cx| st.set_value(value, window, cx));
            }
            return s.state.clone();
        }
        let (min, max, step) = field.range();
        // `max` first: `min` is clamped against the default max of 100.
        let state = cx.new(|_| SliderState::new().max(max).min(min).step(step).default_value(value));
        cx.subscribe(&state, move |this, _, ev: &SliderEvent, cx| {
            let (value, release) = match ev {
                SliderEvent::Change(v) => (v, false),
                SliderEvent::Release(v) => (v, true),
            };
            let SliderValue::Single(v) = *value else { return };
            this.slide_look(field, v, release, cx);
        })
        .detach();
        self.view_look.sliders.push(LookSlider { field, state: state.clone(), seen: value });
        state
    }

    /// A slider moved: applied at once, saved when let go.
    fn slide_look(&mut self, field: LookField, v: f32, release: bool, cx: &mut Context<Self>) {
        match field {
            LookField::Dimming => {
                self.scrim.dim = v / 100.;
                // The roll-off is a new raster of the art: it follows when
                // the slider is let go.
                if release {
                    self.rolloff = heronite_images::backdrop_rolloff(self.scrim.dim);
                }
            }
            LookField::Scrim => self.scrim.scrim = v / 100.,
            // Each blur and colour intensity is a new raster of the art:
            // they change when the slider is let go.
            LookField::Blur if release => self.blur_px = v,
            LookField::Saturation if release => self.saturation = v / 100.,
            LookField::Blur | LookField::Saturation => {}
            LookField::Opacity => self.apply_look(|a| a.collection_opacity = v, release, cx),
            LookField::PanelOpacity => self.apply_look(|a| a.interface_opacity = v, release, cx),
            LookField::PanelBlur => self.apply_look(|a| a.interface_blur = v, release, cx),
        }
        let now = self.look_value(field);
        if let Some(s) = self.view_look.sliders.iter_mut().find(|s| s.field == field) {
            s.seen = now;
        }
        if release && matches!(field, LookField::Dimming | LookField::Scrim | LookField::Blur | LookField::Saturation) {
            self.save_background(cx);
        }
        cx.notify();
    }

    /// The Collection background as the menu has it, to
    /// `local_library/settings.json` off the UI thread (the library reloads
    /// it from there).
    fn save_background(&self, cx: &mut Context<Self>) {
        let patch = PartialBackground {
            enabled: Some(self.stage_enabled),
            blur: Some(self.blur_px as f64),
            dimming: Some((self.scrim.dim * 100.) as f64),
            saturation: Some((self.saturation * 100.) as f64),
            scrim: Some((self.scrim.scrim * 100.) as f64),
        };
        let paths = self.data_paths.clone();
        cx.background_spawn(async move {
            if let Err(err) = settings::save_ui(&paths, None, Some(&patch)) {
                log::warn!("saving the Collection background: {err:#}");
            }
        })
        .detach();
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
            && a.cover_badges == d.cover_badges
            && a.cover_categories == d.cover_categories
            && a.cover_stores == d.cover_stores;
        let bg = BackgroundSettings::default();
        let near = |v: f32, d: f64| (v - d as f32).abs() < 0.5;
        let background = self.stage_enabled == bg.enabled
            && near(self.blur_px, bg.blur.get())
            && near(self.scrim.dim * 100., bg.dimming.get())
            && near(self.saturation * 100., bg.saturation.get())
            && near(self.scrim.scrim * 100., bg.scrim.get())
            && a.background == d.background
            && a.collection_opacity == d.collection_opacity;
        let panels =
            a.interface_mode == d.interface_mode && a.interface_opacity == d.interface_opacity && a.interface_blur == d.interface_blur;
        let mode = if self.ui.grid_view {
            self.ui.grid_cover_w == GRID_COVER_W.2 && self.ui.grid_label == GridLabel::default() && self.ui.grid_open_on_select
        } else {
            !self.ui.focus_list
                && self.ui.focus_cover_size == FocusCoverSize::default()
                && Section::ALL.iter().all(|s| a.sections.shown(*s))
        };
        covers && background && panels && mode && self.ui.group_copies
    }

    /// "Reset to defaults": the current mode's options, the covers' look,
    /// the background and the panels. The order of the game's sections stays
    /// (it is set in Appearance), and so do the solid panels' colour.
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
        self.ui.group_copies = true;
        self.save_ui();
        self.refresh_groups();
        let bg = BackgroundSettings::default();
        self.stage_enabled = bg.enabled;
        self.blur_px = bg.blur.get() as f32;
        self.saturation = bg.saturation.get() as f32 / 100.;
        self.scrim = super::stage::Scrim { dim: bg.dimming.get() as f32 / 100., scrim: bg.scrim.get() as f32 / 100. };
        self.rolloff = heronite_images::backdrop_rolloff(self.scrim.dim);
        self.save_background(cx);
        self.save_card_shape(CoverAspect::default(), cx);
        self.set_look(
            |a| {
                let d = Appearance::DEFAULT;
                (a.cover_corners, a.density, a.cover_hover, a.cover_badges) = (d.cover_corners, d.density, d.cover_hover, d.cover_badges);
                a.cover_categories = d.cover_categories;
                a.cover_stores = d.cover_stores;
                (a.background, a.collection_opacity) = (d.background, d.collection_opacity);
                (a.interface_mode, a.interface_opacity, a.interface_blur) = (d.interface_mode, d.interface_opacity, d.interface_blur);
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

    /// "Group copies of a game": one card per game, or one per copy.
    pub(super) fn set_group_copies(&mut self, on: bool, cx: &mut Context<Self>) {
        self.ui.group_copies = on;
        self.save_ui();
        self.refresh_groups();
        cx.notify();
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
        if cx.has_global::<crate::dialogs::metadata_update::hub::HubGlobal>() {
            let before = self.card_shape;
            crate::dialogs::metadata_update::hub::hub(cx).update(cx, |h, cx| h.aspect_picked(before, shape, cx));
        }
        crate::dialogs::metadata_update::families::aspect_changed(self.card_shape, shape);
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
    pub(super) fn set_look(&mut self, edit: impl FnOnce(&mut Appearance), cx: &mut Context<Self>) {
        self.apply_look(edit, true, cx);
    }

    /// [`Self::set_look`], saved only when `save` (a slider saves when let go).
    fn apply_look(&mut self, edit: impl FnOnce(&mut Appearance), save: bool, cx: &mut Context<Self>) {
        let mut look = theme::appearance();
        edit(&mut look);
        theme::set_appearance(look);
        // Achievements may have been switched: running games' live checks follow.
        crate::services::achievements::get(cx).update(cx, |s, cx| s.sync_live(cx));
        if save {
            let look = theme::appearance();
            let app = self.app_paths.clone();
            cx.background_spawn(async move {
                if let Err(err) = look.save(&app) {
                    log::warn!("saving appearance.json: {err:#}");
                }
            })
            .detach();
        }
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

/// A rounded chip, filled with the accent while `on`.
fn chip(id: impl Into<ElementId>, label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
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
        .child(label)
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
