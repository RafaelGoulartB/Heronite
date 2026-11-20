//! "Performance" group: what is on screen (overlays), the replay of a game
//! (on or off, `replay`), the FPS limit, frame generation (Lossless Scaling
//! and NVIDIA's Smooth Motion), latency and system tweaks while playing.
//! Heroic's Other tab.

use crate::icons::Icon;
use crate::ui::controls::{icon_button_sm, row, toggle, ButtonKind};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::fps_limit::KEY_MANGOHUD_HIDDEN;
use serde_json::Value;

use super::gpu::GpuPart;
use super::logic;
use super::parts::{column, heading, rows, Rows};
use super::SettingsPanel;
use heronite_i18n::tr;

impl SettingsPanel {
    /// `off`: a Steam game whose launch options cannot change now.
    pub(super) fn perf_tab(&mut self, cx: &mut Context<Self>) -> Div {
        self.perf_rows(false, cx)
    }

    pub(super) fn perf_rows(&mut self, off: bool, cx: &mut Context<Self>) -> Div {
        let mut col = column().child(heading(tr!("On screen"))).child(self.overlay_rows(off, cx).card());
        col = self.replay_section(col, cx);
        let frames = self.gpu_card(rows(), GpuPart::Frames, off, cx);
        col = col.child(self.fps_section(off, cx)).child(self.lsfg_section(off, frames, cx));
        col = col.children(self.gpu_card(rows(), GpuPart::Latency, off, cx).titled(tr!("Latency")));
        let system = self.game_system_rows(off, cx);
        col.children(self.gpu_card(system, GpuPart::Tweaks, off, cx).titled(tr!("System")))
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

    /// GameMode and keeping the computer awake.
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
        if !self.is_steam() && self.shows(heronite_launch::keep_awake::KEY) {
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
        r
    }

    /// Whether the game's Proton reads `PROTON_PREFER_SDL` (Steam's too).
    pub(super) fn has_sdl_row(&self) -> bool {
        self.is_steam() || (!self.native() && self.wine_version().kind == "proton")
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
