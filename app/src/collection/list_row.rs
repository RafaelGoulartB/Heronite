//! A game of the Focus list drawn as one row (`focusList`): the cover's
//! thumbnail, the title and the card's caption line; the play button shows
//! under the pointer.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::card::{caption_line, CardLook};
use super::dock::tri_mark;
use super::epic::CardEpic;
use super::selection::Tri;
use super::vm::CardVm;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

const ROW_H: f32 = 44.;
pub(super) const THUMB_W: f32 = 26.;
/// Narrower rows drop the caption: the title needs the room.
const CAPTION_MIN_W: f32 = 220.;

pub(super) struct RowState<'a> {
    pub vm: &'a CardVm,
    pub cover: Option<ImageSource>,
    pub width: f32,
    pub thumb_h: f32,
    pub selected: bool,
    pub hovered: bool,
    pub grey_uninstalled: bool,
    /// Selecting several games: `Some(ticked)` draws the tick before the
    /// thumbnail and dims the rows left out.
    pub pick: Option<bool>,
    pub epic: Option<CardEpic>,
    pub look: CardLook,
}

pub(super) fn row(
    s: RowState<'_>,
    on_select: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let vm = s.vm;
    let grey = (s.grey_uninstalled && !vm.installed || vm.hidden) && !s.hovered;
    let left_out = s.pick == Some(false) && !s.hovered;
    let r = s.look.radius.min(4.);
    let thumb = div()
        .w(px(THUMB_W))
        .h(px(s.thumb_h))
        .flex_none()
        .rounded(px(r))
        .overflow_hidden()
        .bg(theme::ink_a(0.06))
        .when(vm.hidden && !s.hovered, |d| d.opacity(0.62))
        .children(s.cover.map(|src| img(src).size_full().rounded(px(r)).object_fit(ObjectFit::Cover).grayscale(grey)));
    let title = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(5.))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(14.))
                .font_weight(theme::W_500)
                .text_color(theme::ink_a(if grey { 0.6 } else { 0.9 }))
                .child(vm.title.clone()),
        )
        .when(s.look.badges && vm.favourite, |d| d.child(icon(Icon::Star, px(12.), theme::rgb_a(255, 209, 102, 1.))))
        .when(vm.hidden, |d| d.child(icon(Icon::VisibilityOff, px(12.), theme::ink_a(0.5))));
    let status = match &s.epic {
        Some(e) => Some(super::epic::row_status(e)),
        None => {
            (s.look.caption && s.width >= CAPTION_MIN_W).then(|| caption_line(vm).flex_none().max_w(px(s.width * 0.4)).text_size(px(13.)))
        }
    };
    let play_icon = if vm.playing {
        Icon::StopCircle
    } else if vm.installed {
        Icon::PlayCircle
    } else {
        Icon::InstallCircle
    };
    let play = (s.hovered && s.pick.is_none()).then(|| {
        div()
            .id(SharedString::from(format!("row-play-{}", vm.key)))
            .flex_none()
            .size(px(22.))
            .cursor_pointer()
            .on_click(move |e, w, cx| {
                cx.stop_propagation();
                on_play(e, w, cx)
            })
            .child(img(play_icon.path()).size(px(22.)))
    });
    let border = if vm.playing || s.pick == Some(true) {
        theme::accent().into()
    } else if s.selected {
        theme::accent_tint_border()
    } else {
        transparent_black()
    };
    div()
        .id(ElementId::Name(format!("row-{}", vm.key).into()))
        .flex()
        .items_center()
        .gap(px(10.))
        .w(px(s.width))
        .h(px(ROW_H))
        .px(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(border)
        .cursor_pointer()
        .when(s.selected, |d| d.bg(theme::accent_tint()))
        .when(!s.selected && s.hovered, |d| d.bg(theme::ink_a(0.06)))
        .when(left_out, |d| d.opacity(0.55))
        .on_click(on_select)
        .on_hover(on_hover)
        .children(s.pick.map(|ticked| tri_mark(if ticked { Tri::All } else { Tri::None })))
        .child(thumb)
        .child(title)
        .children(status)
        .children(play)
        .into_any_element()
}
