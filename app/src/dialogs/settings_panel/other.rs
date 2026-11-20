//! "Performance" tab: what is on screen (overlays), the replay of a game (on
//! or off, `replay`), the FPS limit, frame generation, the image (gamescope,
//! Wine's FSR) and system tweaks while playing. Heroic's Other and Gamescope
//! tabs.

use crate::icons::Icon;
use crate::ui::controls::{icon_button_sm, row, toggle, ButtonKind};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::fps_limit::KEY_MANGOHUD_HIDDEN;
use serde_json::{json, Value};

use super::logic;
use super::parts::{column, heading, rows, Rows};
use super::SettingsPanel;
use heronite_i18n::tr;

impl SettingsPanel {
    pub(super) fn perf_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let native = self.native();
        let mut col = column().child(heading(tr!("On screen"))).child(self.overlay_rows(false, cx).card());
        col = self.replay_section(col, cx);
        col = col.child(self.fps_section(false, cx)).child(self.lsfg_section(false, cx));
        // Linux games get a line about vkBasalt instead.
        if self.reshade_applies() || (native && cfg!(not(windows))) {
            col = col.child(self.reshade_section(cx));
        }
        let mut image = rows();
        if !native {
            image = self.fsr_rows(image, cx);
        }
        col = col.child(self.gamescope_section(image, false, cx));
        col.child(heading(tr!("System"))).child(self.game_system_rows(false, cx).card())
    }

    /// The FPS counter of DXVK and MangoHud.
    pub(super) fn overlay_rows(&self, disabled: bool, cx: &mut Context<Self>) -> Rows {
        let mut r = rows();
        if !self.native() {
            r = r.child(self.toggle_row(
                "show-fps",
                "showFps",
                false,
                tr!("FPS counter"),
                Some(tr!("A small counter for DirectX 9 to 11 games")),
                disabled,
                cx,
            ));
        }
        r.child(self.mangohud_row(disabled, cx))
    }

    /// MangoHud's switch, with an eye button before it while it is on: shown
    /// or hidden when the game starts.
    fn mangohud_row(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let on = self.flag("showMangohud", false);
        let hidden = on && self.flag(KEY_MANGOHUD_HIDDEN, false);
        let help =
            if hidden { self.mangohud_hidden_help() } else { tr!("FPS, frame times, temperatures and load (needs MangoHud)").to_string() };
        let eye = on.then(|| {
            let (ic, label, tip) = if hidden {
                (Icon::VisibilityOff, tr!("Hidden"), tr!("Starts hidden. Click to show it when the game starts."))
            } else {
                (Icon::Visibility, tr!("Shown"), tr!("Shows when the game starts. Click to start it hidden."))
            };
            let click = cx.listener(|this, _, _, cx| {
                // Not the row's click: MangoHud stays on.
                cx.stop_propagation();
                this.toggle(KEY_MANGOHUD_HIDDEN, false, cx);
            });
            icon_button_sm("mangohud-eye", ButtonKind::Glass, ic, label, !disabled, click).tooltip(crate::ui::tooltip(tip))
        });
        let control = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .children(self.reset_button("showMangohud", cx))
            .children(on.then(|| self.reset_button(KEY_MANGOHUD_HIDDEN, cx)).flatten())
            .children(eye)
            .child(toggle("mangohud-sw", on, false));
        let row = row("MangoHud", Some(&help), control).id("mangohud").cursor_pointer();
        if disabled {
            row.opacity(0.4).cursor_not_allowed().into_any_element()
        } else {
            row.on_click(cx.listener(|this, _, _, cx| this.toggle("showMangohud", false, cx))).into_any_element()
        }
    }

    /// What shows MangoHud when it starts hidden: the in-game overlay can
    /// for the games Heronite starts, not for Steam's.
    pub(super) fn mangohud_hidden_help(&self) -> String {
        if self.steam.is_some() {
            tr!("Starts hidden: its toggle key shows it (Right Shift + F12 unless changed).").into()
        } else {
            tr!("Starts hidden: its toggle key (Right Shift + F12 unless changed) or the in-game overlay shows it.").into()
        }
    }

    /// Wine's own FSR upscaling in fullscreen, with its sharpness.
    fn fsr_rows(&self, mut r: Rows, cx: &mut Context<Self>) -> Rows {
        r = r.child(self.toggle_row(
            "fsr",
            "enableFSR",
            false,
            tr!("Wine FSR"),
            Some(tr!("Upscales fullscreen games running below the screen's resolution (needs a Wine build with the FSR hack)")),
            false,
            cx,
        ));
        if self.flag("enableFSR", false) {
            let sharp = self.number("maxSharpness").unwrap_or(5).clamp(1, 5);
            r = r.field(self.select_row(
                "fsr-sharpness",
                Some(tr!("FSR sharpness")),
                sharp.to_string(),
                (1..=5).map(|n| (SharedString::from(n.to_string()), n == sharp)).collect(),
                false,
                cx,
                |this, i, _, cx| this.set("maxSharpness", json!(i as i64 + 1), cx),
            ));
        }
        r
    }

    /// GameMode, keeping the computer awake, controllers.
    pub(super) fn game_system_rows(&self, disabled: bool, cx: &mut Context<Self>) -> Rows {
        let mut r = rows().child(self.toggle_row(
            "gamemode",
            "useGameMode",
            false,
            "GameMode",
            Some(tr!("System tweaks while the game runs (needs GameMode)")),
            disabled,
            cx,
        ));
        // Steam keeps the computer awake itself while a game runs.
        if !self.is_steam() {
            r = r.child(self.toggle_row(
                "keep-awake",
                heronite_launch::keep_awake::KEY,
                true,
                tr!("Keep the computer awake"),
                Some(tr!("A controller does not count as activity, so the computer could sleep mid-game")),
                disabled,
                cx,
            ));
        }
        if self.is_steam() || (!self.native() && self.wine_version().kind == "proton") {
            r = r.child(self.sdl_row(disabled, cx));
        }
        r
    }

    /// `PROTON_PREFER_SDL` (Proton only).
    pub(super) fn sdl_row(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        self.toggle_row(
            "prefer-sdl",
            heronite_launch::env::KEY_PREFER_SDL,
            false,
            tr!("Read controllers through SDL"),
            Some(tr!("Fixes a controller the game misses or buttons that repeat; PlayStation extras (gyro, touchpad) stop working")),
            disabled,
            cx,
        )
    }

    pub(super) fn steam_runtime_field(&self, cx: &mut Context<Self>) -> Div {
        let current = match self.settings.get("steamRuntime") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            _ => "none".to_string(),
        };
        let mut items: Vec<(SharedString, bool)> = vec![(tr!("None").into(), current == "none")];
        items.extend(logic::STEAM_RUNTIMES.iter().map(|(id, name)| (SharedString::from(*name), *id == current)));
        let shown = logic::STEAM_RUNTIMES
            .iter()
            .find(|(id, _)| *id == current)
            .map(|(_, n)| (*n).to_string())
            .unwrap_or_else(|| tr!("None").into());
        self.select_row("steam-runtime", Some(tr!("Steam Runtime")), shown, items, false, cx, |this, i, _, cx| {
            let v = if i == 0 { Value::Bool(false) } else { Value::String(logic::STEAM_RUNTIMES[i - 1].0.into()) };
            this.set("steamRuntime", v, cx);
        })
    }
}
