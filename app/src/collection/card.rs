//! `CollectionCard` in the focused layout: 125px cover (2/3), radius 6,
//! soft shadow, playtime label below, hover bar with the play button.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::vm::CardVm;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

pub(super) struct CardState<'a> {
    pub vm: &'a CardVm,
    pub cover: Option<ImageSource>,
    /// No source of the cover chain loaded: the card shows the title.
    pub no_art: bool,
    pub cover_h: f32,
    pub selected: bool,
    pub hovered: bool,
    /// The pointer just left this card: play the bar's slide-out.
    pub leaving: bool,
    pub grey_uninstalled: bool,
    /// Epic install / update progress and update hint.
    pub epic: Option<super::epic::CardEpic>,
}

pub fn card(
    s: CardState<'_>,
    on_select: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let vm = s.vm;
    let ring: Option<Rgba> = if vm.playing {
        Some(theme::accent())
    } else if s.selected {
        Some(theme::selected_ring())
    } else {
        None
    };

    // `box-shadow: 0 0 0 2px <ring>, 0 10px 24px rgba(0,0,0,.42)` — the ring
    // is drawn as a 2px border just outside the cover.
    let shadow = if ring.is_some() {
        BoxShadow { color: theme::black_a(0.42), offset: point(px(0.), px(10.)), blur_radius: px(24.), spread_radius: px(0.), inset: false }
    } else {
        BoxShadow { color: theme::black_a(0.38), offset: point(px(0.), px(8.)), blur_radius: px(20.), spread_radius: px(0.), inset: false }
    };

    let grey = (s.grey_uninstalled && !vm.installed || vm.hidden) && !s.hovered;
    let cover_img = match s.cover {
        Some(src) => {
            // GPUI does not clip images to a parent's rounded corners: round the image itself.
            let el = img(src).size_full().rounded(px(6.)).object_fit(ObjectFit::Cover).grayscale(grey);
            if theme::animations() {
                el.with_animation(
                    ElementId::Name(format!("cover-in-{}", vm.key).into()),
                    Animation::new(Duration::from_millis(500)),
                    |el, t| el.opacity(t),
                )
                .into_any_element()
            } else {
                el.into_any_element()
            }
        }
        // No art at all: the title, so the card still says which game it is.
        None if s.no_art => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(12.))
            .bg(theme::rgb_a(255, 255, 255, 0.05))
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(px(14.))
                    .line_height(px(19.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::white_a(0.78))
                    .child(vm.title.clone()),
            )
            .into_any_element(),
        None => div().size_full().into_any_element(),
    };

    // Hover bar: slides up from translateY(100%) in 200ms.
    let bar_id = format!("bar-{}-{}", vm.key, s.hovered);
    let play_icon = if vm.playing {
        Icon::StopCircle
    } else if vm.installed {
        Icon::PlayCircle
    } else {
        Icon::InstallCircle
    };
    let hovered = s.hovered;
    let show_bar = s.hovered || s.leaving;
    const BAR_H: f32 = 24. + 2. * 5.6;
    let bar = div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .justify_end()
        .p(px(5.6))
        .bg(linear_gradient(180., linear_color_stop(theme::black_a(0.), 0.), linear_color_stop(theme::black_a(0.55), 1.)))
        .child(
            div()
                .id(SharedString::from(format!("play-{}", vm.key)))
                .size(px(24.))
                .cursor_pointer()
                .on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    on_play(e, w, cx)
                })
                .child(img(play_icon.path()).size(px(24.))),
        )
        .with_animation(
            ElementId::Name(bar_id.into()),
            Animation::new(Duration::from_millis(200)).with_easing(ease_in_out),
            move |el, t| {
                let shown = if hovered { t } else { 1. - t };
                el.bottom(px(-BAR_H * (1. - shown)))
            },
        );

    div()
        .id(ElementId::Name(format!("card-{}", vm.key).into()))
        .flex()
        .flex_col()
        .w(px(theme::CARD_W))
        .cursor_pointer()
        .on_click(on_select)
        .on_hover(on_hover)
        .child(
            div()
                .relative()
                .w(px(theme::CARD_W))
                .h(px(s.cover_h))
                .rounded(px(6.))
                .shadow(vec![shadow])
                .bg(theme::rgb_a(255, 255, 255, 0.03))
                .child(
                    div().size_full().rounded(px(6.)).overflow_hidden().when(vm.hidden && !s.hovered, |d| d.opacity(0.62)).child(cover_img),
                )
                .children(s.epic.as_ref().map(super::epic::card_overlay))
                .children(vm.hidden.then(hidden_badge))
                .children(ring.map(|color| {
                    div()
                        .absolute()
                        .top(px(-2.))
                        .left(px(-2.))
                        .right(px(-2.))
                        .bottom(px(-2.))
                        .rounded(px(8.))
                        .border_2()
                        .border_color(color)
                }))
                .when(show_bar, |d| d.child(div().absolute().inset_0().rounded(px(6.)).overflow_hidden().child(bar))),
        )
        .child(playtime_label(vm))
        .into_any_element()
}

/// "Hidden" tag on the top-left of a hidden game shown by "Show hidden games".
fn hidden_badge() -> Div {
    div()
        .absolute()
        .top(px(6.))
        .left(px(6.))
        .flex()
        .items_center()
        .gap(px(3.))
        .px(px(6.))
        .py(px(2.))
        .rounded(px(4.))
        .bg(theme::black_a(0.62))
        .text_size(px(10.5))
        .font_weight(theme::W_BOLD)
        .text_color(theme::white_a(0.9))
        .child(icon(Icon::VisibilityOff, px(11.), theme::white_a(0.9)))
        .child("Hidden")
}

fn playtime_label(vm: &CardVm) -> Div {
    div()
        .mt(px(6.))
        .px(px(1.))
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(12.))
        .font_weight(theme::W_500)
        .text_color(theme::white_a(0.72))
        // MUI's 1.5rem SvgIcon size wins over the 14px rule in Heronite.
        .child(icon(Icon::AccessTime, px(24.), theme::white_a(0.72 * 0.78)))
        .when(vm.playing, |d| {
            d.child(div().size(px(6.)).rounded_full().bg(theme::success()).with_animation(
                ElementId::Name(format!("live-{}", vm.key).into()),
                Animation::new(Duration::from_millis(1600)).repeat().with_easing(ease_in_out),
                |el, t| el.opacity(0.4 + 0.6 * (1. - (2. * t - 1.).abs())),
            ))
        })
        .child(vm.playtime.clone())
}
