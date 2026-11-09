//! `CollectionCard`: cover (125px in the list, the grid's width in the grid),
//! radius 6 (Settings → Appearance → Corners), soft shadow, playtime label
//! below (the title above it when the grid shows titles), hover bar with the
//! play button, the hover effect of [`CardLook`], the game's categories
//! (`category_badges`) and its stores (`store_badges`).

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use heronite_data::appearance::{CategoryStyle, CoverHover};

use super::category_badges::{self as cats, CategoryBadge};
use super::store_badges as stores;
use super::vm::CardVm;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;
use heronite_i18n::tr;

/// How every card looks (Settings → Appearance → Covers).
#[derive(Clone, Copy)]
pub(super) struct CardLook {
    /// Corner radius of the cover, px.
    pub radius: f32,
    pub hover: CoverHover,
    /// A star on the covers of favourites.
    pub badges: bool,
    /// The line under the cover (playtime, sort value, status...).
    pub caption: bool,
    /// How the game's categories show; `None` while they are off.
    pub categories: Option<CategoryStyle>,
    /// The logos of the game's stores at the bottom-left of the cover.
    pub stores: bool,
    /// How far their chip sits from the cover's left and bottom edges, px:
    /// closer on the Focus view's small covers.
    pub stores_inset: f32,
}

pub(super) struct CardState<'a> {
    pub vm: &'a CardVm,
    pub cover: Option<ImageSource>,
    /// No source of the cover chain loaded: the card shows the title.
    pub no_art: bool,
    pub width: f32,
    pub cover_h: f32,
    /// The title under the cover (grid).
    pub title: bool,
    pub selected: bool,
    pub hovered: bool,
    /// The pointer just left this card: play the bar's slide-out.
    pub leaving: bool,
    pub grey_uninstalled: bool,
    /// Selecting several games: `Some(ticked)` draws the tick circle, dims
    /// the cards left out and drops the play bar.
    pub pick: Option<bool>,
    /// Epic install / update progress and update hint.
    pub epic: Option<super::epic::CardEpic>,
    pub look: CardLook,
    /// The game's categories (empty while [`CardLook::categories`] is off).
    pub categories: Vec<CategoryBadge>,
    /// The logos of the game's stores, its own first (empty while
    /// [`CardLook::stores`] is off).
    pub stores: Vec<Icon>,
    /// The window's scale factor (the categories' SVG shapes).
    pub scale: f32,
}

