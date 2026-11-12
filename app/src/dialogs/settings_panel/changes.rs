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
use crate::ui::controls::{action_button, ButtonKind};
use heronite_i18n::{n_, tr};

/// One setting as the user sees it: what it is saved as, its group, and the
/// label of its row.
pub(crate) struct Group {
    /// The row, input or setting key that shows it.
    pub id: &'static str,
    pub section: Section,
    pub keys: &'static [&'static str],
    /// Its row's label (English; `tr` when shown): the Setup card names
    /// the changed settings with it.
    pub label: &'static str,
    /// Its row gets the reset icon when changed. Plain settings (a switch
    /// anyone can flip back, a text box to empty) don't, so the tabs don't
    /// fill up with icons; "Reset this tab" still covers them.
    pub button: bool,
    /// Rarely used: behind "Show N rarely used settings" while the game
    /// leaves it as the defaults have it (a changed one always shows).
    pub rare: bool,
}

/// A plain setting: no reset icon on its row.
const fn g(id: &'static str, section: Section, keys: &'static [&'static str], label: &'static str) -> Group {
    Group { id, section, keys, label, button: false, rare: false }
}

/// A technical setting (or several keys behind one row): reset icon.
const fn t(id: &'static str, section: Section, keys: &'static [&'static str], label: &'static str) -> Group {
    Group { id, section, keys, label, button: true, rare: false }
}

impl Group {
    const fn rare(self) -> Group {
        Group { rare: true, ..self }
    }
}

use Section::{Compatibility as C, Graphics as G, Performance as P, Saves as S, Start as L};

/// Every setting the game groups show, group by group. The prefix and the
/// saves folder are not here: every game has its own.
pub(crate) const GROUPS: &[Group] = &[
    // Start.
    g("launch-option", L, &[heronite_launch::launch_option::KEY], n_!("Start with")),
    g("launcherArgs", L, &["launcherArgs"], n_!("Game arguments")),
    g("offlineMode", L, &["offlineMode"], n_!("Play offline")),
    g("ignoreGameUpdates", L, &["ignoreGameUpdates"], n_!("Ignore updates")),
    g("enviromentOptions", L, &["enviromentOptions"], n_!("Environment variables")),
    g("language", L, &["language"], n_!("Game language")).rare(),
    g("beforeScript", L, &["beforeLaunchScriptPath"], n_!("Before the game starts")).rare(),
    g("afterScript", L, &["afterLaunchScriptPath"], n_!("After the game exits")).rare(),
    g("companions", L, &[heronite_launch::companions::KEY], n_!("Programs alongside the game")).rare(),
    g("targetExe", L, &["targetExe"], n_!("Alternative executable")).rare(),
    g("verboseLogs", L, &["verboseLogs"], n_!("Detailed logs")).rare(),
    t("steam-runtime", L, &["steamRuntime"], n_!("Steam Runtime")).rare(),
    // No keys: never changed, only folded.
    g("presets", L, &[], n_!("Presets")).rare(),
    g("wrapperOptions", L, &["wrapperOptions"], n_!("Wrappers")).rare(),
    // Compatibility.
    t("wine-version", C, &["wineVersion"], n_!("Wine / Proton")),
    g("crossoverBottle", C, &["wineCrossoverBottle"], n_!("CrossOver bottle")),
    g("umuId", C, &[heronite_launch::umu_ids::KEY], n_!("Game fixes")).rare(),
    t("autoInstallDxvk", C, &["autoInstallDxvk"], "DXVK"),
    t("autoInstallVkd3d", C, &["autoInstallVkd3d"], "VKD3D"),
    t("preferSystemLibs", C, &["preferSystemLibs"], n_!("Prefer system libraries")).rare(),
    t(heronite_steam::options::KEY_WINED3D, C, &[heronite_steam::options::KEY_WINED3D], "WineD3D").rare(),
    t("sync", C, &["enableEsync", "enableFsync"], n_!("Sync")).rare(),
    t("enableWineWayland", C, &["enableWineWayland"], n_!("Wayland driver")).rare(),
    t("enableWoW64", C, &["enableWoW64"], "WoW64").rare(),
    t(heronite_launch::env::KEY_PREFER_SDL, C, &[heronite_launch::env::KEY_PREFER_SDL], n_!("Read controllers through SDL")),
    t("disableUMU", C, &["disableUMU"], n_!("Disable umu")).rare(),
    g("battlEyeRuntime", C, &["battlEyeRuntime"], "BattlEye"),
    g("eacRuntime", C, &["eacRuntime"], "Easy Anti-Cheat"),
    g("sandbox", C, &[heronite_launch::sandbox::KEY], n_!("Sandbox")),
    // Graphics.
    t("gpu-device", G, &[gpu::KEY_DEVICE, gpu::KEY_PRIME], n_!("Draws the game")),
    t("gpu-preference", G, &[gpu::KEY_PREFERENCE], n_!("Draws the game")),
    t("gpu-vendor", G, &[gpu::KEY_VENDOR], n_!("Card maker the game sees")).rare(),
    t("gpu-vram", G, &[gpu::KEY_VRAM], n_!("Video memory the game sees")).rare(),
    g("enableHDR", G, &["enableHDR"], "HDR"),
    t("gpu-vsync", G, &[gpu::KEY_VSYNC], n_!("VSync")),
    t("autoInstallDxvkNvapi", G, &["autoInstallDxvkNvapi"], "DXVK-NVAPI"),
    t(heronite_steam::options::KEY_NVAPI, G, &[heronite_steam::options::KEY_NVAPI], "NVAPI"),
    t(gpu::KEY_DLSS_UPGRADE, G, &[gpu::KEY_DLSS_UPGRADE], n_!("Newest DLSS")),
    t("gpu-dlss-preset", G, &[gpu::KEY_DLSS_PRESET], n_!("DLSS model")),
    g(gpu::KEY_DLSS_INDICATOR, G, &[gpu::KEY_DLSS_INDICATOR], n_!("Show the DLSS indicator")),
    t(gpu::KEY_FSR4_UPGRADE, G, &[gpu::KEY_FSR4_UPGRADE], n_!("FSR 4")),
    g(gpu::KEY_FSR4_INDICATOR, G, &[gpu::KEY_FSR4_INDICATOR], n_!("Show the FSR indicator")),
    t(gpu::KEY_XESS_UPGRADE, G, &[gpu::KEY_XESS_UPGRADE], n_!("Newest XeSS")),
    g("enableFSR", G, &["enableFSR"], n_!("Wine FSR")),
    t("fsr-sharpness", G, &["maxSharpness"], n_!("FSR sharpness")),
    t("gamescope", G, &["gamescope"], "gamescope"),
    // Performance.
    g("showFps", P, &["showFps"], n_!("FPS counter")),
    g("showMangohud", P, &["showMangohud"], "MangoHud"),
    t("fps", P, &[fps_limit::KEY_LIMIT, fps_limit::KEY_METHOD, fps_limit::KEY_SMOOTH, fps_limit::KEY_NO_FOCUS], n_!("FPS limit")),
    t("lsfg", P, &lsfg::INHERITED_KEYS, n_!("Lossless Scaling frame generation")),
    g(gpu::KEY_SMOOTH_MOTION, P, &[gpu::KEY_SMOOTH_MOTION], n_!("Smooth Motion")),
    g(gpu::KEY_REFLEX, P, &[gpu::KEY_REFLEX], n_!("Reflex in Vulkan games")),
    g(gpu::KEY_ANTI_LAG, P, &[gpu::KEY_ANTI_LAG], n_!("Anti-Lag")),
    g("useGameMode", P, &["useGameMode"], "GameMode"),
    g(heronite_launch::keep_awake::KEY, P, &[heronite_launch::keep_awake::KEY], n_!("Keep the computer awake")).rare(),
    t(gpu::KEY_SHADER_CACHE, P, &[gpu::KEY_SHADER_CACHE], n_!("Bigger shader cache")).rare(),
    t(gpu::KEY_GLTHREAD, P, &[gpu::KEY_GLTHREAD], n_!("Threaded OpenGL")).rare(),
    // Saves.
    g("autoSyncSaves", S, &["autoSyncSaves"], n_!("Sync automatically")),
    g("enableQuickSavesMenu", S, &["enableQuickSavesMenu"], n_!("Quick sync menu")),
];

pub(crate) fn group(id: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.id == id)
}

/// Whether `group` differs between a game's settings and the defaults it
/// would otherwise get.
pub(crate) fn differs(group: &Group, settings: &Map<String, Value>, base: &Map<String, Value>) -> bool {
    print(group, settings) != print(group, base)
}

/// The settings of `section` that differ from the defaults, in the order of
/// [`GROUPS`] (the most asked for first).
pub(crate) fn changed_groups(section: Section, settings: &Map<String, Value>, base: &Map<String, Value>) -> Vec<&'static Group> {
    GROUPS.iter().filter(|g| g.section == section && differs(g, settings, base)).collect()
}

