//! "Make it yours": a profile, the theme, the accent colour, the corners of
//! the covers and the view the Collection opens in, next to a preview drawn
//! with the current look. Each choice applies at once, as in Settings →
//! Appearance.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{presets, Accent, Appearance, Corners, StartView, Theme};
use heronite_i18n::{tr, trc};

use super::art::{self, Art};
use super::steps::{Action, Step};
use super::OnboardingDialog;
use crate::icons::Icon;
use crate::pages::settings::Section;
use crate::theme;
use crate::ui::controls::{button_sm, mini_checkbox, tabs, ButtonKind};
use crate::ui::icon;

const PICKER_W: f32 = 430.;
const PREVIEW_W: f32 = 600.;

/// `look` is the built-in `preset` with any start view (and window-width
/// switch): here the view is chosen on its own, so it neither changes nor
/// breaks a profile.
pub(super) fn is_preset(look: Appearance, preset: Appearance) -> bool {
    Appearance { start_view: preset.start_view, ..look }.with_window_choices_of(&preset) == preset
}

pub(super) fn preset_name(id: &str) -> &'static str {
    match id {
        "cinema" => tr!("Cinema"),
        "minimal" => tr!("Minimal"),
        "contrast" => tr!("High contrast"),
        _ => tr!("Standard"),
    }
}

fn preset_note(id: &str) -> &'static str {
    match id {
        "cinema" => tr!("The art comes first"),
        "minimal" => tr!("Solid and quiet"),
        "contrast" => tr!("Easy to read"),
        _ => tr!("Today's look"),
    }
}

pub(super) fn accent_name(a: Accent) -> &'static str {
    match a {
        Accent::Ice => tr!("Ice"),
        Accent::Mint => tr!("Mint"),
        Accent::Amber => tr!("Amber"),
        Accent::Rose => tr!("Rose"),
        Accent::Lavender => tr!("Lavender"),
        Accent::Game => tr!("From the game"),
    }
}

/// A round swatch of the accent in the theme's shade (a blend for "From the game").
pub(super) fn accent_dot(a: Accent, size: f32) -> Div {
    let d = div().flex_none().size(px(size)).rounded_full();
    match theme::accent_shade(a) {
        Some((r, g, b)) => d.bg(theme::rgba_hsla(r, g, b, 1.)),
        None => d.bg(linear_gradient(135., linear_color_stop(rgb(0xff9a6b), 0.), linear_color_stop(rgb(0x7ec7ff), 1.))),
    }
}

/// A small picture of a built-in look (44x32).
fn swatch(id: &str) -> Div {
    let base = div().relative().flex_none().w(px(44.)).h(px(32.)).rounded(px(8.)).overflow_hidden();
    let panel = || div().absolute().left(px(5.)).top(px(5.)).bottom(px(5.)).w(relative(0.38)).rounded(px(3.));
    match id {
        "cinema" => base
            .bg(linear_gradient(120., linear_color_stop(rgb(0x3a2a52), 0.), linear_color_stop(rgb(0xffcf8f), 1.)))
            .child(div().absolute().left(px(5.)).bottom(px(5.)).w(relative(0.5)).h(px(4.)).rounded_full().bg(theme::white_a(0.85))),
        "minimal" => base.bg(rgb(0x1a1a1c)).child(panel().bg(rgb(0x26262a))),
        "contrast" => base
            .bg(linear_gradient(120., linear_color_stop(rgb(0x0c0f14), 0.), linear_color_stop(rgb(0x1d2230), 1.)))
            .child(panel().border_2().border_color(theme::white_a(0.75))),
        _ => base
            .bg(linear_gradient(120., linear_color_stop(rgb(0x2a3346), 0.), linear_color_stop(rgb(0xc3876d), 1.)))
            .child(panel().bg(theme::rgba_hsla(14, 18, 24, 0.6))),
    }
}

fn caption(text: &'static str) -> Div {
    div().mt(px(18.)).mb(px(10.)).text_size(px(11.5)).text_color(theme::text_dim()).child(text.to_uppercase())
}

