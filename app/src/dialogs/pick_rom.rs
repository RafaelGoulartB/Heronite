//! `PickRomDialog` ("Choose disc"): a title with several ROMs asks which one
//! to launch. The same list asks which version to play of a group of ROM
//! copies (romhacks, regions, revisions: "Choose version").

mod console;

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;

/// What the dialog picks: one ROM of a game, or one copy of a group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PickKind {
    Disc,
    Version,
}

pub struct PickRomDialog {
    title: String,
    kind: PickKind,
    /// `(name, id)`: a ROM's path, or a copy's Collection key.
    roms: Vec<(String, String)>,
    /// The second line of each row: a ROM's path, or a copy's emulator and playtime.
    descs: Vec<String>,
    selected: Option<String>,
    on_play: Rc<dyn Fn(String, &mut App)>,
    console_list: crate::console::list::ListNav,
}

impl EventEmitter<DialogEvent> for PickRomDialog {}

impl PickRomDialog {
    pub fn new(title: String, roms: Vec<(String, String)>, selected_path: Option<String>, on_play: Rc<dyn Fn(String, &mut App)>) -> Self {
        let descs = roms.iter().map(|(_, p)| p.clone()).collect();
        Self::with(title, PickKind::Disc, roms, descs, selected_path, on_play)
    }

    /// The versions of a group of ROM copies: `(name, key, desc)`.
    pub fn versions(
        title: String,
        versions: Vec<(String, String, String)>,
        selected_key: Option<String>,
        on_play: Rc<dyn Fn(String, &mut App)>,
    ) -> Self {
        let (roms, descs) = versions.into_iter().map(|(name, key, desc)| ((name, key), desc)).unzip();
        Self::with(title, PickKind::Version, roms, descs, selected_key, on_play)
    }

    fn with(
        title: String,
        kind: PickKind,
        roms: Vec<(String, String)>,
        descs: Vec<String>,
        selected: Option<String>,
        on_play: Rc<dyn Fn(String, &mut App)>,
    ) -> Self {
        let selected = selected.filter(|p| roms.iter().any(|(_, rp)| rp == p)).or_else(|| roms.first().map(|(_, p)| p.clone()));
        let mut this = Self { title, kind, roms, descs, selected, on_play, console_list: Default::default() };
        this.console_focus_selected();
        this
    }

    fn heading(&self) -> String {
        match self.kind {
            PickKind::Disc => tr!("Choose disc").to_string(),
            PickKind::Version => tr!("Choose version").to_string(),
        }
    }

    fn intro(&self) -> String {
        match self.kind {
            PickKind::Disc => tr!("This title has more than one ROM. Pick which disc to launch.").to_string(),
            PickKind::Version => tr!("This game has more than one version. Pick which one to play.").to_string(),
        }
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
        for (i, ((name, path), desc)) in self.roms.iter().zip(&self.descs).enumerate() {
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
                    .hover(|s| s.bg(if on { theme::accent_tint() } else { theme::ink_a(0.08) }))
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
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(desc.clone())),
                    ),
            );
        }
        let body = div().flex().flex_col().gap(px(12.)).child(paragraph(self.intro())).child(list);
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("rom-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "rom-play",
                ButtonKind::Primary,
                tr!("Play"),
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
                title: self.heading().into(),
                width: 520f32.min(f32::from(crate::ui::content_size(window).width) * 0.9) + 64.,
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
