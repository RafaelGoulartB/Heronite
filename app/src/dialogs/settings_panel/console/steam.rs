//! Console rows of the Steam tabs (see `settings_panel::steam`).

use std::rc::Rc;

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::wine_tools::WineTool;
use heronite_steam::options;

use super::super::advanced::{ENV, WRAPPERS};
use super::super::logic::launcher_args_error;
use super::super::protondb::{tier_advice, tier_name, ProtonDb, Recommendation};
use super::super::steam::Restart;
use super::{item, Act, Item, SettingsPanel};
use crate::console::list::Row;
use crate::icons::Icon;

impl SettingsPanel {
    /// Where the settings go, or why they cannot change, plus "read again".
    fn c_steam_status(&self) -> Vec<Item> {
        let (text, error) = self.steam_status();
        let mut v = vec![Self::c_info("steam-status", if error { tr!("Problem") } else { "Steam" }, text)];
        if !self.steam.as_ref().is_some_and(|s| s.editable) {
            v.push(Self::c_info("steam-shell", tr!("Launch options"), tr!("They use shell syntax; edit them as text in Advanced")));
        }
        let read_only = self.steam_locked().is_some_and(|w| w == heronite_steam::read_only_message());
        if read_only {
            v.extend(self.c_restart());
        }
        v.push(Self::c_do("steam-reload", tr!("Read from Steam again"), Icon::Refresh, true, |this, _, cx| {
            this.notice = None;
            this.load_steam(cx)
        }));
        v
    }

    /// "Restart Steam", asking once before Steam's games close.
    fn c_restart(&self) -> Vec<Item> {
        match self.steam.as_ref().map(|s| s.restart).unwrap_or_default() {
            Restart::Idle => vec![Self::c_do("steam-restart", tr!("Restart Steam"), Icon::Refresh, true, |this, _, cx| {
                this.steam_restart_state(Restart::Confirm, cx)
            })],
            Restart::Confirm => vec![
                Self::c_do(
                    "steam-restart-yes",
                    tr!("Restart now: games running from Steam will close"),
                    Icon::Refresh,
                    true,
                    |this, _, cx| this.restart_steam(cx),
                ),
                Self::c_do("steam-restart-no", tr!("Cancel"), Icon::Close, true, |this, _, cx| this.steam_restart_state(Restart::Idle, cx)),
            ],
            Restart::Running(_) => vec![Self::c_info("steam-restart", "Steam", tr!("Restarting Steam\u{2026}"))],
        }
    }

    /// The ProtonDB tier and a row that picks the suggested build.
    pub(super) fn c_protondb(&self, rec: Option<Recommendation>, pick: fn(&mut Self, usize, &mut Context<Self>)) -> Vec<Item> {
        let mut v = Vec::new();
        match &self.protondb {
            ProtonDb::Off => {}
            ProtonDb::Loading => v.push(Self::c_info("protondb", "ProtonDB", tr!("Asking ProtonDB\u{2026}"))),
            ProtonDb::Tier(t) => v.push(Self::c_info("protondb", "ProtonDB", format!("{}: {}", tier_name(t), tier_advice(t)))),
        }
        if let Some(rec) = rec {
            match (rec.in_use, rec.index) {
                (true, _) => v.push(Self::c_info("protondb-rec", tr!("Recommended"), tr!("{build} (in use)", build = rec.label))),
                (false, Some(i)) => v.push(Self::c_do(
                    "protondb-use",
                    &tr!("Use the recommended build: {build}", build = rec.label),
                    Icon::WineBar,
                    true,
                    move |this, _, cx| pick(this, i, cx),
                )),
                (false, None) => {
                    v.push(Self::c_info("protondb-rec", tr!("Recommended"), tr!("{build}, from the Wine Manager", build = rec.label)))
                }
            }
        }
        v
    }

    fn c_steam_toggle(&self, key: &'static str, default: bool, label: &str, desc: Option<&str>) -> Item {
        self.c_toggle(key, default, label, desc, !self.steam_fields_editable())
    }