impl OnboardingDialog {
    pub(super) fn render_look(&self, art: &Art, cx: &mut Context<Self>) -> Div {
        let a = theme::appearance();
        let profiles = presets().into_iter().map(|p| {
            let on = is_preset(a, p.look);
            let look = p.look;
            div()
                .id(SharedString::from(format!("onb-profile-{}", p.id)))
                .flex()
                .items_center()
                .gap(px(10.))
                .w(relative(0.485))
                .p(px(7.))
                .rounded(px(14.))
                .bg(if on { theme::accent_alpha(0.09).into() } else { theme::ink_a(0.04) })
                .border_2()
                .border_color(if on { theme::accent().into() } else { theme::ink_a(0.08) })
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.07)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_look(|l| *l = Appearance { start_view: l.start_view, ..look }.with_window_choices_of(l), cx)
                }))
                .child(swatch(p.id))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().text_size(px(13.5)).font_weight(theme::W_BOLD).child(preset_name(p.id)))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(theme::text_muted())
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(preset_note(p.id)),
                        ),
                )
        });
        let accents = Accent::ALL.iter().copied().map(|accent| {
            let on = a.accent_own.is_none() && a.accent == accent;
            div()
                .id(SharedString::from(format!("onb-accent-{}", accent.as_str())))
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(6.))
                .pr(px(12.))
                .py(px(5.))
                .rounded_full()
                .bg(theme::ink_a(0.04))
                .border_2()
                .border_color(if on { theme::accent().into() } else { theme::ink_a(0.08) })
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.08)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_look(
                        |l| {
                            l.accent = accent;
                            l.accent_own = None;
                        },
                        cx,
                    )
                }))
                .child(accent_dot(accent, 20.))
                .child(div().text_size(px(13.)).child(accent_name(accent)))
        });
        let this = cx.entity();
        let theme_tabs = tabs(
            "onb-theme",
            &[(Theme::Dark, trc!("theme", "Dark")), (Theme::Light, trc!("theme", "Light")), (Theme::System, trc!("theme", "System"))],
            a.theme,
            {
                let this = this.clone();
                move |v, _, cx| this.update(cx, |d, cx| d.set_look(|l| l.theme = v, cx))
            },
        );
        let corners = tabs(
            "onb-corners",
            &[(Corners::Square, tr!("Square")), (Corners::Soft, tr!("Soft")), (Corners::Round, tr!("Round"))],
            a.cover_corners,
            move |v, _, cx| this.update(cx, |d, cx| d.set_look(|l| l.cover_corners = v, cx)),
        );
        let picker = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(PICKER_W))
            .child(caption(tr!("Profile")).mt_0())
            .child(div().flex().flex_wrap().justify_between().gap_y(px(8.)).children(profiles))
            .child(caption(tr!("Theme")))
            .child(theme_tabs)
            .child(caption(tr!("Accent colour")))
            .child(div().flex().flex_wrap().gap(px(8.)).children(accents))
            .child(caption(tr!("Corners of the covers")))
            .child(corners)
            .child(
                div().mt(px(20.)).flex().child(
                    button_sm("onb-look-more", ButtonKind::Ghost, tr!("Background, font, motion and more"))
                        .on_click(cx.listener(|this, _, _, cx| this.go(Step::Appearance, Action::Settings(Section::Appearance), cx))),
                ),
            );
        let note = if art.has_games() { tr!("Preview with your Collection") } else { tr!("Your games will show here") };
        div().flex().gap(px(36.)).child(picker).child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(12.))
                .child(self.render_views(cx))
                .child(art::collection_preview(art, PREVIEW_W, self.preview_grid()))
                .child(
                    div()
                        .flex()
                        .justify_center()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.5))
                        .text_color(theme::text_dim())
                        .child(icon(Icon::Visibility, px(14.), theme::text_dim()))
                        .child(note),
                ),
        )
    }

    /// The two ways the Collection opens, as big choices over the preview,
    /// and "Remember the last one used".
    fn render_views(&self, cx: &mut Context<Self>) -> Div {
        let last = theme::appearance().start_view == StartView::Last;
        let grid = self.preview_grid();
        let view = |id: &'static str, ic: Icon, title: &'static str, desc: &'static str, value: StartView, on: bool| {
            div()
                .id(id)
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(14.))
                .bg(if on { theme::accent_alpha(0.1).into() } else { theme::ink_a(0.04) })
                .border_2()
                .border_color(if on { theme::accent().into() } else { theme::ink_a(0.08) })
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.07)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_look(|l| l.start_view = value, cx)))
                .child(
                    div()
                        .flex_none()
                        .size(px(32.))
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(if on { theme::accent_alpha(0.2) } else { theme::accent_alpha(0.1) })
                        .child(icon(ic, px(17.), theme::accent())),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(14.)).child(title))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(desc),
                        ),
                )
        };
        let current = if grid { StartView::Grid } else { StartView::Focus };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .text_color(theme::text_dim())
                            .child(tr!("Open the Collection in").to_uppercase()),
                    )
                    .child(
                        div()
                            .id("onb-view-last")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(12.5))
                            .text_color(theme::text_muted())
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let next = if last { current } else { StartView::Last };
                                this.set_look(|l| l.start_view = next, cx)
                            }))
                            .child(tr!("Remember the last one used"))
                            .child(mini_checkbox("onb-view-last-sw", last, false)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(view(
                        "onb-view-focus",
                        Icon::ViewSidebar,
                        tr!("Focus"),
                        tr!("A list beside the selected game's page"),
                        StartView::Focus,
                        !grid,
                    ))
                    .child(view(
                        "onb-view-grid",
                        Icon::GridView,
                        tr!("Grid"),
                        tr!("Covers across the window, the game in a drawer"),
                        StartView::Grid,
                        grid,
                    )),
            )
    }
}
