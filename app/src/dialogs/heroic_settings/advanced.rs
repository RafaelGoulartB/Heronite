//! "Advanced" tab: logs, alternative exe, launcher arguments, scripts,
//! wrappers, environment variables, language, and the legacy options.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::logic::{self, launcher_args_error, pairs, pairs_to_value};
use super::parts::{column, danger_text, heading, help};
use super::HeroicSettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};
use crate::ui::fields::{field_label, input_box};
use crate::ui::icon;

/// Static description of a two-column table setting.
#[derive(Clone, Copy)]
struct PairTable {
    id: &'static str,
    title: &'static str,
    setting: &'static str,
    k1: &'static str,
    k2: &'static str,
    in1: &'static str,
    in2: &'static str,
    head: (&'static str, &'static str),
    env: bool,
}

const ENV: PairTable = PairTable {
    id: "env",
    title: "Environment Variables",
    setting: "enviromentOptions",
    k1: "key",
    k2: "value",
    in1: "env.key",
    in2: "env.value",
    head: ("Name", "Value"),
    env: true,
};
const WRAPPERS: PairTable = PairTable {
    id: "wrap",
    title: "Wrapper command:",
    setting: "wrapperOptions",
    k1: "exe",
    k2: "args",
    in1: "wrap.exe",
    in2: "wrap.args",
    head: ("Wrapper", "Arguments"),
    env: false,
};