/// Whether a row shows: `rare` ones only while "Show rarely used" is open
/// or the game changed them.
pub(crate) fn shown(rare: bool, changed: bool, show_rare: bool) -> bool {
    !rare || changed || show_rare
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
        self.inherited.as_ref().is_some_and(|base| differs(group, &self.settings, base))
    }

    /// Whether the row of `id` shows (see [`shown`]). A rarely used one the
    /// game left alone is counted for "Show N rarely used settings".
    pub(super) fn shows(&self, id: &str) -> bool {
        let Some(g) = group(id) else { return true };
        let changed = self.changed(g);
        if g.rare && !changed {
            let mut folded = self.folded.borrow_mut();
            if !folded.contains(&g.id) {
                folded.push(g.id);
            }
        }
        shown(g.rare, changed, self.show_rare)
    }

    /// "Show N rarely used settings" (or "Hide rarely used settings") under
    /// the group, when it has any.
    pub(super) fn rare_link(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let n = self.folded.borrow().len();
        let text = match (self.show_rare, n) {
            (_, 0) => return None,
            (false, n) => heronite_i18n::trn!(n, "Show {n} rarely used setting", "Show {n} rarely used settings"),
            (true, _) => tr!("Hide rarely used settings").to_string(),
        };
        Some(
            div()
                .id("rare-link")
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(6.))
                .py(px(4.))
                .text_size(px(13.))
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|s| s.text_color(theme::accent_overlay()))
                .child(text)
                .when(!self.show_rare, |d| d.child(crate::ui::icon(Icon::ExpandMore, px(16.), theme::accent())))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_rare = !this.show_rare;
                    cx.notify();
                })),
        )
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
        self.confirm_reset = false;
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

    /// Whether "Reset this tab" is waiting for its answer.
    pub fn asking_reset(&self) -> bool {
        self.confirm_reset
    }

    /// Starts the question before "Reset this tab".
    pub fn ask_reset(&mut self, cx: &mut Context<Self>) {
        self.confirm_reset = true;
        cx.notify();
    }

    /// Drops the question; true when there was one (Esc, Cancel, B).
    pub fn cancel_reset(&mut self, cx: &mut Context<Self>) -> bool {
        let asking = std::mem::take(&mut self.confirm_reset);
        if asking {
            cx.notify();
        }
        asking
    }

    /// The footer row that replaces "Reset this tab" while it is being
    /// asked: what will happen, Cancel and the danger button.
    pub fn reset_question(panel: &Entity<Self>, id: &'static str) -> Div {
        let (no, yes) = (panel.clone(), panel.clone());
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(tr!("This tab's changed settings go back to the defaults.")),
            )
            .child(action_button(SharedString::from(format!("{id}-no")), ButtonKind::Ghost, tr!("Cancel"), true, move |_, _, cx| {
                no.update(cx, |p, cx| {
                    p.cancel_reset(cx);
                })
            }))
            .child(action_button(
                SharedString::from(format!("{id}-yes")),
                ButtonKind::Danger,
                tr!("Reset this tab"),
                true,
                move |_, _, cx| yes.update(cx, |p, cx| p.reset_tab(cx)),
            ))
    }

    /// Sets every changed setting of the tab shown back to the defaults.
    pub fn reset_tab(&mut self, cx: &mut Context<Self>) {
        let groups = self.changed_in(self.section);
        self.reset(&groups, cx);
    }

    /// Console: the group of a row id when it changed.
    pub(super) fn console_changed(&self, row_id: &str) -> Option<&'static Group> {
        console_group(row_id).filter(|g| self.changed(g))
    }

    /// Console: the rows without the rarely used settings the game left
    /// alone (unless they are shown), and the row that shows or hides them.
    /// Headers left without rows go too.
    pub(super) fn console_fold(&self, items: Vec<super::console::Item>) -> Vec<super::console::Item> {
        let mut folded = 0;
        let mut kept: Vec<super::console::Item> = Vec::new();
        for item in items {
            let rare = console_group(&item.row.id).filter(|g| g.rare && !self.changed(g));
            if rare.is_some() {
                folded += 1;
                if !self.show_rare {
                    continue;
                }
            }
            kept.push(item);
        }
        let mut kept = drop_empty_headers(kept, |i| matches!(i.row.value, crate::console::list::Value::Header));
        if folded > 0 {
            let label = if self.show_rare {
                tr!("Hide rarely used settings").to_string()
            } else {
                heronite_i18n::trn!(folded, "Show {n} rarely used setting", "Show {n} rarely used settings")
            };
            kept.push(Self::c_do("rare", &label, Icon::Tune, true, |this, _, cx| {
                this.show_rare = !this.show_rare;
                cx.notify();
            }));
        }
        kept
    }

    /// Console: the tab's changed groups, for "Reset this tab".
    pub(super) fn console_tab_changes(&self) -> Vec<&'static Group> {
        self.changed_in(self.section)
    }
}

