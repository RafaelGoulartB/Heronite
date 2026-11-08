//! "PC" section of the focus panel: facts from PCGamingWiki, with the page,
//! its fix count and the licence named and linked (CC BY-NC-SA 3.0).

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{h3, panel};
use crate::collection::vm::PcVm;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{chip, icon};

/// A button of the "PC" section.
#[derive(Clone)]
pub(in crate::collection) enum PcAction {
    /// Opens a page (the game's on PCGamingWiki, the licence).
    Open(SharedString),
    /// Copies a save folder.
    Copy(SharedString),
}

pub(in crate::collection) type OnPc = Rc<dyn Fn(PcAction, &mut Window, &mut App)>;

fn link(id: &'static str, ic: Icon, label: SharedString, action: PcAction, on: &OnPc) -> Stateful<Div> {
    let on = on.clone();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(7.))
        .py(px(3.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(theme::ink_a(0.7))
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.08)).text_color(theme::ink_a(0.94)))
        .on_click(move |_, window, cx| on(action.clone(), window, cx))
        .child(icon(ic, px(13.), theme::ink_a(0.6)))
        .child(label)
}

fn row(label: &'static str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .text_size(px(13.))
        .child(div().w(px(96.)).flex_none().text_color(theme::ink_a(0.52)).child(label))
        .child(div().flex_1().min_w_0().text_color(theme::ink_a(0.94)).child(value))
}

pub(super) fn pc_section(pc: &PcVm, on: &OnPc) -> Div {
    let mut body = panel(true).gap(px(10.));
    if !pc.chips.is_empty() {
        body = body.child(div().flex().flex_wrap().gap(px(5.)).children(pc.chips.iter().map(|c| chip(c.clone()))));
    }
    if let Some(engine) = &pc.engine {
        body = body.child(row(tr!("Engine"), engine.clone()));
    }
    if let Some(ac) = &pc.anticheat {
        body = body.child(row(tr!("Anti-cheat"), ac.clone()));
    }
    if let Some(save) = &pc.save {
        let on_copy = on.clone();
        let path = save.clone();
        body = body.child(row(
            tr!("Saves"),
            div().flex().items_start().gap(px(6.)).child(div().flex_1().min_w_0().text_size(px(12.)).child(save.clone())).child(
                div()
                    .id("pc-copy-save")
                    .flex_none()
                    .p(px(3.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::ink_a(0.08)))
                    .on_click(move |_, window, cx| on_copy(PcAction::Copy(path.clone()), window, cx))
                    .child(icon(Icon::ContentCopy, px(13.), theme::ink_a(0.6))),
            ),
        ));
    }
    let page = if pc.fixes > 0 { tr!("PCGamingWiki · {n} fixes", n = pc.fixes).into() } else { SharedString::from("PCGamingWiki") };
    body = body.child(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .ml(px(-7.))
            .child(link("pc-open", Icon::OpenInNew, page, PcAction::Open(pc.link.clone()), on))
            .child(link(
                "pc-licence",
                Icon::Link,
                tr!("Facts under CC BY-NC-SA 3.0").into(),
                PcAction::Open(heronite_data::pcgw::LICENCE_URL.into()),
                on,
            )),
    );
    div().flex().flex_col().child(h3("PC")).child(body)
}
