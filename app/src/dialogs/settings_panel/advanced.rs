//! "Advanced" tab: logs, alternative exe, launcher arguments, scripts,
//! wrappers, environment variables, language, and the legacy options.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::logic::{self, launcher_args_error, pairs, pairs_to_value};
use super::parts::{column, danger_text, heading, help, rows};
use super::SettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};
use crate::ui::fields::input_box;
use crate::ui::icon;
use heronite_i18n::{n_, tr};

/// Static description of a two-column table setting.
#[derive(Clone, Copy)]
pub(super) struct PairTable {
    pub(super) id: &'static str,
    pub(super) title: &'static str,
    pub(super) setting: &'static str,
    pub(super) k1: &'static str,
    pub(super) k2: &'static str,
    pub(super) in1: &'static str,
    pub(super) in2: &'static str,
    pub(super) env: bool,
}

pub(super) const ENV: PairTable = PairTable {
    id: "env",
    title: n_!("Environment Variables"),
    setting: "enviromentOptions",
    k1: "key",
    k2: "value",
    in1: "env.key",
    in2: "env.value",
    env: true,
};
pub(super) const WRAPPERS: PairTable = PairTable {
    id: "wrap",
    title: n_!("Wrapper command:"),
    setting: "wrapperOptions",
    k1: "exe",
    k2: "args",
    in1: "wrap.exe",
    in2: "wrap.args",
    env: false,
};

impl SettingsPanel {
    pub(super) fn advanced_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let sideload = self.facts().is_some_and(|f| f.is_sideload());
        let legendary = self.facts().is_some_and(|f| f.runner == "legendary");
        let game = self.is_game();
        let mut col = column();
        let mut general = rows();
        if game && !sideload {
            general =
                general.child(self.toggle_row("ignore-updates", "ignoreGameUpdates", false, tr!("Ignore game updates"), None, false, cx));
        }
        if game && legendary {
            general = general.child(self.toggle_row("offline", "offlineMode", false, tr!("Run Game Offline"), None, false, cx));
        }
        if game && !self.launch_options.is_empty() {
            general = general.field_help(
                self.launch_option_select(cx),
                vec![help(tr!("How Play starts the game: a mode it offers, or a playable DLC."))],
            );
        }
        general = general.child(self.toggle_row("verbose", "verboseLogs", true, tr!("Enable verbose logs"), None, false, cx));
        if game && !sideload {
            general = general.field(self.path_row(
                "targetExe",
                Some(tr!("Select an alternative EXE to run")),
                false,
                tr!("Select EXE"),
                true,
                cx,
            ));
        }
        if !sideload {
            general = general.field_help(
                self.text_row("language", Some(tr!("Prefered Language (Language Code)")), None),
                vec![help(tr!("Leave blank to use the app's language. Not all games support this configuration, some have in-game language setting. Valid language codes are game-dependant."))],
            );
        }
        col = col.child(heading(tr!("General"))).child(general.card());

        let mut launch = rows();
        if game {
            let args = self.input_text("launcherArgs", cx);
            let err = launcher_args_error(&args);
            let mut notes = Vec::new();
            if !err.is_empty() {
                notes.push(danger_text(err));
            }
            notes.push(help(tr!("Use the Game Arguments to be called after the launch command, for instance: -nolauncher to skip the launcher in some games, etc.")));
            launch = launch
                .field_help(self.text_row("launcherArgs", Some(tr!("Game Arguments (appended to game launch command)")), None), notes);
        }
        launch = launch
            .field(self.path_row(
                "beforeScript",
                Some(tr!("Select a script to run before the game is launched")),
                false,
                tr!("Select script ..."),
                true,
                cx,
            ))
            .field(self.path_row(
                "afterScript",
                Some(tr!("Select a script to run after the game exits")),
                false,
                tr!("Select script ..."),
                true,
                cx,
            ));
        col = col.child(heading(tr!("Launch"))).child(launch.card());
        if self.is_game() {
            col = col.child(self.companions_section(cx));
        }
        col = col.child(self.presets_section(false, cx)).child(self.pair_table(WRAPPERS, cx)).child(self.pair_table(ENV, cx));

