//! `CollectionPlaytimeDialog`: edit the total time played of a game
//! (`store/timestamp.json` `totalPlayed`, in minutes). New sessions keep
//! adding to the value.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::playtime::{clamp_number, set_playtime_minutes, split_minutes};

use super::{DialogCtx, DialogEvent};
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{field_label, input_box, new_input, paragraph};

pub struct PlaytimeDialog {
    ctx: DialogCtx,
    app_name: String,
    title: String,
    hours: Entity<InputState>,
    minutes: Entity<InputState>,
    error: Option<SharedString>,
}

impl EventEmitter<DialogEvent> for PlaytimeDialog {}

/// Hours and minutes typed in the fields as the dialog understands them.
fn parsed(hours: &str, minutes: &str) -> (u64, u64) {
    (clamp_number(hours, None), clamp_number(minutes, Some(59)))
}

impl PlaytimeDialog {
    /// `minutes`: the current `totalPlayed`.
    pub fn new(ctx: DialogCtx, app_name: String, title: String, minutes: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (h, m) = split_minutes(minutes);
        let hours = new_input(window, cx, &h.to_string(), "0");
        let mins = new_input(window, cx, &m.to_string(), "0");
        for input in [&hours, &mins] {
            cx.subscribe_in(input, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => cx.notify(),
                InputEvent::Blur => this.normalize(window, cx),
                InputEvent::PressEnter { .. } => this.save(cx),
                _ => {}
            })
            .detach();
        }
        Self { ctx, app_name, title, hours, minutes: mins, error: None }
    }

    fn values(&self, cx: &App) -> (u64, u64) {
        parsed(self.hours.read(cx).value().as_ref(), self.minutes.read(cx).value().as_ref())
    }

    /// Shows the clamped numbers once a field loses focus.
    fn normalize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (h, m) = self.values(cx);
        for (input, value) in [(&self.hours, h), (&self.minutes, m)] {
            let text = value.to_string();
            if input.read(cx).value().as_ref() != text {
                input.update(cx, |s, cx| s.set_value(text, window, cx));
            }
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let (h, m) = self.values(cx);
        let total = h * 60 + m;
        match set_playtime_minutes(&self.ctx.data_paths, &self.app_name, total as f64) {
            Ok(()) => cx.emit(DialogEvent::ChangedAndClose),
            Err(err) => {
                self.error = Some(format!("Could not save the playtime: {err:#}").into());
                cx.notify();
            }
        }
    }
}

impl Render for PlaytimeDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (h, m) = self.values(cx);
        let field = |label: &'static str, input: &Entity<InputState>| {
            div().flex().flex_col().flex_1().gap(px(2.)).child(field_label(label)).child(input_box(input, false))
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(paragraph(format!("Set the total time played for {}. New sessions keep adding to this value.", self.title)))
            .child(div().flex().gap(px(16.)).child(field("Hours", &self.hours)).child(field("Minutes", &self.minutes)))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(crate::theme::W_BOLD)
                    .text_color(crate::theme::text_primary())
                    .child(format_playtime_minutes(h * 60 + m)),
            )
            .children(self.error.clone().map(|e| div().text_color(crate::theme::danger_text()).child(e)));
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("playtime-cancel", ButtonKind::Ghost, "Cancel").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(button("playtime-save", ButtonKind::Primary, "Save").on_click(cx.listener(|this, _, _, cx| this.save(cx))));
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "playtime".into(), title: "Edit playtime".into(), width: 460., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::parsed;

    #[test]
    fn typed_values_are_clamped_like_the_dialog() {
        assert_eq!(parsed("2", "5"), (2, 5));
        assert_eq!(parsed("", "75"), (0, 59));
        assert_eq!(parsed("-1", "x"), (0, 0));
        assert_eq!(parsed("100", "0"), (100, 0));
    }
}
