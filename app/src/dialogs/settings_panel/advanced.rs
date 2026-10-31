//! The panel's "Launch" section (Heroic's Advanced tab): arguments, offline
//! and updates, language, scripts, programs alongside, then the advanced
//! part (alternative exe, logs, runtime, umu, presets, wrappers, variables).

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
    title: n_!("Environment variables"),
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
    /// The game's launch settings (Heroic's Advanced tab); the Launch tab of
    /// the game settings shows them under Play and the actions.
    pub(super) fn launch_opts_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let sideload = self.facts().is_some_and(|f| f.is_sideload());
        let legendary = self.facts().is_some_and(|f| f.runner == "legendary");
        let game = self.is_game();
        let native = self.native();
        let mut col = column();

        let mut start = rows();
        if game && !self.launch_options.is_empty() {
            start = start.field_help(self.launch_option_select(cx), vec![help(tr!("A mode the game offers, or a playable DLC"))]);
        }
        if game {
            let err = launcher_args_error(&self.input_text("launcherArgs", cx));
            let mut notes = Vec::new();
            if !err.is_empty() {
                notes.push(danger_text(err));
            }
            notes.push(help(tr!("Added after the game's command, like -nolauncher")));
            start = start.field_help(self.text_row("launcherArgs", Some(tr!("Game arguments")), None, cx), notes);
        }
        if game && legendary {
            start = start.child(self.toggle_row(
                "offline",
                "offlineMode",
                false,
                tr!("Play offline"),
                Some(tr!("Starts without signing in to Epic")),
                false,
                cx,
            ));
        }
        if game && !sideload {
            start = start.child(self.toggle_row(
                "ignore-updates",
                "ignoreGameUpdates",
                false,
                tr!("Ignore updates"),
                Some(tr!("Play even when an update is waiting")),
                false,
                cx,
            ));
        }
        if !sideload {
            start = start.field_help(
                self.text_row("language", Some(tr!("Game language")), None, cx),
                vec![help(tr!("A code like en or pt-BR; empty: the app's language. Not every game reads it."))],
            );
        }
        col = col.child(heading(tr!("Starting"))).child(start.card());
        let extras = logic::extras();
        if extras {
            col = col.child(self.scripts_section(false, cx));
        }
        if game && extras {
            col = col.child(self.companions_section(cx));
        }

        let mut advanced = rows();
        if game && !sideload {
            advanced = advanced.field(self.path_row("targetExe", Some(tr!("Alternative executable")), false, tr!("Select EXE"), true, cx));
        }
        advanced = advanced.child(self.toggle_row(
            "verbose",
            "verboseLogs",
            true,
            tr!("Detailed logs"),
            Some(tr!("More detail in the game's log, for troubleshooting")),
            false,
            cx,
        ));
        if native && extras {
            advanced = advanced.field_help(
                self.steam_runtime_field(cx),
                vec![help(tr!("Steam's libraries for Linux games; can help a game that misses some"))],
            );
        }
        if game && !native && self.wine_version().kind == "proton" {
            advanced = advanced.child(self.toggle_row(
                "disable-umu",
                "disableUMU",
                false,
                tr!("Disable umu"),
                Some(tr!("Runs Proton directly, without umu and its game fixes")),
                false,
                cx,
            ));
        }
        col.child(heading(tr!("Advanced")))
            .child(advanced.card())
            .when(extras, |c| c.child(self.presets_section(false, cx)).child(self.pair_table(WRAPPERS, cx)))
            .child(self.pair_table(ENV, cx))
            .when(self.has_preview(), |c| c.child(self.preview_section(cx)))
    }

    /// The user's scripts before the game starts and after it exits.
    /// `steam`: Steam runs them through Heronite's wrapper script.
    pub(super) fn scripts_section(&mut self, steam: bool, cx: &mut Context<Self>) -> Div {
        let mut r = rows()
            .field(self.path_row("beforeScript", Some(tr!("Before the game starts")), false, tr!("Select script ..."), true, cx))
            .field(self.path_row("afterScript", Some(tr!("After the game exits")), false, tr!("Select script ..."), true, cx));
        if steam {
            r = r.child(help(tr!("Steam runs them through a small Heronite script, however the game starts.")).pb(px(12.)));
        }
        column().child(heading(tr!("Scripts"))).child(r.card())
    }

    pub(super) fn pair_table(&mut self, t: PairTable, cx: &mut Context<Self>) -> Div {
        if t.env {
            if let Some(text) = self.env_text.clone() {
                return self.env_text_editor(&text, cx);
            }
        }
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
                .hover(|s| s.bg(theme::ink_a(0.10)))
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
        let mut title_row =
            div().flex().items_end().justify_between().child(self.marked_heading(tr(t.title).trim_end_matches(':'), t.setting, cx));
        if t.env {
            title_row = title_row.child(
                action_button(
                    "env-as-text",
                    ButtonKind::Ghost,
                    tr!("Edit as text"),
                    true,
                    cx.listener(|this, _, window, cx| this.open_env_text(window, cx)),
                )
                .mb(px(4.)),
            );
        }
        let mut col = column().child(title_row).child(card_rows.card());
        if t.env && !self.env_error.is_empty() {
            col = col.child(danger_text(&self.env_error).px(px(6.)));
        }
        let note = if t.env {
            tr!("Set environment variables to append to the command. Do NOT include the \"=\" sign, e.g: for a setting like \"MY_FLAG=123\", set MY_FLAG in NAME and 123 in VALUE.")
        } else {
            tr!("Quote arguments with spaces, like \"my folder/game.ini\"")
        };
        col.child(help(note).px(px(6.)))
    }

    /// Opens the variables as text, one `NAME=value` per line.
    pub(super) fn open_env_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = logic::env_block(&pairs(self.settings.get(ENV.setting), ENV.k1, ENV.k2));
        self.env_text = Some(crate::ui::fields::new_textarea(window, cx, &current, 8));
        self.env_error.clear();
        cx.notify();
    }

    /// Replaces every variable with the text's. `false` (and the reason in
    /// `env_error`) when a line is not `NAME=value`.
    pub(super) fn apply_env_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        match logic::parse_env_block(text) {
            Ok(rows) => {
                self.env_error.clear();
                self.set(ENV.setting, pairs_to_value(&rows, ENV.k1, ENV.k2), cx);
                true
            }
            Err(why) => {
                self.env_error = why;
                cx.notify();
                false
            }
        }
    }

    fn env_text_editor(&mut self, text: &Entity<gpui_kit::component::input::TextareaState>, cx: &mut Context<Self>) -> Div {
        let box_ = div().flex().flex_col().gap(px(10.)).py(px(12.)).child(crate::ui::fields::textarea_box(text, 8)).child(
            div()
                .flex()
                .gap(px(8.))
                .justify_end()
                .child(action_button(
                    "env-text-cancel",
                    ButtonKind::Ghost,
                    tr!("Cancel"),
                    true,
                    cx.listener(|this, _, _, cx| {
                        this.env_text = None;
                        this.env_error.clear();
                        cx.notify();
                    }),
                ))
                .child(action_button(
                    "env-text-apply",
                    ButtonKind::Primary,
                    tr!("Apply"),
                    true,
                    cx.listener(|this, _, _, cx| {
                        let Some(text) = this.env_text.clone() else { return };
                        let value = text.read(cx).value().to_string();
                        if this.apply_env_text(&value, cx) {
                            this.env_text = None;
                        }
                        cx.notify();
                    }),
                )),
        );
        let mut col = column().child(self.marked_heading(tr(ENV.title), ENV.setting, cx)).child(rows().child(box_).card());
        if !self.env_error.is_empty() {
            col = col.child(danger_text(&self.env_error).px(px(6.)));
        }
        col.child(help(tr!(
            "One NAME=value per line. A line copied from ProtonDB like PROTON_NO_ESYNC=1 DXVK_ASYNC=1 %command% works too. Applying replaces the list."
        )).px(px(6.)))
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
