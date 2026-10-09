//! `PickRomDialog` ("Choose disc"): a title with several ROMs asks which one
//! to launch.

mod console;

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;

pub struct PickRomDialog {
    title: String,
    /// `(name, path)`.
    roms: Vec<(String, String)>,
    selected: Option<String>,
    on_play: Rc<dyn Fn(String, &mut App)>,
    console_list: crate::console::list::ListNav,
}

impl EventEmitter<DialogEvent> for PickRomDialog {}

impl PickRomDialog {
    pub fn new(
        title: String,
        roms: Vec<(String, String)>,
        selected_path: Option<String>,
        on_play: Rc<dyn Fn(String, &mut App)>,
    ) -> Self {
        let selected = selected_path
            .filter(|p| roms.iter().any(|(_, rp)| rp == p))
            .or_else(|| roms.first().map(|(_, p)| p.clone()));
        let mut this = Self { title, roms, selected, on_play, console_list: Default::default() };
        this.console_focus_selected();
        this
    }
}

/// Radio dot (MUI-like, in the accent colour).
pub fn radio(on: bool) -> Div {
    div()
        .size(px(18.))
        .flex_none()
        .rounded_full()
        .border(px(2.))
        .border_color(if on { theme::accent() } else { crate::theme::text_primary() })
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| d.child(div().size(px(8.)).rounded_full().bg(theme::accent())))
}

impl Render for PickRomDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console();
        }
        let mut list = div().flex().flex_col().gap(px(8.));
        for (i, (name, path)) in self.roms.iter().enumerate() {
            let on = self.selected.as_deref() == Some(path.as_str());
            let p1 = path.clone();
            let label = if name.is_empty() { self.title.clone() } else { name.clone() };
            list = list.child(
                div()
                    .id(SharedString::from(format!("rom-{i}")))
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(theme::R_TILE))
                    .border_1()
                    .border_color(if on { theme::accent_tint_border() } else { theme::card_border() })
                    .bg(if on { theme::accent_tint() } else { theme::card_fill() })
                    .cursor_pointer()
                    .hover(|s| s.bg(if on { theme::accent_tint() } else { theme::white_a(0.08) }))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.selected = Some(p1.clone());
                        if ev.click_count() >= 2 {
                            (this.on_play)(p1.clone(), cx);
                        }
                        cx.notify();
                    }))
                    .child(div().mt(px(2.)).child(radio(on)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().font_weight(theme::W_BOLD).text_size(px(14.)).text_color(theme::text_primary()).child(label))
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(path.clone())),
                    ),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph("This title has more than one ROM. Pick which disc to launch."))
            .child(list);
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("rom-cancel", ButtonKind::Ghost, "Cancel").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "rom-play",
                ButtonKind::Primary,
                "Play",
                self.selected.is_some(),
                cx.listener(|this, _, _, cx| {
                    if let Some(p) = this.selected.clone() {
                        (this.on_play)(p, cx);
                    }
                }),
            ))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "pick-rom".into(),
                title: "Choose disc".into(),
                width: 520f32.min(f32::from(window.viewport_size().width) * 0.9) + 64.,
                height: None,
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