impl HeroicSettingsPanel {
    pub(super) fn advanced_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let sideload = self.facts().is_some_and(|f| f.is_sideload());
        let legendary = self.facts().is_some_and(|f| f.runner == "legendary");
        let game = self.is_game();
        let mut col = column();
        if game && !sideload {
            col = col.child(self.toggle_row("ignore-updates", "ignoreGameUpdates", false, "Ignore game updates", None, false, cx));
        }
        if game && legendary {
            col = col.child(self.toggle_row("offline", "offlineMode", false, "Run Game Offline", None, false, cx));
        }
        col = col.child(self.toggle_row("verbose", "verboseLogs", true, "Enable verbose logs", None, false, cx));
        if game && !sideload {
            col = col.child(self.path_row("targetExe", Some("Select an alternative EXE to run"), false, "Select EXE", true, cx));
        }
        if game {
            let args = self.input_text("launcherArgs", cx);
            let err = launcher_args_error(&args);
            col = col
                .child(self.text_row("launcherArgs", Some("Game Arguments (appended to game launch command)"), None))
                .children((!err.is_empty()).then(|| danger_text(err)))
                .child(help("Use the Game Arguments to be called after the launch command, for instance: -nolauncher to skip the launcher in some games, etc."));
        }
        col = col
            .child(div().flex().flex_col().gap(px(10.)).child(field_label("Scripts:"))
                .child(self.path_row("beforeScript", Some("Select a script to run before the game is launched"), false, "Select script ...", true, cx))
                .child(self.path_row("afterScript", Some("Select a script to run after the game exits"), false, "Select script ...", true, cx)))
            .child(self.pair_table(WRAPPERS, cx))
            .child(self.pair_table(ENV, cx));
        if !sideload {
            col = col.child(self.text_row(
                "language",
                Some("Prefered Language (Language Code)"),
                Some("Leave blank to use Heroic's language. Not all games support this configuration, some have in-game language setting. Valid language codes are game-dependant."),
            ));
        }
        // Legacy options (a tab of its own in Heroic).
        let wine = self.wine_version();
        let show_dxvk_fps = !self.native() && !wine.bin.contains("toolkit") && !wine.name.ends_with("-DXMT");
        let show_umu = game && !self.native() && wine.kind == "proton";
        if show_dxvk_fps || show_umu {
            col = col.child(heading("Legacy")).child(help("Warning: The settings on this tab are mostly deprecated and might not work at all."));
            if show_dxvk_fps {
                col = col.child(self.toggle_row(
                    "dxvk-fps",
                    "enableDXVKFpsLimit",
                    false,
                    "Limit DirectX Games FPS",
                    Some("Sets a frame rate cap for DirectX Games (9-12)"),
                    false,
                    cx,
                ));
                if self.flag("enableDXVKFpsLimit", false) {
                    col = col.child(self.text_row("dxvkFps", None, None));
                }
            }
            if show_umu {
                col = col.child(self.toggle_row("disable-umu", "disableUMU", false, "Disable umu", None, false, cx));
            }
        }
        col
    }

    fn pair_table(&mut self, t: PairTable, cx: &mut Context<Self>) -> Div {
        let rows = pairs(self.settings.get(t.setting), t.k1, t.k2);
        let editing = self.input_text(t.in1, cx).trim().to_string();
        let exists = rows.iter().any(|(a, _)| *a == editing);
        let mut list = div().flex().flex_col().gap(px(4.));
        for (i, (a, b)) in rows.iter().enumerate() {
            let (ea, eb) = (a.clone(), b.clone());
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(theme::white_a(0.04))
                    .child(div().w(px(260.)).flex_none().truncate().font_weight(theme::W_BOLD).text_size(px(14.)).text_color(rgb(0xcaf3fd)).child(a.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(white()).child(b.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-edit-{i}", t.id)))
                            .p(px(4.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.12)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_input(t.in1, &ea, window, cx);
                                this.set_input(t.in2, &eb, window, cx);
                                cx.notify();
                            }))
                            .child(icon(Icon::Edit, px(18.), rgb(0xcaf3fd))),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-del-{i}", t.id)))
                            .p(px(4.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.12)))
                            .on_click(cx.listener(move |this, _, _, cx| this.pair_remove(t, i, cx)))
                            .child(icon(Icon::DeleteOutline, px(18.), rgb(0xcaf3fd))),
                    ),
            );
        }
        let add = div()
            .flex()
            .items_end()
            .gap(px(8.))
            .child(div().flex_1().child(input_box(&self.input(t.in1), false)))
            .child(div().flex_1().child(input_box(&self.input(t.in2), false)))
            .child(action_button(
                SharedString::from(format!("{}-add", t.id)),
                ButtonKind::Outline,
                if exists { "Update" } else { "Add" },
                true,
                cx.listener(move |this, _, window, cx| this.pair_add(t, window, cx)),
            ));
        let mut col = div().flex().flex_col().gap(px(8.)).child(field_label(t.title));
        if !rows.is_empty() {
            col = col.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .px(px(10.))
                    .text_size(px(12.))
                    .text_color(theme::white_a(0.6))
                    .child(div().w(px(260.)).flex_none().child(t.head.0))
                    .child(div().flex_1().child(t.head.1)),
            );
        }
        col = col.child(list).child(add);
        if t.env && !self.env_error.is_empty() {
            col = col.child(danger_text(&self.env_error));
        }
        if t.env {
            col.child(help("Set environment variables to append to the command. Do NOT include the \"=\" sign, e.g: for a setting like \"MY_FLAG=123\", set MY_FLAG in NAME and 123 in VALUE."))
        } else {
            col.child(help("Warning: Make sure to quote args with spaces! E.g.: \"path/with spaces/\". Arguments example: --arg; --extra-file=\"file-path/ with/spaces\""))
        }
    }

    fn pair_remove(&mut self, t: PairTable, index: usize, cx: &mut Context<Self>) {
        let mut rows = pairs(self.settings.get(t.setting), t.k1, t.k2);
        if index < rows.len() {
            rows.remove(index);
            self.set(t.setting, pairs_to_value(&rows, t.k1, t.k2), cx);
        }
    }

    fn pair_add(&mut self, t: PairTable, window: &mut Window, cx: &mut Context<Self>) {
        let mut a = self.input_text(t.in1, cx).trim().to_string();
        let mut b = self.input_text(t.in2, cx).trim().to_string();
        if t.env {
            if a.contains('=') && b.is_empty() {
                let (k, v) = a.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).unwrap_or_default();
                a = k;
                b = v;
            }
            let err = logic::validate_env_key(&a);
            if !err.is_empty() {
                self.env_error = err.to_string();
                cx.notify();
                return;
            }
            self.env_error.clear();
        } else if a.is_empty() {
            return;
        }
        let mut rows = pairs(self.settings.get(t.setting), t.k1, t.k2);
        match rows.iter().position(|(k, _)| *k == a) {
            Some(i) => rows[i].1 = b,
            None => rows.push((a, b)),
        }
        self.set(t.setting, pairs_to_value(&rows, t.k1, t.k2), cx);
        self.set_input(t.in1, "", window, cx);
        self.set_input(t.in2, "", window, cx);
        cx.notify();
    }
}
