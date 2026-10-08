//! "Other" tab: overlays, GameMode, runtimes.

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::logic;
use super::parts::{column, help};
use super::HeroicSettingsPanel;

impl HeroicSettingsPanel {
    pub(super) fn other_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let native = self.native();
        let wine = self.wine_version();
        let mut col = column();
        if !native {
            col = col.child(self.toggle_row("show-fps", "showFps", false, "Show FPS (DX9, 10 and 11)", None, false, cx));
        }
        col = col
            .child(self.toggle_row(
                "mangohud",
                "showMangohud",
                false,
                "Enable Mangohud (Mangohud needs to be installed)",
                Some("MangoHUD is an overlay that displays and monitors FPS, temperatures, CPU/GPU load and other system resources."),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "gamemode",
                "useGameMode",
                false,
                "Use GameMode (Feral Game Mode needs to be installed)",
                Some("Feral GameMode applies automatic and temporary tweaks to the system when running games. Enabling may improve performance."),
                false,
                cx,
            ));
        if !native && logic::offers_system_libs(&wine) {
            col = col.child(self.toggle_row(
                "system-libs",
                "preferSystemLibs",
                false,
                "Prefer system libraries",
                Some("Custom Wine versions (Wine-GE, Wine-Lutris) are shipped with their library dependencies. By enabling this option, these shipped libraries will be ignored and Wine will load system libraries instead. Warning! Issues may occur if dependencies are not met."),
                false,
                cx,
            ));
        }
        if native {
            col = col.child(self.steam_runtime_field(cx));
        }
        if self.gpu_count >= 2 || self.flag("nvidiaPrime", false) {
            col = col.child(self.toggle_row(
                "nvidia-prime",
                "nvidiaPrime",
                false,
                "Force use of NVIDIA Optimus or AMD Dynamic Switchable Graphics dGPU. ONLY use this for OpenGL and Vulkan games.",
                Some("Use dedicated graphics card to render game on multi-GPU systems. Only needed on gaming laptops or desktops that use a headless GPU for rendering (NVIDIA Optimus, AMD Dynamic Switchable Graphics)."),
                false,
                cx,
            ));
        }
        if !native {
            col = col.child(self.toggle_row("battleye", "battlEyeRuntime", false, "BattlEye AntiCheat Runtime", None, false, cx));
            if self.flag("battlEyeRuntime", false) && !self.runtimes.0 {
                col = col.child(help("The BattlEye runtime is not installed yet. Heroic downloads it when you enable it there.").pl(px(41.)));
            }
            col = col.child(self.toggle_row("eac", "eacRuntime", false, "EasyAntiCheat Runtime", None, false, cx));
            if self.flag("eacRuntime", false) && !self.runtimes.1 {
                col = col.child(help("The EAC runtime is not installed yet. Heroic downloads it when you enable it there.").pl(px(41.)));
            }
        }
        col
    }

    fn steam_runtime_field(&self, cx: &mut Context<Self>) -> Div {
        let current = match self.settings.get("steamRuntime") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            _ => "none".to_string(),
        };
        let mut items: Vec<(SharedString, bool)> = vec![("None".into(), current == "none")];
        items.extend(logic::STEAM_RUNTIMES.iter().map(|(id, name)| (SharedString::from(*name), *id == current)));
        let shown = logic::STEAM_RUNTIMES.iter().find(|(id, _)| *id == current).map(|(_, n)| (*n).to_string()).unwrap_or_else(|| "None".into());
        column().gap(px(4.)).child(self.select_row("steam-runtime", Some("Use Steam Runtime"), shown, items, false, cx, |this, i, _, cx| {
            let v = if i == 0 { Value::Bool(false) } else { Value::String(logic::STEAM_RUNTIMES[i - 1].0.into()) };
            this.set("steamRuntime", v, cx);
        })).child(help("Custom libraries provided by Steam to help run Linux and Windows (Proton) games. Enabling might improve compatibility."))
    }
}
