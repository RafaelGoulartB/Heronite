//! gamescope's part of the Performance tab ("Image").

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::parts::{column, heading, help, Rows};
use super::SettingsPanel;
use crate::theme;
use crate::ui::controls::row;
use crate::ui::fields::{field_label, input_box};
use heronite_i18n::{n_, tr};

pub(super) const UPSCALE_METHODS: [(&str, &str); 4] =
    [("fsr", "AMD FSR 1"), ("nis", "NVIDIA NIS"), ("integer", n_!("Integer scaling")), ("stretch", n_!("Stretch"))];
pub(super) const WINDOW_TYPES: [(&str, &str); 3] =
    [("fullscreen", n_!("Fullscreen")), ("borderless", n_!("Borderless")), ("windowed", n_!("Windowed"))];

impl SettingsPanel {
    pub(super) fn gs_flag(&self, field: &str) -> bool {
        matches!(self.gamescope().get(field), Some(Value::Bool(true)))
    }

    fn gs_toggle(
        &self,
        id: &'static str,
        field: &'static str,
        label: &str,
        help_text: Option<&str>,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.gs_flag(field);
        // The first switch stands for all of gamescope's settings.
        let group = if id == "gs-upscaling" { "gamescope" } else { "" };
        let mut row = self.switch_row(id, group, label, help_text, on, cx);
        if disabled {
            row = row.opacity(0.4).cursor_not_allowed();
        } else {
            row = row.on_click(cx.listener(move |this, _, _, cx| {
                let now = this.gs_flag(field);
                this.set_gamescope(field, Value::Bool(!now), cx)
            }));
        }
        row.into_any_element()
    }

    fn gs_input(&self, id: &'static str, label: &str, help_text: &str) -> Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(field_label(label.to_string()))
            .child(input_box(&self.input(id), false))
            .when(!help_text.is_empty(), |d| d.child(help(help_text).pt(px(5.))))
    }

    fn gs_select(
        &self,
        id: &'static str,
        field: &'static str,
        label: &str,
        table: &'static [(&'static str, &'static str)],
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let g = self.gamescope();
        let current = g.get(field).and_then(Value::as_str).unwrap_or("").to_string();
        let shown = match table.iter().find(|(k, _)| *k == current) {
            Some((_, l)) => tr(l).to_string(),
            // A Steam line's gamescope without an upscaling filter.
            None if current.is_empty() => tr!("None").to_string(),
            None => current.clone(),
        };
        let items = table.iter().map(|(k, l)| (SharedString::from(tr(l)), *k == current)).collect();
        self.select_row(id, Some(label), shown, items, disabled, cx, move |this, i, _, cx| {
            this.set_gamescope(field, Value::String(table[i].0.into()), cx)
        })
    }

    /// The "Image" caption and card: `rows` (Wine's FSR) then gamescope's
    /// upscaling and window. `disabled`: a Steam game whose launch options
    /// cannot change now.
    pub(super) fn gamescope_section(&mut self, mut up: Rows, disabled: bool, cx: &mut Context<Self>) -> Div {
        if !self.gamescope_installed {
            up = up.child(row(
                tr!("gamescope"),
                Some(tr!("Upscaling, a fixed resolution and window modes for any game")),
                div().text_size(px(13.)).text_color(theme::text_dim()).child(tr!("Not installed")),
            ));
            return column().child(heading(tr!("Image"))).child(up.card());
        }
        let pair = |a: Div, b: Div| div().flex().gap(px(12.)).child(a).child(b);
        up = up.child(self.gs_toggle(
            "gs-upscaling",
            "enableUpscaling",
            tr!("gamescope upscaling"),
            Some(tr!("Runs the game in gamescope at its own resolution, scaled to the screen")),
            disabled,
            cx,
        ));
        if self.gs_flag("enableUpscaling") {
            up = up
                .field(self.gs_select("gs-method", "upscaleMethod", tr!("Upscale method"), &UPSCALE_METHODS, disabled, cx))
                .field(pair(
                    self.gs_input("gs.gameWidth", tr!("Game width"), tr!("What the game renders")),
                    self.gs_input("gs.gameHeight", tr!("Game height"), ""),
                ))
                .field(pair(
                    self.gs_input("gs.upscaleWidth", tr!("Screen width"), tr!("What you see")),
                    self.gs_input("gs.upscaleHeight", tr!("Screen height"), ""),
                ))
                .field(self.gs_select("gs-window", "windowType", tr!("Window"), &WINDOW_TYPES, disabled, cx))
                .child(self.gs_toggle(
                    "gs-grab",
                    "enableForceGrabCursor",
                    tr!("Keep the mouse in the game"),
                    Some(tr!("For games that lose the cursor or keep losing focus")),
                    disabled,
                    cx,
                ))
                .field(self.gs_input("gs.additionalOptions", tr!("Extra gamescope options"), tr!("Like --hdr-enabled")));
        }
        column().child(heading(tr!("Image"))).child(up.card())
    }
}
