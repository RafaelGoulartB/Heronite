//! "Programs alongside the game" in the Advanced tab of a game: trainers and
//! mod tools started with the game and closed with it (see
//! `heronite_launch::companions`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::companions::{parse, to_value, Companion, KEY};

use super::parts::{column, help, rows};
use super::SettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::choose_path;
use crate::ui::icon;
use heronite_i18n::tr;

/// The delays offered, in seconds.
pub(super) const DELAYS: [u32; 12] = [0, 5, 10, 15, 20, 30, 45, 60, 90, 120, 300, 600];

/// "With the game" or "10 s after the game".
pub(super) fn delay_label(secs: u32) -> String {
    match secs {
        0 => tr!("With the game").to_string(),
        s if s % 60 == 0 => tr!("{minutes} min after the game", minutes = s / 60),
        s => tr!("{seconds} s after the game", seconds = s),
    }
}

/// Index in [`DELAYS`] of the closest offered delay.
pub(super) fn delay_index(secs: u32) -> usize {
    DELAYS.iter().enumerate().min_by_key(|(_, d)| d.abs_diff(secs)).map(|(i, _)| i).unwrap_or(0)
}

pub(super) fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

impl SettingsPanel {
    pub(super) fn companions(&self) -> Vec<Companion> {
        parse(self.settings.get(KEY))
    }

    fn set_companions(&mut self, list: &[Companion], cx: &mut Context<Self>) {
        self.set(KEY, to_value(list), cx);
    }

    pub(super) fn add_companion(&mut self, path: String, cx: &mut Context<Self>) {
        let mut list = self.companions();
        if !list.iter().any(|c| c.path == path) {
            list.push(Companion { path, delay_secs: 0 });
            self.set_companions(&list, cx);
        }
    }

    pub(super) fn remove_companion(&mut self, index: usize, cx: &mut Context<Self>) {
        let mut list = self.companions();
        if index < list.len() {
            list.remove(index);
            self.set_companions(&list, cx);
        }
    }

    pub(super) fn set_companion_delay(&mut self, index: usize, secs: u32, cx: &mut Context<Self>) {
        let mut list = self.companions();
        if let Some(c) = list.get_mut(index) {
            c.delay_secs = secs;
            self.set_companions(&list, cx);
        }
    }

    /// Moves the delay of program `index` one step along [`DELAYS`].
    fn step_companion_delay(&mut self, index: usize, up: bool, cx: &mut Context<Self>) {
        let Some(c) = self.companions().get(index).cloned() else { return };
        let i = delay_index(c.delay_secs);
        let next = if up { (i + 1).min(DELAYS.len() - 1) } else { i.saturating_sub(1) };
        self.set_companion_delay(index, DELAYS[next], cx);
    }

    pub(super) fn companions_section(&mut self, cx: &mut Context<Self>) -> Div {
        let round = |id: String, ic: Icon, enabled: bool| {
            let b = div()
                .id(SharedString::from(id))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded_full()
                .child(icon(ic, px(16.), if enabled { theme::text_muted() } else { theme::text_dim() }));
            if enabled {
                b.cursor_pointer().hover(|s| s.bg(theme::white_a(0.10)))
            } else {
                b.opacity(0.5)
            }
        };
        let mut list = rows();
        for (i, c) in self.companions().iter().enumerate() {
            let windows = c.path.to_lowercase().ends_with(".exe");
            let at = delay_index(c.delay_secs);
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .child(icon(if windows { Icon::WineBar } else { Icon::Description }, px(18.), theme::text_muted()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(file_name(&c.path)))
                            .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(c.path.clone())),
                    )
                    .child(
                        round(format!("companion-sooner-{i}"), Icon::ChevronLeft, at > 0)
                            .on_click(cx.listener(move |this, _, _, cx| this.step_companion_delay(i, false, cx))),
                    )
                    .child(
                        div()
                            .w(px(150.))
                            .flex_none()
                            .text_center()
                            .text_size(px(13.))
                            .text_color(theme::text_primary())
                            .child(delay_label(c.delay_secs)),
                    )
                    .child(
                        round(format!("companion-later-{i}"), Icon::ChevronRight, at + 1 < DELAYS.len())
                            .on_click(cx.listener(move |this, _, _, cx| this.step_companion_delay(i, true, cx))),
                    )
                    .child(
                        round(format!("companion-del-{i}"), Icon::DeleteOutline, true)
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_companion(i, cx))),
                    ),
            );
        }
        list = list.child(div().py(px(10.)).child(self.outline_button("companion-add", tr!("Add program"), true, cx, |_, window, cx| {
            choose_path(window, cx, false, tr!("Choose a program to run alongside the game"), |this: &mut Self, path, _, cx| {
                this.add_companion(path, cx)
            });
        })));
        column().child(self.marked_heading(tr!("Programs alongside the game"), "companions", cx)).child(list.card()).child(
            help(tr!("Trainers and tools started with the game and closed with it. Windows programs run in the game's prefix.")).px(px(6.)),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the GPUI prelude has its own `test` attribute.
    use super::{delay_index, file_name, DELAYS};

    #[test]
    fn delays_snap_to_the_offered_steps() {
        assert_eq!(DELAYS[delay_index(0)], 0);
        assert_eq!(DELAYS[delay_index(12)], 10);
        assert_eq!(DELAYS[delay_index(100)], 90);
        assert_eq!(DELAYS[delay_index(9999)], 600);
        assert_eq!(file_name("/a/b/Trainer.exe"), "Trainer.exe");
    }
}