/// The group of a console row id (`t:<key>`, `c:<id>`...).
fn console_group(row_id: &str) -> Option<&'static Group> {
    let id = row_id.split_once(':').map_or(row_id, |(_, rest)| rest);
    let id = match id {
        "wine" => "wine-version",
        "fps-limit" => "fps",
        "sharp" => "fsr-sharpness",
        "runtime" => "steam-runtime",
        "enableUpscaling" if row_id.starts_with("g:") => "gamescope",
        "beforeLaunchScriptPath" => "beforeScript",
        "afterLaunchScriptPath" => "afterScript",
        other => other,
    };
    // A group of several keys is shown by the row of its first one.
    group(id).or_else(|| GROUPS.iter().find(|g| g.keys.len() > 1 && g.keys[0] == id))
}

/// `items` without the headers that have no row before the next header (or
/// the end).
pub(crate) fn drop_empty_headers<T>(items: Vec<T>, is_header: impl Fn(&T) -> bool) -> Vec<T> {
    let heads: Vec<bool> = items.iter().map(&is_header).collect();
    items.into_iter().enumerate().filter(|(i, _)| !heads[*i] || heads.get(i + 1).is_some_and(|next| !next)).map(|(_, it)| it).collect()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has its own `test` attribute.
    use super::{changed_groups, console_group, differs, drop_empty_headers, group, norm, print, shown, GROUPS};
    use crate::dialogs::settings_panel::logic::Section;
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

    /// Where the settings the old tabs split went (Discussion #290).
    #[test]
    fn settings_sit_in_their_task_group() {
        let at = |id: &str| group(id).unwrap_or_else(|| panic!("{id}")).section;
        // Frame generation together, in Performance.
        assert_eq!(at("lsfg"), Section::Performance);
        assert_eq!(at(heronite_launch::gpu::KEY_SMOOTH_MOTION), Section::Performance);
        // Every upscaler in Graphics, with what DLSS needs.
        for id in [
            heronite_launch::gpu::KEY_DLSS_UPGRADE,
            heronite_launch::gpu::KEY_FSR4_UPGRADE,
            heronite_launch::gpu::KEY_XESS_UPGRADE,
            "enableFSR",
            "fsr-sharpness",
            "gamescope",
            "autoInstallDxvkNvapi",
            heronite_steam::options::KEY_NVAPI,
        ] {
            assert_eq!(at(id), Section::Graphics, "{id}");
        }
        assert_eq!(at("enableHDR"), Section::Graphics);
        assert_eq!(at("gpu-vsync"), Section::Graphics);
        // Latency next to the FPS limit.
        assert_eq!(at(heronite_launch::gpu::KEY_REFLEX), Section::Performance);
        assert_eq!(at(heronite_launch::gpu::KEY_ANTI_LAG), Section::Performance);
        // Proton fixes in Compatibility; the sandbox is a block of it.
        assert_eq!(at(heronite_launch::env::KEY_PREFER_SDL), Section::Compatibility);
        assert_eq!(at("disableUMU"), Section::Compatibility);
        assert_eq!(at("sandbox"), Section::Compatibility);
        assert_eq!(at("launcherArgs"), Section::Start);
        assert!(GROUPS.iter().all(|g| g.section != Section::General));
    }

    /// The most used settings never fold; a changed one always shows.
    #[test]
    fn rarely_used_fold_but_never_when_changed() {
        for id in ["wine-version", "launcherArgs", "fps", "showMangohud", "enableHDR", "gpu-device", "lsfg", "autoSyncSaves"] {
            assert!(!group(id).unwrap().rare, "{id}");
        }
        assert!(group("gpu-vram").unwrap().rare);
        assert!(!shown(true, false, false));
        assert!(shown(true, true, false), "changed: never hidden");
        assert!(shown(true, false, true), "shown on demand");
        assert!(shown(false, false, false));
    }

    #[test]
    fn changed_lists_a_groups_differences_in_order() {
        let base = map(json!({"enableHDR": false, "showMangohud": false}));
        let game = map(json!({"enableHDR": true, "showMangohud": true, "heroniteGpuVsync": "off", "enableFSR": true}));
        let graphics: Vec<&str> = changed_groups(Section::Graphics, &game, &base).iter().map(|g| g.id).collect();
        assert_eq!(graphics.first(), Some(&"enableHDR"));
        assert!(graphics.contains(&"enableFSR"));
        assert!(!graphics.contains(&"showMangohud"), "Performance's");
        assert!(differs(group("showMangohud").unwrap(), &game, &base));
        assert!(changed_groups(Section::Saves, &game, &base).is_empty());
        // Presets have no keys: never "changed".
        assert!(!differs(group("presets").unwrap(), &game, &base));
    }

    #[test]
    fn console_rows_find_their_setting() {
        assert_eq!(console_group("c:wine").map(|g| g.id), Some("wine-version"));
        assert_eq!(console_group("t:enableHDR").map(|g| g.id), Some("enableHDR"));
        assert_eq!(console_group("p:beforeScript").map(|g| g.id), Some("beforeScript"));
        assert_eq!(console_group("t:enableEsync").map(|g| g.id), Some("sync"));
        assert!(console_group("h:Sync").is_none());
    }

    #[test]
    fn headers_without_rows_go() {
        // `true`: a header.
        let items = vec![(true, 'a'), (false, 'b'), (true, 'c'), (true, 'd'), (false, 'e'), (true, 'f')];
        let kept: Vec<char> = drop_empty_headers(items, |i| i.0).into_iter().map(|i| i.1).collect();
        assert_eq!(kept, ['a', 'b', 'd', 'e']);
    }
}
