//! "Other" tab: overlays, GameMode, frame generation, controllers and
//! power, runtimes.

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::logic;
use super::parts::{column, heading, help, rows};
use super::SettingsPanel;
use heronite_i18n::tr;

impl SettingsPanel {
    pub(super) fn other_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let native = self.native();
        let wine = self.wine_version();
        let mut col = column();
        let mut overlays = rows();
        if !native {
            overlays = overlays.child(self.toggle_row("show-fps", "showFps", false, tr!("Show FPS (DX9, 10 and 11)"), None, false, cx));
        }
        overlays = overlays.child(self.toggle_row(
            "mangohud",
            "showMangohud",
            false,
            tr!("Enable Mangohud (Mangohud needs to be installed)"),
            Some(tr!("MangoHUD is an overlay that displays and monitors FPS, temperatures, CPU/GPU load and other system resources.")),
            false,
            cx,
        ));
        if self.flag("showMangohud", false) {
            overlays = overlays.field(self.text_row(
                "mangohudFps",
                Some(tr!("FPS limit")),
                Some(tr!("MangoHud's frame rate cap. Works with Vulkan, OpenGL and DirectX games. Leave empty for no limit.")),
            ));
        }
        overlays = overlays.child(self.toggle_row(
            "gamemode",
            "useGameMode",
            false,
            tr!("Use GameMode (Feral Game Mode needs to be installed)"),
            Some(tr!(
                "Feral GameMode applies automatic and temporary tweaks to the system when running games. Enabling may improve performance."
            )),
            false,
            cx,
        ));
        if !native && logic::offers_system_libs(&wine) {
            overlays = overlays.child(self.toggle_row(
                "system-libs",
                "preferSystemLibs",
                false,
                tr!("Prefer system libraries"),
                Some(tr!("Custom Wine versions (Wine-GE, Wine-Lutris) are shipped with their library dependencies. By enabling this option, these shipped libraries will be ignored and Wine will load system libraries instead. Warning! Issues may occur if dependencies are not met.")),
                false,
                cx,
            ));
        }
        if self.gpu_count >= 2 || self.flag("nvidiaPrime", false) {
            overlays = overlays.child(self.toggle_row(
                "nvidia-prime",
                "nvidiaPrime",
                false,
                tr!("Force use of NVIDIA Optimus or AMD Dynamic Switchable Graphics dGPU. ONLY use this for OpenGL and Vulkan games."),
                Some(tr!("Use dedicated graphics card to render game on multi-GPU systems. Only needed on gaming laptops or desktops that use a headless GPU for rendering (NVIDIA Optimus, AMD Dynamic Switchable Graphics).")),
                false,
                cx,
            ));
        }
        col = col.child(heading(tr!("Overlays and performance"))).child(overlays.card());
        col = col.child(self.lsfg_section(false, cx));
        let mut system = rows().child(self.toggle_row(
            "keep-awake",
            heronite_launch::keep_awake::KEY,
            false,
            tr!("Keep the computer awake while playing"),
            Some(tr!("Stops the computer from suspending or going idle while the game runs. Useful with a controller, which the system does not count as activity.")),
            false,
            cx,
        ));
        if !native && wine.kind == "proton" {
            system = system.child(self.sdl_row(false, cx));
        }
        col = col.child(heading(tr!("Controllers and power"))).child(system.card());
        if native {
            col = col.child(heading(tr!("Runtime"))).child(rows().field_help(
                self.steam_runtime_field(cx),
                vec![help(tr!("Custom libraries provided by Steam to help run Linux and Windows (Proton) games. Enabling might improve compatibility."))],
            ).card());
        }
        if !native {
            let note = |on: bool, installed: bool, name: &'static str| {
                (on && !installed).then(|| tr!("The {name} runtime is not installed yet. It is downloaded before the next launch.", name))
            };
            let be = note(self.flag("battlEyeRuntime", false), self.runtimes.0, "BattlEye");
            let eac = note(self.flag("eacRuntime", false), self.runtimes.1, "EAC");
            let anti = rows()
                .child(self.toggle_row("battleye", "battlEyeRuntime", false, tr!("BattlEye AntiCheat Runtime"), be.as_deref(), false, cx))
                .child(self.toggle_row("eac", "eacRuntime", false, tr!("EasyAntiCheat Runtime"), eac.as_deref(), false, cx));
            col = col.child(heading(tr!("Anti-cheat"))).child(anti.card());
        }
        col
    }

    /// `PROTON_PREFER_SDL` (Proton only).
    pub(super) fn sdl_row(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        self.toggle_row(
            "prefer-sdl",
            heronite_launch::env::KEY_PREFER_SDL,
            false,
            tr!("Use SDL for controllers"),
            Some(tr!("Can fix a controller the game does not see, swapped buttons or double input. PlayStation controllers lose their own features (adaptive triggers, gyro, touchpad).")),
            disabled,
            cx,
        )
    }

    fn steam_runtime_field(&self, cx: &mut Context<Self>) -> Div {
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
        self.select_row("steam-runtime", Some(tr!("Use Steam Runtime")), shown, items, false, cx, |this, i, _, cx| {
            let v = if i == 0 { Value::Bool(false) } else { Value::String(logic::STEAM_RUNTIMES[i - 1].0.into()) };
            this.set("steamRuntime", v, cx);
        })
    }
}
