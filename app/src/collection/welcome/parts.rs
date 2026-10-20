//! The pieces of the welcome: the source rows, the shelf of empty covers,
//! and the screen shown when the library cannot be loaded.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use crate::collection::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::icon;

/// The covers on the side: a 5-wide shelf, 2:3 like the real ones.
const SHELF_W: f32 = 560.;
const SHELF_COLS: usize = 5;
const SHELF_GAP: f32 = 16.;
const SHELF_PAD: f32 = 26.;

/// What a source row shows: its status and a line on what happens next.
pub(super) struct Status {
    pub label: String,
    pub color: Rgba,
    pub line: String,
}

pub(super) fn status(label: impl Into<String>, color: Rgba, line: impl Into<String>) -> Status {
    Status { label: label.into(), color, line: line.into() }
}

/// Empty covers where the games will show, fading out at the bottom.
pub(super) fn shelf(h: f32) -> Div {
    let h = h.clamp(420., 760.);
    let w = (SHELF_W - 2. * SHELF_PAD - (SHELF_COLS - 1) as f32 * SHELF_GAP) / SHELF_COLS as f32;
    let rows = ((h - SHELF_PAD) / (w * 1.5 + SHELF_GAP)).ceil() as usize;
    let slot = || {
        div().flex_none().w(px(w)).h(px(w * 1.5)).rounded(px(10.)).bg(theme::white_a(0.04)).border_1().border_color(theme::white_a(0.07))
    };
    let grid = div()
        .absolute()
        .top(px(SHELF_PAD))
        .left(px(SHELF_PAD))
        .right(px(SHELF_PAD))
        .flex()
        .flex_col()
        .gap(px(SHELF_GAP))
        .children((0..rows).map(|_| div().flex().gap(px(SHELF_GAP)).children((0..SHELF_COLS).map(|_| slot()))));
    let bg = theme::page_bg();
    div()
        .relative()
        .flex_none()
        .w(px(SHELF_W))
        .h(px(h))
        .rounded(px(26.))
        .overflow_hidden()
        .bg(theme::white_a(0.02))
        .border_1()
        .border_color(theme::white_a(0.06))
        .child(grid)
        .child(div().absolute().left_0().right_0().bottom_0().h(relative(0.5)).bg(linear_gradient(
            180.,
            linear_color_stop(theme::rgb_a(7, 9, 12, 0.), 0.),
            linear_color_stop(bg, 0.85),
        )))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(28.))
                .px(px(40.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .text_center()
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(17.))
                        .text_color(theme::text_primary())
                        .child(tr!("Your games show up here")),
                )
                .child(
                    div()
                        .text_size(px(13.5))
                        .line_height(px(20.))
                        .text_color(theme::text_muted())
                        .child(tr!("With their covers, what is installed and the play time Steam already counted.")),
                ),
        )
}

impl CollectionView {
    /// The first load failed (a damaged file in the data folder, a disk
    /// error): what happened and a way to try again, instead of an empty list.
    pub(in crate::collection) fn render_load_error(&self, err: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(32.))
            .child(
                div()
                    .w(px(560.))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .px(px(40.))
                    .py(px(36.))
                    .rounded(px(24.))
                    .bg(theme::rgb_a(14, 18, 24, 0.96))
                    .border_1()
                    .border_color(theme::white_a(0.1))
                    .child(icon(Icon::Warning, px(36.), theme::danger_text()))
                    .child(
                        div()
                            .font_family(theme::font_heading())
                            .text_size(px(24.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child(tr!("Your library could not be loaded")),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(14.))
                            .text_color(theme::text_muted())
                            .child(tr!("A file in the data folder could not be read. Your games and their files were not touched.")),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .max_w_full()
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .bg(theme::white_a(0.05))
                            .font_family(theme::FONT_MONO)
                            .text_size(px(12.))
                            .text_color(theme::text_dim())
                            .child(err),
                    )
                    .child(div().mt(px(8.)).child(button("load-retry", ButtonKind::Primary, tr!("Try again")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.reload(cx);
                        },
                    )))),
            )
            .into_any_element()
    }
}

/// A small caption over a group of rows.
pub(super) fn label(ic: Icon, text: &'static str) -> Div {
    div()
        .mt(px(28.))
        .mb(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(11.5))
        .text_color(theme::text_dim())
        .child(icon(ic, px(14.), theme::text_dim()))
        .child(text.to_uppercase())
}

/// A source found on this PC: its icon, name and status, a line on what
/// happens next, a note and a button. `main` is Steam's larger row.
pub(super) fn source_row(
    ic: Icon,
    title: String,
    st: Status,
    button: Option<Stateful<Div>>,
    note: Option<(String, Rgba)>,
    main: bool,
) -> Div {
    let c: Hsla = st.color.into();
    let pill = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .pl(px(8.))
        .pr(px(10.))
        .py(px(2.))
        .rounded_full()
        .bg(Hsla { a: 0.14, ..c })
        .text_size(px(12.))
        .text_color(st.color)
        .whitespace_nowrap()
        .child(div().size(px(7.)).rounded_full().bg(st.color))
        .child(st.label);
    let size = if main { 50. } else { 40. };
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .p(px(if main { 20. } else { 16. }))
        .rounded(px(18.))
        .bg(if main { theme::accent_alpha(0.07).into() } else { theme::white_a(0.04) })
        .border_1()
        .border_color(if main { theme::accent_alpha(0.28).into() } else { theme::white_a(0.08) })
        .child(
            div()
                .flex_none()
                .size(px(size))
                .rounded(px(size * 0.3))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::accent_alpha(0.15))
                .child(icon(ic, px(size * 0.52), theme::accent())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .font_family(theme::font_heading())
                                .font_weight(theme::W_BOLD)
                                .text_size(px(if main { 19. } else { 16. }))
                                .text_color(theme::text_primary())
                                .child(title),
                        )
                        .child(pill),
                )
                .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(st.line))
                .when_some(note, |d, (text, color)| d.child(div().text_size(px(12.5)).line_height(px(17.)).text_color(color).child(text))),
        )
        .when_some(button, |d, b| d.child(div().flex_none().child(b)))
}

/// A store to log in to: its name over "Log in", tinted with its colour.
pub(super) fn store_tile(id: &'static str, name: &'static str, (r, g, b): (u8, u8, u8)) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(12.))
        .p(px(14.))
        .rounded(px(16.))
        .bg(theme::white_a(0.045))
        .border_1()
        .border_color(theme::white_a(0.09))
        .cursor_pointer()
        .hover(|s| s.bg(theme::accent_tint()).border_color(theme::accent_tint_border()))
        .child(
            div()
                .flex_none()
                .size(px(38.))
                .rounded(px(11.))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::rgba_hsla(r, g, b, 0.14))
                .child(icon(Icon::Storefront, px(20.), theme::rgba_hsla(r, g, b, 1.))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(15.))
                        .text_color(theme::text_primary())
                        .child(name),
                )
                .child(div().text_size(px(12.5)).text_color(theme::accent()).child(tr!("Log in"))),
        )
}
