//! `LaunchStateDialog` ("How do you want to play?"): continue from one of the
//! emulator's save states or start from the beginning.

use std::rc::Rc;
use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::pick_rom::radio;
use super::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::paragraph;
use crate::ui::icon;

/// A decoded thumbnail ready for the GPU: BGRA pixels.
pub struct ThumbRaw {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl ThumbRaw {
    /// Decodes PNG/JPEG bytes, scaled down to at most 192x144.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let img = image::load_from_memory(bytes).ok()?.thumbnail(192, 144).to_rgba8();
        let (width, height) = img.dimensions();
        let mut bgra = img.into_raw();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Some(Self { width, height, bgra })
    }

    fn into_render_image(self) -> Option<Arc<RenderImage>> {
        let buf = image::RgbaImage::from_raw(self.width, self.height, self.bgra)?;
        Some(Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(buf)])))
    }
}

/// One save state as shown in the list.
pub struct StateRow {
    pub path: String,
    pub resume: bool,
    pub slot: Option<u32>,
    /// ISO 8601 UTC.
    pub modified_at: String,
    pub thumb: Option<ThumbRaw>,
}

struct Row {
    path: String,
    label: String,
    when: String,
    glyph: String,
    resume: bool,
    thumb: Option<Arc<RenderImage>>,
}

pub struct LaunchStateDialog {
    title: String,
    emulator: String,
    rows: Vec<Row>,
    /// Index into `rows`; `None` = start from the beginning.
    selected: Option<usize>,
    on_play: Rc<dyn Fn(Option<String>, &mut App)>,
}

impl EventEmitter<DialogEvent> for LaunchStateDialog {}

fn relative(dt: DateTime<Utc>) -> String {
    let secs = (Utc::now() - dt).num_seconds().max(0);
    let units = [
        (365 * 24 * 3600, "year"),
        (30 * 24 * 3600, "month"),
        (7 * 24 * 3600, "week"),
        (24 * 3600, "day"),
        (3600, "hour"),
        (60, "minute"),
    ];
    for (size, name) in units {
        if secs >= size {
            let n = secs / size;
            return format!("{n} {name}{} ago", if n == 1 { "" } else { "s" });
        }
    }
    "just now".into()
}

fn describe(iso: &str) -> String {
    match DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => {
            let utc = dt.with_timezone(&Utc);
            format!("{} · {}", relative(utc), utc.with_timezone(&Local).format("%b %-d, %Y, %-I:%M %p"))
        }
        Err(_) => iso.to_string(),
    }
}

impl LaunchStateDialog {
    pub fn new(
        title: String,
        emulator: String,
        states: Vec<StateRow>,
        on_play: Rc<dyn Fn(Option<String>, &mut App)>,
    ) -> Self {
        let rows: Vec<Row> = states
            .into_iter()
            .map(|s| Row {
                label: if s.resume {
                    "Resume point".into()
                } else {
                    match s.slot {
                        Some(n) => format!("Slot {n}"),
                        None => "Save state".into(),
                    }
                },
                glyph: s.slot.map(|n| n.to_string()).unwrap_or_else(|| "•".into()),
                when: describe(&s.modified_at),
                resume: s.resume,
                thumb: s.thumb.and_then(ThumbRaw::into_render_image),
                path: s.path,
            })
            .collect();
        let selected = if rows.is_empty() { None } else { Some(0) };
        Self { title, emulator, rows, selected, on_play }
    }

    fn play(&self, cx: &mut App) {
        let path = self.selected.and_then(|i| self.rows.get(i)).map(|r| r.path.clone());
        (self.on_play)(path, cx);
    }
}

fn thumb_box(content: impl IntoElement) -> Div {
    div()
        .w(px(96.))
        .h(px(72.))
        .flex_none()
        .rounded(px(10.))
        .overflow_hidden()
        .bg(theme::black_a(0.35))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(19.2))
        .text_color(white())
        .child(content)
}

impl Render for LaunchStateDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let item = |id: SharedString, on: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(theme::R_TILE))
                .border_1()
                .border_color(if on { theme::accent().into() } else { theme::white_a(0.08) })
                .when(on, |d| d.bg(theme::white_a(0.04)))
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.04)))
        };
        let mut list = div().id("launch-states").flex().flex_col().gap(px(8.)).max_h(px(420.)).overflow_y_scroll();
        for (i, row) in self.rows.iter().enumerate() {
            let on = self.selected == Some(i);
            let thumb = match &row.thumb {
                Some(img) => thumb_box(gpui_kit::img(ImageSource::Render(img.clone())).size_full().object_fit(ObjectFit::Cover)),
                None if row.resume => thumb_box(icon(Icon::PlayArrow, px(28.), white())),
                None => thumb_box(row.glyph.clone()),
            };
            list = list.child(
                item(SharedString::from(format!("state-{i}")), on)
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.selected = Some(i);
                        if ev.click_count() >= 2 {
                            this.play(cx);
                        }
                        cx.notify();
                    }))
                    .child(radio(on))
                    .child(thumb)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(row.label.clone()))
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(row.when.clone())),
                    ),
            );
        }
        let fresh_on = self.selected.is_none();
        list = list.child(
            item("state-fresh".into(), fresh_on)
                .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                    this.selected = None;
                    if ev.click_count() >= 2 {
                        this.play(cx);
                    }
                    cx.notify();
                }))
                .child(radio(fresh_on))
                .child(thumb_box(icon(Icon::Replay, px(28.), white())))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child("Start from the beginning"))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::text_dim())
                                .child("Boot the game normally. In-game saves on memory cards are kept."),
                        ),
                ),
        );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph(format!(
                "{} has save states for {}. Continue from one or start from the beginning.",
                if self.emulator.is_empty() { "The emulator" } else { &self.emulator },
                self.title
            )))
            .child(list);
        let footer = div()
            .flex()
            .gap(px(16.))
            .child(button("state-cancel", ButtonKind::Ghost, "Cancel").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(
                button("state-play", ButtonKind::Default, if fresh_on { "Start" } else { "Continue" })
                    .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
            )
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "launch-state".into(),
                title: "How do you want to play?".into(),
                width: 560f32.min(f32::from(window.viewport_size().width) * 0.9) + 64.,
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
