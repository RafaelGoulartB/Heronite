//! What a game changed from the defaults it would otherwise get: a quiet
//! reset icon on the right of a changed row (before its switch, or at the end
//! of a field's label) for the technical settings only ([`Group::button`]:
//! the ones whose default a player would not know), and
//! "Reset this tab" next to Done (the game settings dialog draws it).
//!
//! "The defaults" are what a new game gets from Settings › Game defaults
//! (`game_config::default_game_settings`); a Steam game's are an empty
//! launch options line. Every setting the tabs show is one [`Group`] of keys;
//! rows find theirs by id (the row's, the input's, or the setting key).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::{fps_limit, gpu, lsfg};
use serde_json::{Map, Value};

use super::logic::Section;
use super::SettingsPanel;
use crate::icons::Icon;
use crate::theme;

/// One setting as the user sees it: what it is saved as, and its tab.
pub(super) struct Group {
    /// The row, input or setting key that shows it.
    pub id: &'static str,
    pub section: Section,
    pub keys: &'static [&'static str],
    /// Its row gets the reset icon when changed. Plain settings (a switch
    /// anyone can flip back, a text box to empty) don't, so the tabs don't
    /// fill up with icons; "Reset this tab" still covers them.
    pub button: bool,
}

/// A plain setting: no reset icon on its row.
const fn g(id: &'static str, section: Section, keys: &'static [&'static str]) -> Group {
    Group { id, section, keys, button: false }
}

/// A technical setting (or several keys behind one row): reset icon.
const fn t(id: &'static str, section: Section, keys: &'static [&'static str]) -> Group {
    Group { id, section, keys, button: true }
}

use Section::{Compatibility as C, Gpu as G, Launch as L, Performance as P, Sandbox as X, Saves as S};

/// Every setting the game tabs show. The prefix and the saves folder are not
/// here: every game has its own.
const GROUPS: &[Group] = &[
    t("wine-version", C, &["wineVersion"]),
    g("crossoverBottle", C, &["wineCrossoverBottle"]),
    g("umuId", C, &[heronite_launch::umu_ids::KEY]),
    t("autoInstallDxvk", C, &["autoInstallDxvk"]),
    t("autoInstallVkd3d", C, &["autoInstallVkd3d"]),
    t("autoInstallDxvkNvapi", C, &["autoInstallDxvkNvapi"]),
    t("preferSystemLibs", C, &["preferSystemLibs"]),
    t("sync", C, &["enableEsync", "enableFsync"]),
    t("enableWineWayland", C, &["enableWineWayland"]),
    g("enableHDR", C, &["enableHDR"]),
    t("enableWoW64", C, &["enableWoW64"]),
    g("battlEyeRuntime", C, &["battlEyeRuntime"]),
    g("eacRuntime", C, &["eacRuntime"]),
    t(heronite_steam::options::KEY_NVAPI, C, &[heronite_steam::options::KEY_NVAPI]),
    t(heronite_steam::options::KEY_WINED3D, C, &[heronite_steam::options::KEY_WINED3D]),
    g("showFps", P, &["showFps"]),
    g("showMangohud", P, &["showMangohud"]),
    t("fps", P, &[fps_limit::KEY_LIMIT, fps_limit::KEY_METHOD, fps_limit::KEY_SMOOTH, fps_limit::KEY_NO_FOCUS]),
    t("lsfg", P, &lsfg::INHERITED_KEYS),
    g("enableFSR", P, &["enableFSR"]),
    t("fsr-sharpness", P, &["maxSharpness"]),
    t("gamescope", P, &["gamescope"]),
    g("useGameMode", P, &["useGameMode"]),
    g(heronite_launch::keep_awake::KEY, P, &[heronite_launch::keep_awake::KEY]),
    t(heronite_launch::env::KEY_PREFER_SDL, P, &[heronite_launch::env::KEY_PREFER_SDL]),
    t("gpu-device", G, &[gpu::KEY_DEVICE, gpu::KEY_PRIME]),
    g(gpu::KEY_SMOOTH_MOTION, G, &[gpu::KEY_SMOOTH_MOTION]),
    g(gpu::KEY_REFLEX, G, &[gpu::KEY_REFLEX]),
    t(gpu::KEY_SHADER_CACHE, G, &[gpu::KEY_SHADER_CACHE]),
    t(gpu::KEY_DLSS_UPGRADE, G, &[gpu::KEY_DLSS_UPGRADE]),
    t("gpu-dlss-preset", G, &[gpu::KEY_DLSS_PRESET]),
    g(gpu::KEY_DLSS_INDICATOR, G, &[gpu::KEY_DLSS_INDICATOR]),
    t(gpu::KEY_FSR4_UPGRADE, G, &[gpu::KEY_FSR4_UPGRADE]),
    g(gpu::KEY_FSR4_INDICATOR, G, &[gpu::KEY_FSR4_INDICATOR]),
    t(gpu::KEY_XESS_UPGRADE, G, &[gpu::KEY_XESS_UPGRADE]),
    g(gpu::KEY_ANTI_LAG, G, &[gpu::KEY_ANTI_LAG]),
    t(gpu::KEY_GLTHREAD, G, &[gpu::KEY_GLTHREAD]),
    t("gpu-vsync", G, &[gpu::KEY_VSYNC]),
    t("gpu-vendor", G, &[gpu::KEY_VENDOR]),
    t("gpu-vram", G, &[gpu::KEY_VRAM]),
    g("launch-option", L, &[heronite_launch::launch_option::KEY]),
    g("launcherArgs", L, &["launcherArgs"]),
    g("offlineMode", L, &["offlineMode"]),
    g("ignoreGameUpdates", L, &["ignoreGameUpdates"]),
    g("language", L, &["language"]),
    g("beforeScript", L, &["beforeLaunchScriptPath"]),
    g("afterScript", L, &["afterLaunchScriptPath"]),
    g("companions", L, &[heronite_launch::companions::KEY]),
    g("targetExe", L, &["targetExe"]),
    g("verboseLogs", L, &["verboseLogs"]),
    t("steam-runtime", L, &["steamRuntime"]),
    t("disableUMU", L, &["disableUMU"]),
    g("wrapperOptions", L, &["wrapperOptions"]),
    g("enviromentOptions", L, &["enviromentOptions"]),
    g("sandbox", X, &[heronite_launch::sandbox::KEY]),
    g("autoSyncSaves", S, &["autoSyncSaves"]),
    g("enableQuickSavesMenu", S, &["enableQuickSavesMenu"]),
];

pub(super) fn group(id: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.id == id)
}

/// The value a key has when nothing sets it, where that is not "off".
fn implicit(key: &str) -> Value {
    match key {
        heronite_launch::keep_awake::KEY | "verboseLogs" => Value::Bool(true),
        fps_limit::KEY_METHOD => Value::String(fps_limit::Method::Auto.id().into()),
        _ => Value::Null,
    }
}

/// A value as compared: missing, `false`, `""`, `0`, `[]` and `{}` are all
/// "off"; numbers and numeric text are the same.
fn norm(key: &str, v: Option<&Value>) -> Value {
    let v = v.filter(|v| !v.is_null()).cloned().unwrap_or_else(|| implicit(key));
    match v {
        Value::Bool(false) => Value::Null,
        Value::String(s) if s.trim().is_empty() => Value::Null,
        Value::String(s) => Value::String(s.trim().to_string()),
        Value::Number(n) if n.as_f64() == Some(0.0) => Value::Null,
        Value::Number(n) => Value::String(n.to_string()),
        Value::Array(a) if a.is_empty() => Value::Null,
        Value::Object(o) if o.is_empty() => Value::Null,
        v => v,
    }
}

/// What a group means for a settings map: equal prints, same setting.
fn print(group: &Group, m: &Map<String, Value>) -> String {
    match group.id {
        // The FPS limit also reads Heroic's old limiters.
        "fps" => format!("{:?}", fps_limit::FpsLimit::from_map(m)),
        "lsfg" => format!("{:?}", lsfg::Lsfg::from_map(m)),
        "gpu-device" => {
            let g = gpu::Gpu::from_map(m);
            format!("{}|{}", g.device, g.dedicated && g.device.is_empty())
        }
        "gamescope" => {
            let mut o = gamescope_defaults();
            if let Some(Value::Object(saved)) = m.get("gamescope") {
                o.extend(saved.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            // The limiter fields belong to the FPS limit.
            for f in fps_limit::LEGACY_GAMESCOPE {
                o.remove(f);
            }
            let fields: Vec<String> = o.iter().map(|(k, v)| format!("{k}={}", norm(k, Some(v)))).collect();
            fields.join(";")
        }
        _ => group.keys.iter().map(|k| norm(k, m.get(*k)).to_string()).collect::<Vec<_>>().join("|"),
    }
}

/// Heroic's gamescope object when nothing is set (see `SettingsPanel::gamescope`).
fn gamescope_defaults() -> Map<String, Value> {
    match serde_json::json!({
        "enableUpscaling": false, "enableLimiter": false, "enableForceGrabCursor": false, "windowType": "fullscreen",
        "gameWidth": "", "gameHeight": "", "upscaleHeight": "", "upscaleWidth": "", "upscaleMethod": "fsr",
        "fpsLimiter": "", "fpsLimiterNoFocus": "", "additionalOptions": ""
    }) {
        Value::Object(o) => o,
        _ => unreachable!(),
    }
}

/// `v` emptied: what "off" is for its type.
fn off_like(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Bool(_)) => Value::Bool(false),
        Some(Value::String(_)) => Value::String(String::new()),
        Some(Value::Number(_)) => Value::from(0),
        Some(Value::Array(_)) => Value::Array(Vec::new()),
        Some(Value::Object(_)) => Value::Object(Map::new()),
        _ => Value::Null,
    }
}

impl SettingsPanel {
    /// Whether `group` differs from the defaults (never for the defaults
    /// themselves).
    pub(super) fn changed(&self, group: &Group) -> bool {
        self.inherited.as_ref().is_some_and(|base| print(group, &self.settings) != print(group, base))
    }

    /// The changed groups of a section.
    fn changed_in(&self, section: Section) -> Vec<&'static Group> {
        GROUPS.iter().filter(|g| g.section == section && self.changed(g)).collect()
    }

    /// The values that bring the groups back to the defaults.
    fn reset_values(&self, groups: &[&Group]) -> Vec<(&'static str, Value)> {
        let Some(base) = self.inherited.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for g in groups {
            for key in g.keys {
                let value = match base.get(*key).filter(|v| !v.is_null()) {
                    Some(v) => v.clone(),
                    None => match implicit(key) {
                        Value::Null => off_like(self.settings.get(*key)),
                        v => v,
                    },
                };
                if self.settings.get(*key) != Some(&value) {
                    out.push((*key, value));
                }
            }
        }
        out
    }

    /// Sets the groups back to the defaults. DXVK and its kind are installed
    /// or removed like their switches do.
    pub(super) fn reset(&mut self, groups: &[&Group], cx: &mut Context<Self>) {
        let mut plain = Vec::new();
        for (key, value) in self.reset_values(groups) {
            if super::wine::prefix_tool(key).is_some() {
                if value.as_bool().unwrap_or(false) != self.flag(key, false) {
                    self.toggle(key, false, cx);
                }
            } else {
                plain.push((key, value));
            }
        }
        if !plain.is_empty() {
            self.set_all(plain, cx);
        }
        self.notice = None;
        cx.notify();
    }

    /// A field (label above its control) with the reset button at the right
    /// end of the label line when its group differs from the defaults.
    pub(super) fn marked(&self, id: &str, el: impl IntoElement, cx: &mut Context<Self>) -> Div {
        div().relative().child(el).children(self.reset_button(id, cx).map(|b| b.absolute().top(px(-3.)).right(px(0.))))
    }

    /// A caption whose card is one setting (lists, the FPS limit, gamescope),
    /// with the reset button after the text when it changed.
    pub(super) fn marked_heading(&self, text: &str, id: &str, cx: &mut Context<Self>) -> Div {
        let caption = super::parts::heading(text);
        match self.reset_button(id, cx) {
            Some(b) => caption.flex().items_center().gap(px(6.)).child(b),
            None => caption,
        }
    }

    /// Bare ↺ (accent under the pointer) that sets the group of `id` back to
    /// the defaults; `None` while it has not changed or is a plain setting.
    pub(super) fn reset_button(&self, id: &str, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let g = group(id).filter(|g| g.button && self.changed(g))?;
        let hover = SharedString::from(format!("reset-{}", g.id));
        Some(
            div()
                .id(hover.clone())
                .group(hover.clone())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded_full()
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.06)))
                .child(crate::ui::icon(Icon::Replay, px(15.), theme::text_dim()).group_hover(hover, |s| s.text_color(theme::accent())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.reset(&[g], cx);
                })),
        )
    }

    /// Whether the tab shown has settings that differ from the defaults.
    pub fn tab_changed(&self) -> bool {
        !self.changed_in(self.section).is_empty()
    }

    /// Sets every changed setting of the tab shown back to the defaults.
    pub fn reset_tab(&mut self, cx: &mut Context<Self>) {
        let groups = self.changed_in(self.section);
        self.reset(&groups, cx);
    }

    /// Console: the group of a row id (`t:<key>`, `c:<id>`...) when it changed.
    pub(super) fn console_changed(&self, row_id: &str) -> Option<&'static Group> {
        let id = row_id.split_once(':').map_or(row_id, |(_, rest)| rest);
        let id = match id {
            "wine" => "wine-version",
            "fps-limit" => "fps",
            "sharp" => "fsr-sharpness",
            "runtime" => "steam-runtime",
            "enableUpscaling" if row_id.starts_with("g:") => "gamescope",
            other => other,
        };
        // A group of several keys is shown by the row of its first one.
        let found = group(id).or_else(|| GROUPS.iter().find(|g| g.keys.len() > 1 && g.keys[0] == id));
        found.filter(|g| self.changed(g))
    }

    /// Console: the tab's changed groups, for "Reset this tab".
    pub(super) fn console_tab_changes(&self) -> Vec<&'static Group> {
        self.changed_in(self.section)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute.
    use super::{group, norm, print, GROUPS};
    use serde_json::{json, Map, Value};

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn off_values_and_numbers_compare_equal() {
        assert_eq!(norm("x", None), norm("x", Some(&json!(false))));
        assert_eq!(norm("x", Some(&json!(""))), norm("x", Some(&json!([]))));
        assert_eq!(norm("x", Some(&json!(60))), norm("x", Some(&json!("60"))));
        assert_ne!(norm(heronite_launch::keep_awake::KEY, None), norm(heronite_launch::keep_awake::KEY, Some(&json!(false))));
    }

    #[test]
    fn groups_compare_by_meaning() {
        let fps = group("fps").unwrap();
        // Heroic's DXVK limit and the same limit in our keys are one setting.
        let heroic = map(json!({"enableDXVKFpsLimit": true, "DXVKFpsCap": "60"}));
        let ours = map(json!({"heroniteFpsLimit": "60", "heroniteFpsLimitMethod": "dxvk"}));
        assert_eq!(print(fps, &heroic), print(fps, &ours));
        let gs = group("gamescope").unwrap();
        let empty = map(json!({}));
        let limiter_only = map(json!({"gamescope": {"windowType": "fullscreen", "fpsLimiter": "60"}}));
        let upscaling = map(json!({"gamescope": {"enableUpscaling": true}}));
        assert_eq!(print(gs, &empty), print(gs, &limiter_only));
        assert_ne!(print(gs, &empty), print(gs, &upscaling));
        let device = group("gpu-device").unwrap();
        assert_ne!(print(device, &empty), print(device, &map(json!({"nvidiaPrime": true}))));
    }

    #[test]
    fn every_group_id_is_unique() {
        for (i, a) in GROUPS.iter().enumerate() {
            assert!(GROUPS[i + 1..].iter().all(|b| b.id != a.id), "{}", a.id);
        }
    }
}