    pub(super) fn c_proton(&self) -> Vec<Item> {
        let mut v = self.c_steam_status();
        let options = self.steam_tool_options();
        let current = self.steam_current_tool();
        let names = options
            .iter()
            .map(|o| {
                SharedString::from(match o.note {
                    Some(note) => format!("{} \u{b7} {note}", o.label),
                    None => o.label.clone(),
                })
            })
            .collect();
        let selected = options.iter().position(|o| o.name == current).unwrap_or(0);
        if let Some(text) = self.steam_unseen_text() {
            v.push(Self::c_info("steam-unseen", tr!("New builds"), text));
            if !self.steam_locked().is_some_and(|w| w == heronite_steam::read_only_message()) {
                v.extend(self.c_restart());
            }
        }
        v.push(Self::c_header("Proton"));
        v.push(item(
            Row::choice("c:steam-tool", tr!("Compatibility tool"), names, selected)
                .icon(Icon::WineBar)
                .desc(tr!("Steam only sees a new build after it restarts"))
                .enabled(self.steam_locked().is_none()),
            Act::Choice(Rc::new(|this, i, cx| this.pick_steam_tool(i, cx))),
        ));
        let rec = self.steam_recommendation(&options).filter(|_| self.steam_locked().is_none());
        v.extend(self.c_protondb(rec, |this, i, cx| this.pick_steam_tool(i, cx)));
        if self.steam.as_ref().is_some_and(|s| s.editable) {
            v.push(Self::c_header(tr!("Proton options")));
            v.push(self.c_steam_toggle("enableEsync", true, "Esync", None));
            v.push(self.c_steam_toggle("enableFsync", true, "Fsync", None));
            v.push(self.c_steam_toggle(
                "enableWineWayland",
                false,
                tr!("Wayland driver (experimental)"),
                Some(tr!("Proton 10 or newer, or GE-Proton")),
            ));
            if self.flag("enableWineWayland", false) || self.flag("enableHDR", false) {
                v.push(self.c_steam_toggle("enableHDR", false, tr!("HDR"), Some(tr!("The display and the game must support it"))));
            }
            v.push(self.c_steam_toggle("enableWoW64", false, tr!("WoW64 (experimental)"), Some(tr!("32-bit games on 64-bit Wine"))));
            v.push(self.c_steam_toggle(options::KEY_NVAPI, false, tr!("Enable NVAPI"), Some(tr!("DLSS and other NVIDIA features"))));
            v.push(self.c_steam_toggle(
                options::KEY_WINED3D,
                false,
                tr!("Use WineD3D instead of DXVK and VKD3D"),
                Some(tr!("OpenGL-based; slower, but can help games that misbehave with Vulkan.")),
            ));
        }
        v.push(Self::c_header(tr!("Prefix")));
        let prefix = self.steam_drive_c();
        if prefix.is_none() {
            v.push(Self::c_info("steam-pfx", tr!("Prefix"), tr!("Created when Steam first runs the game")));
            return v;
        }
        let can_run = self.tool_busy.is_none() && self.steam_tool_settings().is_some();
        v.push(Self::c_do("steam-winecfg", "Winecfg", Icon::Tune, can_run, |this, _, cx| this.run_tool("winecfg", WineTool::Winecfg, cx)));
        v.push(Self::c_do("steam-winetricks", "Winetricks", Icon::Build, can_run, |this, _, cx| {
            this.run_tool("winetricks", WineTool::Winetricks, cx)
        }));
        v.push(Self::c_do("steam-runexe", tr!("Run a program in the prefix\u{2026}"), Icon::PlayArrow, can_run, |this, window, cx| {
            this.console_pick_file(
                tr!("Run a program in the prefix"),
                vec!["exe".into(), "msi".into(), "bat".into()],
                window,
                cx,
                |this, path, cx| this.run_tool("runexe", WineTool::RunExe(path), cx),
            )
        }));
        v.push(Self::c_do("steam-openpfx", tr!("Open prefix folder"), Icon::FolderOpen, true, |this, _, cx| this.open_steam_prefix(cx)));
        v
    }

    pub(super) fn c_steam_other(&self, cx: &App) -> Vec<Item> {
        let mut v = self.c_steam_status();
        if !self.steam.as_ref().is_some_and(|s| s.editable) {
            return v;
        }
        v.push(Self::c_header(tr!("Overlays and performance")));
        v.push(self.c_steam_toggle("showFps", false, tr!("Show FPS (DX9, 10 and 11)"), None));
        v.push(self.c_steam_toggle(
            "showMangohud",
            false,
            "MangoHud",
            Some(tr!("FPS, temperatures and load overlay (needs MangoHud installed)")),
        ));
        if self.flag("showMangohud", false) {
            v.push(self.c_text("mangohudFps", tr!("MangoHud FPS limit"), cx));
        }
        v.push(self.c_steam_toggle("useGameMode", false, "GameMode", Some(tr!("Feral GameMode system tweaks (needs GameMode installed)"))));
        if self.gpu_count >= 2 || self.flag("nvidiaPrime", false) {
            v.push(self.c_steam_toggle(
                "nvidiaPrime",
                false,
                tr!("Use the dedicated GPU"),
                Some(tr!("NVIDIA Optimus / AMD switchable graphics; OpenGL and Vulkan games only")),
            ));
        }
        v
    }

    pub(super) fn c_steam_advanced(&self, cx: &App) -> Vec<Item> {
        let mut v = self.c_steam_status();
        if self.steam.as_ref().is_some_and(|s| s.editable) {
            v.push(Self::c_header(tr!("Launch")));
            v.push(self.c_steam_toggle(
                "verboseLogs",
                false,
                tr!("Proton logs"),
                Some(tr!("Proton writes steam-<app id>.log in your home folder.")),
            ));
            let err = launcher_args_error(&self.input_text("launcherArgs", cx));
            let mut it = self.c_text("launcherArgs", tr!("Game arguments"), cx);
            it.row = it.row.desc(if err.is_empty() { tr!("Appended to the launch command, like -nolauncher") } else { err });
            v.push(it);
            v.push(self.c_path("beforeScript", tr!("Script before launch"), false, cx));
            v.push(self.c_path("afterScript", tr!("Script after exit"), false, cx));
            v.extend(self.c_pairs(WRAPPERS));
            v.extend(self.c_pairs(ENV));
            v.push(Self::c_header("DirectX"));
            v.push(self.c_steam_toggle("enableDXVKFpsLimit", false, tr!("Limit DirectX games FPS"), None));
            if self.flag("enableDXVKFpsLimit", false) {
                v.push(self.c_text("dxvkFps", tr!("FPS limit"), cx));
            }
        }
        v.push(Self::c_header("Steam"));
        v.push(self.c_text("steamLine", tr!("Steam launch options"), cx));
        v
    }
}
