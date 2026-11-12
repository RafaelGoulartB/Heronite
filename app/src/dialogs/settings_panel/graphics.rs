//! The "Graphics" group: how the game looks. The card that draws it, the
//! picture (HDR, VSync), every upscaler (DLSS and the NVAPI it needs, FSR 4,
//! XeSS, Wine's FSR) next to gamescope's, and ReShade.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use serde_json::json;

use super::gpu::GpuPart;
use super::logic::{self, RunKind};
use super::parts::{column, rows, Rows};
use super::SettingsPanel;

impl SettingsPanel {
    pub(super) fn graphics_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let mut col = column();
        let steam = self.is_steam();
        if steam {
            col = col.child(self.steam_banner(cx));
            if !self.steam.as_ref().is_some_and(|s| s.editable) {
                return col;
            }
        }
        let off = steam && !self.steam_fields_editable();
        let reshade_only = self.facts().is_some_and(|f| logic::reshade_only(f.run_kind));
        if !reshade_only || self.windows_preference() {
            col = col.children(self.gpu_card(rows(), GpuPart::Card, off, cx).titled(tr!("Graphics card")));
        }
        let mut picture = rows();
        if self.through_wine() {
            picture = picture.child(self.hdr_row(off, cx));
        }
        col = col.children(self.gpu_card(picture, GpuPart::Picture, off, cx).titled(tr!("Picture")));
        col = col.children(self.upscaling_rows(off, cx).titled(tr!("Upscaling")));
        // Linux's gamescope; a store client is not a game to scale.
        if !reshade_only && !self.is_client() {
            col = col.child(self.gamescope_section(rows(), off, cx));
        }
        let reshade = match self.facts() {
            None => false,
            Some(_) if self.is_client() => false,
            Some(f) if logic::reshade_only(f.run_kind) || f.run_kind == RunKind::Steam => self.reshade_applies(),
            // Linux games get a line about vkBasalt instead.
            Some(_) => self.reshade_applies() || (self.native() && cfg!(not(windows))),
        };
        if reshade {
            col = col.child(self.reshade_section(cx));
        }
        col
    }

    /// Windows' own GPU preference applies (a game Heronite starts on Windows).
    fn windows_preference(&self) -> bool {
        logic::windows_gpu() && self.facts().is_some_and(|f| f.run_kind == RunKind::Native)
    }

    /// A store client's settings ([`crate::dialogs::client_settings`]): a
    /// program, not a game.
    pub(super) fn is_client(&self) -> bool {
        self.facts().is_some_and(|f| f.game.is_none())
    }

    /// The game runs through Wine or Proton (Steam's too; the defaults are
    /// for such games), where HDR and the NVAPI apply.
    pub(super) fn through_wine(&self) -> bool {
        !self.native() && !self.steam_direct()
    }

    /// HDR needs Wine's Wayland driver (Compatibility): off while it is.
    fn hdr_row(&self, off: bool, cx: &mut Context<Self>) -> AnyElement {
        let wayland = self.flag("enableWineWayland", false);
        let on = self.flag("enableHDR", false);
        let desc = if wayland { tr!("The screen and the game must support it") } else { tr!("Needs the Wayland driver (Compatibility)") };
        let id = if self.is_steam() { "p-hdr" } else { "hdr" };
        self.toggle_row(id, "enableHDR", false, "HDR", Some(desc), off || (!wayland && !on), cx)
    }

    /// The NVAPI DLSS needs, the GPU's upscalers, then Wine's FSR.
    fn upscaling_rows(&self, off: bool, cx: &mut Context<Self>) -> Rows {
        let mut r = rows();
        if self.is_steam() {
            r = r.child(self.toggle_row(
                "p-nvapi",
                heronite_steam::options::KEY_NVAPI,
                false,
                "NVAPI",
                Some(tr!("DLSS and other NVIDIA features")),
                off,
                cx,
            ));
        } else if self.through_wine() && self.wine_version().kind != "crossover" {
            let dxvk_on = self.flag("autoInstallDxvk", false);
            r = r.child(self.toggle_row(
                "auto-dxvk-nvapi",
                "autoInstallDxvkNvapi",
                false,
                "DXVK-NVAPI",
                Some(tr!("DLSS and other NVIDIA features (needs DXVK)")),
                !dxvk_on || self.tool_toggle.is_some(),
                cx,
            ));
        }
        r = self.gpu_card(r, GpuPart::Upscalers, off, cx);
        if !self.is_steam() && self.through_wine() && !self.is_client() {
            r = self.fsr_rows(r, cx);
        }
        r
    }

    /// Wine's own FSR upscaling in fullscreen, with its sharpness.
    pub(super) fn fsr_rows(&self, mut r: Rows, cx: &mut Context<Self>) -> Rows {
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
}
