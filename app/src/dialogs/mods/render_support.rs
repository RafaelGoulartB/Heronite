//! What to expect of mods here: the "Experimental" label of the manager and
//! the game's own label (`heronite_mods::support`), each with its reason
//! under the pointer.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::support::{Level, Support};

use super::{ModsDialog, View};
use crate::pages::{status_pill, tag_pill};
use crate::theme;
use crate::ui::{titled_tip_box, tooltip_titled};

/// A label of the header whose reason is drawn open (`HERONITE_DEBUG_MODS=tip:…`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tip {
    Experimental,
    Support,
}

/// The game's level, from the table and from how it runs here.
pub(super) fn support_of(v: &View) -> Support {
    let info = &v.g.info;
    Support::of(info.known, info.profile.kind, info.native)
}

/// "Experimental", in the manager's header and in Settings → Mods.
pub fn experimental_pill() -> Div {
    let c: Hsla = theme::warning().into();
    div()
        .flex_none()
        .px(px(7.))
        .py(px(1.))
        .rounded_full()
        .bg(Hsla { a: 0.14, ..c })
        .border_1()
        .border_color(Hsla { a: 0.32, ..c })
        .text_size(px(11.))
        .text_color(theme::readable(theme::warning()))
        .whitespace_nowrap()
        .child(tr!("Experimental"))
}

impl ModsDialog {
    /// A label with its reason as a tooltip; the debug hook draws the reason open.
    fn labelled(&self, which: Tip, id: &'static str, label: Div, title: &'static str, text: &'static str) -> Div {
        let open = (self.debug_tip == Some(which))
            .then(|| deferred(div().absolute().top(px(26.)).left_0().child(titled_tip_box(title.into(), text.into()))).with_priority(30));
        div().relative().flex_none().child(div().id(id).child(label).tooltip(tooltip_titled(title, text))).children(open)
    }

    pub(super) fn experimental_label(&self) -> Div {
        self.labelled(
            Tip::Experimental,
            "mods-experimental",
            experimental_pill(),
            tr!("Mods are experimental"),
            tr!(
                "Heronite puts a mod's files over the game's folder while you play from here. That is enough for many games, not all: some keep mods elsewhere or need their own tools. The label under the title says what to expect for this game."
            ),
        )
    }

    /// "Should work", "Needs extra steps", "Untested game" or "Most mods won't work".
    pub(super) fn support_label(&self, v: &View) -> Div {
        let support = support_of(v);
        let pill = match support.level {
            Level::Expected => status_pill(support.label(), theme::success()),
            Level::Steps => status_pill(support.label(), theme::warning()),
            Level::Untested => tag_pill(support.label()),
            Level::Unsupported => status_pill(support.label(), theme::danger_text()),
        };
        self.labelled(Tip::Support, "mods-support", pill, support.label(), support.explain())
    }
}