        // Legacy options (a tab of its own in Heroic).
        let wine = self.wine_version();
        let show_dxvk_fps = !self.native() && !wine.bin.contains("toolkit") && !wine.name.ends_with("-DXMT");
        let show_umu = game && !self.native() && wine.kind == "proton";
        if show_dxvk_fps || show_umu {
            let mut legacy = rows();
            if show_dxvk_fps {
                legacy = legacy.child(self.toggle_row(
                    "dxvk-fps",
                    "enableDXVKFpsLimit",
                    false,
                    tr!("Limit DirectX Games FPS"),
                    Some(tr!("Sets a frame rate cap for DirectX Games (9-12)")),
                    false,
                    cx,
                ));
                if self.flag("enableDXVKFpsLimit", false) {
                    legacy = legacy.field(self.text_row("dxvkFps", None, None));
                }
            }
            if show_umu {
                legacy = legacy.child(self.toggle_row("disable-umu", "disableUMU", false, tr!("Disable umu"), None, false, cx));
            }
            col = col
                .child(heading(tr!("Legacy")))
                .child(help(tr!("Warning: The settings on this tab are mostly deprecated and might not work at all.")).px(px(6.)))
                .child(legacy.card());
        }
        col
    }

    pub(super) fn pair_table(&mut self, t: PairTable, cx: &mut Context<Self>) -> Div {
        let rows_data = pairs(self.settings.get(t.setting), t.k1, t.k2);
        let editing = self.input_text(t.in1, cx).trim().to_string();
        let exists = rows_data.iter().any(|(a, _)| *a == editing);
        let icon_button = |id: String, ic: Icon| {
            div()
                .id(SharedString::from(id))
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded_full()
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.10)))
                .child(icon(ic, px(16.), theme::text_muted()))
        };
        let mut card_rows = rows();
        for (i, (a, b)) in rows_data.iter().enumerate() {
            let (ea, eb) = (a.clone(), b.clone());
            card_rows = card_rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .child(div().w(px(220.)).flex_none().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(a.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(theme::text_muted()).child(b.clone()))
                    .child(icon_button(format!("{}-edit-{i}", t.id), Icon::Edit).on_click(cx.listener(move |this, _, window, cx| {
                        this.set_input(t.in1, &ea, window, cx);
                        this.set_input(t.in2, &eb, window, cx);
                        cx.notify();
                    })))
                    .child(
                        icon_button(format!("{}-del-{i}", t.id), Icon::DeleteOutline)
                            .on_click(cx.listener(move |this, _, _, cx| this.pair_remove(t, i, cx))),
                    ),
            );
        }
        let add = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(12.))
            .child(div().flex_1().child(input_box(&self.input(t.in1), false)))
            .child(div().flex_1().child(input_box(&self.input(t.in2), false)))
            .child(action_button(
                SharedString::from(format!("{}-add", t.id)),
                ButtonKind::Glass,
                if exists { tr!("Update") } else { tr!("Add") },
                true,
                cx.listener(move |this, _, window, cx| this.pair_add(t, window, cx)),
            ));
        card_rows = card_rows.child(add);
        let mut col = column().child(heading(tr(t.title).trim_end_matches(':'))).child(card_rows.card());
        if t.env && !self.env_error.is_empty() {
            col = col.child(danger_text(&self.env_error).px(px(6.)));
        }
        let note = if t.env {
            tr!("Set environment variables to append to the command. Do NOT include the \"=\" sign, e.g: for a setting like \"MY_FLAG=123\", set MY_FLAG in NAME and 123 in VALUE.")
        } else {
            tr!("Warning: Make sure to quote args with spaces! E.g.: \"path/with spaces/\". Arguments example: --arg; --extra-file=\"file-path/ with/spaces\"")
        };
        col.child(help(note).px(px(6.)))
    }

    pub(super) fn pair_remove(&mut self, t: PairTable, index: usize, cx: &mut Context<Self>) {
        let mut rows = pairs(self.settings.get(t.setting), t.k1, t.k2);
        if index < rows.len() {
            rows.remove(index);
            self.set(t.setting, pairs_to_value(&rows, t.k1, t.k2), cx);
        }
    }

    pub(super) fn pair_add(&mut self, t: PairTable, window: &mut Window, cx: &mut Context<Self>) {
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

impl SettingsPanel {
    /// `lastUsedLaunchOption` as stored, if it is still one of the game's.
    fn launch_option_current(&self) -> Option<heronite_launch::launch_option::LaunchOption> {
        self.settings
            .get(heronite_launch::launch_option::KEY)
            .and_then(heronite_launch::launch_option::LaunchOption::from_value)
            .filter(|o| self.launch_options.contains(o))
    }

    /// The menu's names (the default first) and the current one.
    pub(super) fn launch_option_choices(&self) -> (Vec<SharedString>, usize) {
        let current = self.launch_option_current();
        let names = std::iter::once(SharedString::from(tr!("Default")))
            .chain(self.launch_options.iter().map(|o| SharedString::from(o.label().to_string())))
            .collect();
        let index = current.and_then(|c| self.launch_options.iter().position(|o| *o == c)).map_or(0, |i| i + 1);
        (names, index)
    }

    /// `i`: 0 is the default, then the options in order.
    pub(super) fn pick_launch_option(&mut self, i: usize, cx: &mut Context<Self>) {
        let value = i.checked_sub(1).and_then(|i| self.launch_options.get(i)).map(|o| o.to_value()).unwrap_or(serde_json::Value::Null);
        self.set(heronite_launch::launch_option::KEY, value, cx);
    }

    fn launch_option_select(&self, cx: &mut Context<Self>) -> Div {
        let (names, index) = self.launch_option_choices();
        let options = names.iter().enumerate().map(|(i, n)| (n.clone(), i == index)).collect();
        self.select_row("launch-option", Some(tr!("Start with")), names[index].to_string(), options, false, cx, |this, i, _, cx| {
            this.pick_launch_option(i, cx)
        })
    }
}
