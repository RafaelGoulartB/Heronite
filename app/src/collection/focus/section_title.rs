//! A section title that is also its menu: while the section is pointed at, a
//! chevron fades and slides in after the title; a click on the title opens
//! the section's menu under it ("Screenshots": "Open folder"; "How Long To
//! Beat": its page, "Change match…", "Refresh"; "Details": "Edit rows…";
//! "Links": "Add links…").
//! Nothing takes room until it is wanted, and the title keeps its look.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::h3;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel, menu_separator};
use crate::ui::icon;

/// The sections whose title is a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::collection) enum TitleMenu {
    Screenshots,
    Details,
    Hltb,
    Links,
}

impl TitleMenu {
    fn id(self) -> &'static str {
        match self {
            Self::Screenshots => "screenshots",
            Self::Details => "details",
            Self::Hltb => "hltb",
            Self::Links => "links",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::collection) enum TitleAction {
    /// The pointer entered (`true`) or left the section.
    Hovered(TitleMenu, bool),
    /// A click on the title: opens its menu (closes it when open).
    Toggle(TitleMenu),
    /// A click outside the menu, or an item was picked.
    Close,
    /// Details → "Edit rows…".
    EditDetails,
}

pub(in crate::collection) type OnTitle = Rc<dyn Fn(TitleAction, &mut Window, &mut App)>;

/// What the titles show: the section pointed at and the open menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::collection) struct TitleState {
    pub hovered: Option<TitleMenu>,
    pub open: Option<TitleMenu>,
}

impl TitleState {
    /// After a hover event of `menu`. A late "left" of another section does
    /// not clear the one entered since.
    pub fn hovered(mut self, menu: TitleMenu, on: bool) -> Self {
        if on {
            self.hovered = Some(menu);
        } else if self.hovered == Some(menu) {
            self.hovered = None;
        }
        self
    }
}

/// The chevron's way in: a fade with a short slide from the left.
const CHEVRON_IN: Duration = Duration::from_millis(200);
const CHEVRON_SLIDE: f32 = 4.;

fn ease_out_cubic(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

/// One row of a title's menu.
pub(in crate::collection) struct MenuEntry {
    pub id: &'static str,
    pub icon: Icon,
    pub label: SharedString,
    pub run: Rc<dyn Fn(&mut Window, &mut App)>,
    /// A line above it.
    pub separated: bool,
}

/// `label` as a title that opens `entries` under it.
pub(in crate::collection) fn section_title(
    menu: TitleMenu,
    label: impl Into<SharedString>,
    state: TitleState,
    entries: Vec<MenuEntry>,
    on: &OnTitle,
) -> Div {
    let open = state.open == Some(menu);
    let show_chevron = open || state.hovered == Some(menu);
    let chevron = show_chevron.then(|| {
        let chevron = div().relative().flex().child(icon(Icon::ExpandMore, px(18.), theme::ink_a(0.6)));
        if theme::animations() {
            chevron
                .with_animation(
                    SharedString::from(format!("focus-title-{}-chevron", menu.id())),
                    Animation::new(CHEVRON_IN).with_easing(ease_out_cubic),
                    |el, t| el.left(px(-CHEVRON_SLIDE * (1. - t))).opacity(t),
                )
                .into_any_element()
        } else {
            chevron.into_any_element()
        }
    });
    let toggle = on.clone();
    let title = h3(label)
        .id(SharedString::from(format!("focus-title-{}", menu.id())))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .cursor_pointer()
        .on_click(move |_, window, cx| toggle(TitleAction::Toggle(menu), window, cx))
        .children(chevron);
    div().relative().flex().child(title).children(open.then(|| title_menu(menu, entries, on)))
}

/// The menu under a title, over a window-wide catcher that closes it (the
/// layering of the Install menu).
fn title_menu(menu: TitleMenu, entries: Vec<MenuEntry>, on: &OnTitle) -> AnyElement {
    let mut panel = menu_panel(230.);
    for entry in entries {
        if entry.separated {
            panel = panel.child(menu_separator());
        }
        let (run, close) = (entry.run.clone(), on.clone());
        panel = panel.child(
            menu_item(SharedString::from(format!("focus-title-{}-{}", menu.id(), entry.id)), false)
                .child(icon(entry.icon, px(16.), theme::ink_a(0.6)))
                .child(entry.label)
                .on_click(move |_, window, cx| {
                    close(TitleAction::Close, window, cx);
                    run(window, cx);
                }),
        );
    }
    let close = on.clone();
    div()
        .absolute()
        .top_0()
        .left_0()
        .child(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .id(SharedString::from(format!("focus-title-{}-catcher", menu.id())))
                        .w(px(20000.))
                        .h(px(20000.))
                        .occlude()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| close(TitleAction::Close, window, cx)),
                ),
            )
            .with_priority(20),
        )
        .child(
            div().absolute().top(px(28.)).left_0().child(
                deferred(
                    anchored()
                        .snap_to_window_with_margin(px(8.))
                        .child(div().occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(panel)),
                )
                .with_priority(21),
            ),
        )
        .into_any_element()
}