pub fn card(
    s: CardState<'_>,
    on_select: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let vm = s.vm;
    let look = s.look;
    let r = look.radius;
    // Hover effects stay off while picking games (the tick says it all).
    let lift = s.hovered && s.pick.is_none() && look.hover == CoverHover::Lift;
    let glow = s.hovered && s.pick.is_none() && look.hover == CoverHover::Glow;
    // While picking games the halo marks where the keyboard is.
    let halo = s.selected && !vm.playing;
    let ring: Option<Rgba> = if vm.playing || s.pick == Some(true) {
        Some(theme::accent())
    } else if s.selected {
        Some(theme::selected_ring())
    } else if glow {
        Some(theme::accent_alpha(0.7))
    } else {
        None
    };

    // `box-shadow: 0 0 0 2px <ring>, 0 10px 24px rgba(0,0,0,.42)` — the ring
    // is drawn as a 2px border just outside the cover.
    let shadow = if lift {
        BoxShadow { color: theme::shadow_a(0.5), offset: point(px(0.), px(16.)), blur_radius: px(30.), spread_radius: px(0.), inset: false }
    } else if ring.is_some() {
        BoxShadow {
            color: theme::shadow_a(0.42),
            offset: point(px(0.), px(10.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
            inset: false,
        }
    } else {
        BoxShadow { color: theme::shadow_a(0.38), offset: point(px(0.), px(8.)), blur_radius: px(20.), spread_radius: px(0.), inset: false }
    };

    let grey = (s.grey_uninstalled && !vm.installed || vm.hidden) && !s.hovered;
    // The categories: the circles join the star's row; the corner styles
    // give way to the selection's tick and push the copies badge left.
    let style = look.categories.filter(|_| !s.categories.is_empty());
    let area = cats::Area { w: s.width, radius: r, scale: s.scale, unit: 1., play_room: true };
    let circles = if style == Some(CategoryStyle::Circles) { cats::circles(&s.categories, area) } else { Vec::new() };
    let corner_taken = style.is_some_and(cats::takes_top_right);
    // The stores' chip sits at the bottom-left: first in the row of the
    // tags, over the strip while it shows, gone while an install's progress
    // takes the bottom of the cover.
    let installing = s.epic.as_ref().is_some_and(|e| !e.label.is_empty());
    let store_chip = if installing { None } else { stores::chip(&s.stores) };
    // Each store's logo already tells the copies apart: no "⧉ 2" then.
    let copies_told = store_chip.is_some() && s.stores.len() > 1 && s.stores.len() >= vm.copies;
    let (category_overlay, store_overlay) = match (style, store_chip) {
        (Some(CategoryStyle::Tags), Some(chip)) => (Some(cats::tags_after(&s.categories, area, chip).into_any_element()), None),
        (_, chip) => {
            let over_band = style == Some(CategoryStyle::Band) && !show_bar_now(&s);
            (
                style
                    .filter(|st| *st != CategoryStyle::Circles && !(s.pick.is_some() && cats::takes_top_right(*st)))
                    .and_then(|st| cats::overlay(st, &s.categories, area, show_bar_now(&s))),
                chip.map(|c| {
                    let inset = look.stores_inset;
                    div().absolute().left(px(inset)).bottom(px(if over_band { 20. + inset } else { inset })).child(c)
                }),
            )
        }
    };
    let copies = vm.copies > 1 && s.pick.is_none() && !copies_told;
    let copies_left = corner_taken && copies;
    let cover_img = match s.cover {
        Some(src) => {
            // GPUI does not clip images to a parent's rounded corners: round the image itself.
            let el = img(src).size_full().rounded(px(r)).object_fit(ObjectFit::Cover).grayscale(grey);
            if theme::fades() {
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
            .bg(theme::ink_a(0.05))
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(px(14.))
                    .line_height(px(19.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::ink_a(0.78))
                    .child(vm.title.clone()),
            )
            .into_any_element(),
        // Still loading: a soft pulse where the cover goes.
        None => {
            let wait = div().size_full().bg(theme::ink_a(0.08));
            if theme::animations() {
                wait.with_animation(
                    ElementId::Name(format!("cover-wait-{}", vm.key).into()),
                    Animation::new(Duration::from_millis(1400)).repeat().with_max_fps(30.),
                    |el, t| el.opacity(0.35 + 0.65 * (1. - (2. * t - 1.).abs())),
                )
                .into_any_element()
            } else {
                wait.into_any_element()
            }
        }
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
    let show_bar = s.pick.is_none() && (s.hovered || s.leaving);
    let left_out = s.pick == Some(false) && !s.hovered;
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
        .w(px(s.width))
        .cursor_pointer()
        .on_click(on_select)
        .on_hover(on_hover)
        .child(
            div()
                .relative()
                .when(lift, |d| d.top(px(-LIFT)))
                .w(px(s.width))
                .h(px(s.cover_h))
                .rounded(px(r))
                .shadow(vec![shadow])
                .bg(theme::ink_a(0.03))
                .child(
                    div()
                        .size_full()
                        .rounded(px(r))
                        .overflow_hidden()
                        .when(vm.hidden && !s.hovered, |d| d.opacity(0.62))
                        .when(left_out, |d| d.opacity(0.55))
                        .child(cover_img),
                )
                .when(glow, |d| d.child(div().absolute().inset_0().rounded(px(r)).bg(theme::white_a(0.07))))
                .children(s.epic.as_ref().map(|e| super::epic::card_overlay(e, r)))
                .children((vm.hidden || look.badges && vm.favourite || vm.family.is_some() || copies_left || !circles.is_empty()).then(
                    || {
                        top_badges(vm.hidden, look.badges && vm.favourite, vm.family.is_some())
                            .when(copies_left, |d| d.child(copies_badge(vm.copies).relative().top_0().right_0()))
                            .children(circles)
                    },
                ))
                .children((copies && !copies_left).then(|| copies_badge(vm.copies)))
                .children(ring.map(|color| {
                    div()
                        .absolute()
                        .top(px(-2.))
                        .left(px(-2.))
                        .right(px(-2.))
                        .bottom(px(-2.))
                        .rounded(px(r + 2.))
                        .border_2()
                        .border_color(color)
                }))
                // The selected card also gets a halo around its ring, to find it
                // at a glance (keyboard, controller, across the room).
                .when_some(ring.filter(|_| halo), |d, color| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(-5.))
                            .left(px(-5.))
                            .right(px(-5.))
                            .bottom(px(-5.))
                            .rounded(px(r + 5.))
                            .border(px(3.))
                            .border_color(Rgba { a: 0.14, ..color }),
                    )
                })
                .when(show_bar, |d| d.child(div().absolute().inset_0().rounded(px(r)).overflow_hidden().child(bar)))
                .children(category_overlay)
                .children(store_overlay)
                .children(s.pick.map(tick_circle)),
        )
        .when(s.title, |d| {
            d.child(
                div()
                    .mt(px(7.))
                    .px(px(1.))
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(theme::W_500)
                    .text_color(theme::ink_a(0.88))
                    .child(vm.title.clone()),
            )
        })
        .when(look.caption, |d| d.child(caption_line(vm).mt(px(if s.title { 1. } else { 6. })).px(px(1.))))
        .into_any_element()
}

/// The play bar is up (or on its way down): the band of categories makes
/// room for it.
fn show_bar_now(s: &CardState<'_>) -> bool {
    s.pick.is_none() && (s.hovered || s.leaving)
}

/// How far a cover rises under the pointer ([`CoverHover::Lift`]), px.
const LIFT: f32 = 4.;

/// The selection's tick on the top-right of the cover: filled while ticked.
fn tick_circle(ticked: bool) -> Div {
    let d =
        div().absolute().top(px(6.)).right(px(6.)).size(px(24.)).rounded_full().flex().items_center().justify_center().border_2().shadow(
            vec![BoxShadow {
                color: theme::black_a(0.4),
                offset: point(px(0.), px(2.)),
                blur_radius: px(8.),
                spread_radius: px(0.),
                inset: false,
            }],
        );
    if ticked {
        d.bg(theme::accent()).border_color(theme::accent()).child(icon(Icon::Check, px(16.), theme::page_bg()))
    } else {
        d.bg(theme::black_a(0.45)).border_color(theme::white_a(0.88))
    }
}

/// Tags on the top-left of the cover: "Hidden" on a hidden game shown by
/// "Show hidden games", a star on a favourite (when badges are on),
/// "Family" on a game the Steam Family shares.
fn top_badges(hidden: bool, favourite: bool, family: bool) -> Div {
    let tag = || div().flex().items_center().gap(px(3.)).px(px(6.)).py(px(2.)).rounded(px(4.)).bg(theme::black_a(0.62));
    div()
        .absolute()
        .top(px(6.))
        .left(px(6.))
        .flex()
        .gap(px(4.))
        .text_size(px(10.5))
        .font_weight(theme::W_BOLD)
        .text_color(theme::white_a(0.9))
        .when(hidden, |d| d.child(tag().child(icon(Icon::VisibilityOff, px(11.), theme::white_a(0.9))).child(tr!("Hidden"))))
        .when(favourite, |d| d.child(tag().px(px(4.)).child(icon(Icon::Star, px(12.), theme::rgb_a(255, 209, 102, 1.)))))
        .when(family, |d| d.child(tag().child(icon(Icon::AccountCircle, px(11.), theme::white_a(0.9))).child(tr!("Family"))))
}

/// The number of copies on the top-right of the cover of a game owned in
/// several stores (the selection's tick takes that corner while picking).
fn copies_badge(copies: usize) -> Div {
    div()
        .absolute()
        .top(px(6.))
        .right(px(6.))
        .flex()
        .items_center()
        .gap(px(3.))
        .px(px(5.))
        .py(px(2.))
        .rounded(px(4.))
        .bg(theme::black_a(0.62))
        .text_size(px(10.5))
        .font_weight(theme::W_BOLD)
        .text_color(theme::white_a(0.9))
        .child(icon(Icon::ContentCopy, px(11.), theme::white_a(0.9)))
        .child(copies.to_string())
}

/// The playtime, or the value the list is sorted by (`CardVm::caption`).
pub(super) fn caption_line(vm: &CardVm) -> Div {
    let c = &vm.caption;
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(12.))
        .font_weight(theme::W_500)
        .text_color(theme::ink_a(theme::contrast_a(0.72)))
        // Material icons fill about 5/6 of their box, the drive icon all of it:
        // both come out the height of the text.
        .child(icon(
            c.icon,
            px(if c.icon == Icon::FaHardDrive { 14. } else { 16. }),
            c.tint.map(theme::readable).unwrap_or(theme::ink_a(theme::contrast_a(0.72 * 0.78))),
        ))
        .when(c.muted, |d| d.text_color(theme::ink_a(theme::contrast_a(0.4))))
        .when(vm.playing, |d| {
            d.child(div().size(px(6.)).rounded_full().bg(theme::success()).with_animation(
                ElementId::Name(format!("live-{}", vm.key).into()),
                Animation::new(Duration::from_millis(1600)).repeat().with_max_fps(20.).with_easing(ease_in_out),
                |el, t| el.opacity(0.4 + 0.6 * (1. - (2. * t - 1.).abs())),
            ))
        })
        .map(|d| match c.stars {
            Some(n) => d.child(div().flex().items_center().children((1..=5u8).map(|i| {
                icon(Icon::Star, px(13.), if i <= n { theme::readable(theme::rgb_a(244, 211, 94, 1.)) } else { theme::ink_a(0.2) })
            }))),
            None => d.child(div().min_w_0().truncate().child(c.text.clone())),
        })
}
