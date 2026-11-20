//! The sidebar of console Settings: the app's icon, name and version, then
//! Search, Changed and every section under its group heading, in one list
//! that scrolls to keep the stop shown in sight. That stop is a solid focus
//! pill while the sidebar has the focus, a faint one while its rows do.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{entries, Entry, SettingsPage, GROUPS};
use crate::console::kit::{self, u};
use crate::console::motion::{self, Goal};
use crate::console::pointer::{self, Clickable};
use crate::console::Pad;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// The app's icon (the one the window and the launcher show).
const APP_ICON: &[u8] = include_bytes!("../../../../assets/app-icon/graphite.png");

thread_local! {
    static ICON_IMAGE: Arc<Image> = Arc::new(Image::from_bytes(ImageFormat::Png, APP_ICON.to_vec()));
}

/// Width of the sidebar.
pub(super) const WIDTH: f32 = 290.;

/// The icon and name of a stop.
fn look(e: Entry) -> (Icon, SharedString) {
    match e {
        Entry::Search => (Icon::FaSearch, tr!("Search").into()),
        Entry::Changed => (Icon::History, tr!("Changed").into()),
        Entry::Sec(s) => (s.icon(), s.label().into()),
    }
}

/// The app's icon, name and version.
fn header() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(u(14.))
        .pl(u(10.))
        .pb(u(18.))
        .child(
            div()
                .flex_none()
                .size(u(42.))
                .rounded(u(10.))
                .overflow_hidden()
                .child(img(ICON_IMAGE.with(Arc::clone)).size_full().object_fit(ObjectFit::Contain)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .font_family(kit::heading_font())
                        .font_weight(theme::W_BOLD)
                        .text_size(u(22.))
                        .line_height(u(26.))
                        .text_color(theme::text_primary())
                        .child("Heronite"),
                )
                .child(
                    div()
                        .text_size(u(13.))
                        .text_color(theme::text_muted())
                        .truncate()
                        .child(tr!("Version {version}", version = env!("CARGO_PKG_VERSION"))),
                ),
        )
}

/// A group heading (not a stop).
fn heading(text: &str) -> Div {
    div()
        .flex_none()
        .pl(u(16.))
        .pt(u(14.))
        .pb(u(3.))
        .text_size(u(13.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_dim())
        .child(text.to_string())
}

pub(super) fn render(p: &mut SettingsPage) -> Div {
    let all = entries();
    let nav = p.console.nav.min(all.len() - 1);
    let content = p.console.content;
    let layer_open = p.console.layer.is_open();
    let mut list =
        div().id("console-settings-side").flex().flex_col().gap(u(2.)).pb(u(20.)).overflow_y_scroll().track_scroll(p.console.side.handle());
    // The index of each stop among the list's children (the headings are
    // children too), for the scroll.
    let mut child = 0;
    let mut focus_child = 0;
    let mut group = None;
    for (i, (e, g)) in all.iter().enumerate() {
        if *g != group {
            if let Some(g) = g {
                list = list.child(heading(heronite_i18n::tr(GROUPS[*g].0)));
                child += 1;
            }
            group = *g;
        }
        if i == nav {
            focus_child = child;
        }
        list = list.child(stop(*e, i, nav, content, layer_open));
        child += 1;
    }
    if std::mem::take(&mut p.console.side_reveal) {
        p.console.side.go(Goal::Reveal { ix: focus_child, margin: 48. });
    }
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(u(WIDTH))
        .h_full()
        .child(header())
        .child(div().flex().flex_col().flex_1().min_h_0().child(list.child(p.console.side.driver())))
}

/// One stop: its icon and name, on the focus pill when it is shown and the
/// sidebar has the focus.
fn stop(e: Entry, i: usize, nav: usize, content: bool, layer_open: bool) -> AnyElement {
    let (ic, label) = look(e);
    let on = i == nav;
    let lit = on && !content && !layer_open;
    // A click: back from the content (B), ↑ ↓ to the stop; on the one shown, A.
    let mut pads = if content { vec![Pad::B] } else { Vec::new() };
    pads.extend(pointer::along(nav, i, Pad::Up, Pad::Down));
    // The mouse over a stop shows it while the sidebar has the focus.
    let hover = if content { Vec::new() } else { pads.clone() };
    if pads.is_empty() {
        pads.push(Pad::A);
    }
    let rest: Hsla = if on { kit::ink(0.08) } else { transparent_black() };
    let row = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(u(14.))
        .h(u(40.))
        .px(u(16.))
        .rounded(u(12.))
        .when(!layer_open, |d| d.on_hover_pads(hover).on_pads(pads));
    let id = SharedString::from(format!("settings-stop-{i}"));
    motion::spring(id, row, motion::SOFT, lit, move |d, k| {
        let k = k.clamp(0., 1.);
        let fill = motion::mix(rest, theme::console_focus(), k);
        let ink = motion::mix(theme::text_primary(), theme::console_on_focus(), k);
        d.bg(fill).child(icon(ic, u(20.), ink)).child(div().flex_1().min_w_0().truncate().text_size(u(16.)).text_color(ink).child(label))
    })
}
