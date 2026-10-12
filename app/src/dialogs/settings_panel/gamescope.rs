//! "Gamescope" tab.

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::parts::{column, danger_text, heading, help, rows};
use super::SettingsPanel;
use crate::ui::controls::toggle_row;
use crate::ui::fields::{field_label, input_box};
use heronite_i18n::{n_, tr};

pub(super) const UPSCALE_METHODS: [(&str, &str); 4] = [
    ("fsr", "AMD FidelityFX\u{2122} Super Resolution 1.0 (FSR)"),
    ("nis", "NVIDIA Image Scale (NIS)"),
    ("integer", n_!("Interger Upscale")),
    ("stretch", n_!("Stretch Image")),
];
pub(super) const WINDOW_TYPES: [(&str, &str); 3] =
    [("fullscreen", n_!("Fullscreen")), ("borderless", n_!("Borderless")), ("windowed", n_!("Windowed"))];

impl SettingsPanel {
    pub(super) fn gs_flag(&self, field: &str) -> bool {
        matches!(self.gamescope().get(field), Some(Value::Bool(true)))
    }

    fn gs_toggle(&self, id: &'static str, field: &'static str, label: &str, help_text: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let on = self.gs_flag(field);
        toggle_row(id, label.to_string(), help_text, on)
            .on_click(cx.listener(move |this, _, _, cx| {
                let now = this.gs_flag(field);
                this.set_gamescope(field, Value::Bool(!now), cx)
            }))
            .into_any_element()
    }

    fn gs_input(&self, id: &'static str, label: &str, help_text: &str) -> Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(field_label(label.to_string()))
            .child(input_box(&self.input(id), false))
            .child(help(help_text).pt(px(5.)))
    }

    fn gs_select(
        &self,
        id: &'static str,
        field: &'static str,
        label: &str,
        table: &'static [(&'static str, &'static str)],
        cx: &mut Context<Self>,
    ) -> Div {
        let g = self.gamescope();
        let current = g.get(field).and_then(Value::as_str).unwrap_or("").to_string();
        let shown = table.iter().find(|(k, _)| *k == current).map(|(_, l)| tr(l).to_string()).unwrap_or(current.clone());
        let items = table.iter().map(|(k, l)| (SharedString::from(tr(l)), *k == current)).collect();
        self.select_row(id, Some(label), shown, items, false, cx, move |this, i, _, cx| {
            this.set_gamescope(field, Value::String(table[i].0.into()), cx)
        })
    }

    pub(super) fn gamescope_tab(&mut self, cx: &mut Context<Self>) -> Div {
        if !self.gamescope_installed {
            return column().child(danger_text(tr!("We could not find gamescope on the PATH. Install it or add it to the PATH.")));
        }
        let pair = |a: Div, b: Div| div().flex().gap(px(12.)).child(a).child(b);
        let mut up = rows().child(self.gs_toggle("gs-upscaling", "enableUpscaling", tr!("Enables Upscaling"), None, cx));
        if self.gs_flag("enableUpscaling") {
            up = up
                .field_help(
                    self.gs_select("gs-method", "upscaleMethod", tr!("Upscale Method"), &UPSCALE_METHODS, cx),
                    vec![help(tr!("The upscaling method gamescope should use."))],
                )
                .field(pair(
                    self.gs_input(
                        "gs.gameWidth",
                        tr!("Game Width"),
                        tr!("The width resolution used by the game. A 16:9 aspect ratio is assumed by gamescope."),
                    ),
                    self.gs_input(
                        "gs.gameHeight",
                        tr!("Game Height"),
                        tr!("The height resolution used by the game. A 16:9 aspect ratio is assumed by gamescope."),
                    ),
                ))
                .field(pair(
                    self.gs_input(
                        "gs.upscaleWidth",
                        tr!("Upscale Width"),
                        tr!("The width resolution used by gamescope. A 16:9 aspect ratio is assumed."),
                    ),
                    self.gs_input(
                        "gs.upscaleHeight",
                        tr!("Upscale Height"),
                        tr!("The height resolution used by gamescope. A 16:9 aspect ratio is assumed."),
                    ),
                ))
                .field(self.gs_select("gs-window", "windowType", tr!("Window Type"), &WINDOW_TYPES, cx));
        }
        up = up.child(self.gs_toggle("gs-limiter", "enableLimiter", tr!("Enable FPS Limiter"), None, cx));
        if self.gs_flag("enableLimiter") {
            up = up.field(pair(
                self.gs_input("gs.fpsLimiter", tr!("FPS Limiter"), tr!("The frame rate limit gamescope should limit per second.")),
                self.gs_input(
                    "gs.fpsLimiterNoFocus",
                    tr!("FPS Limiter (No Focus)"),
                    tr!("The frame rate limit gamescope should limit per second if the game is not focused."),
                ),
            ));
        }
        let other = up
            .child(self.gs_toggle(
                "gs-grab",
                "enableForceGrabCursor",
                tr!("Enable Force Grab Cursor"),
                Some(tr!("Always use relative mouse mode instead of flipping dependent on cursor visibility. (Useful for when applications keep losing focus)")),
                cx,
            ))
            .field(self.gs_input("gs.additionalOptions", tr!("Additional Options"), tr!("Additional commandline flags to pass into gamescope.")));
        column().child(heading("Gamescope")).child(other.card())
    }
}
