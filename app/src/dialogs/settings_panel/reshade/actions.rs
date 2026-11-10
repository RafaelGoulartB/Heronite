//! What the buttons of the ReShade section do. Long work goes through
//! [`SettingsPanel::reshade_op`]; the game's settings keys mirror the result.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::gfxmods::catalog::Found;
use heronite_launch::gfxmods::renodx;
use heronite_launch::reshade::catalog::Package;
use heronite_launch::reshade::install::{self, Request};
use heronite_launch::reshade::{self, manage, State};
use serde_json::{json, Value};

use super::{Confirm, SettingsPanel};
use crate::ui::fields::choose_path;

impl SettingsPanel {
    /// The packages ticked for the install: the user's choice, else the
    /// catalog's defaults and what the chosen preset needs. The required
    /// package is always in.
    pub(super) fn reshade_picked(&self) -> Vec<String> {
        let Some(o) = &self.reshade.overview else { return Vec::new() };
        let mut names: Vec<String> = match &self.reshade.picked {
            Some(p) => p.clone(),
            None => o.packages.iter().filter(|p| p.enabled).map(|p| p.name.clone()).collect(),
        };
        if self.reshade.picked.is_none() {
            for n in self.reshade_preset_needs().packages {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
        for p in o.packages.iter().filter(|p| p.required) {
            if !names.contains(&p.name) {
                names.insert(0, p.name.clone());
            }
        }
        names
    }

    /// What the preset chosen before installing needs.
    pub(super) fn reshade_preset_needs(&self) -> reshade::preset::Needs {
        let (Some(o), Some((_, effects))) = (&self.reshade.overview, &self.reshade.preset) else { return Default::default() };
        reshade::preset::needs(effects, &o.packages, |_| false)
    }

    pub(super) fn toggle_reshade_package(&mut self, name: &str, cx: &mut Context<Self>) {
        let mut picked = self.reshade_picked();
        match picked.iter().position(|p| p == name) {
            Some(i) => {
                picked.remove(i);
            }
            None => picked.push(name.to_string()),
        }
        self.reshade.picked = Some(picked);
        cx.notify();
    }

    pub(super) fn pick_reshade_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(window, cx, false, tr!("Choose a ReShade preset (.ini)"), |this: &mut Self, path, _, cx| {
            this.choose_reshade_preset(PathBuf::from(path), cx)
        });
    }

    /// Before installing, the preset is remembered (and the packages it
    /// needs ticked); once installed, it is imported at once.
    pub(super) fn choose_reshade_preset(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.reshade.overview.as_ref().is_some_and(|o| o.state == State::Installed) {
            let id = self.reshade_id();
            let name = file_name(&path);
            return self.reshade_op(
                tr!("Importing {name}…", name).to_string(),
                cx,
                move |ops| manage::import_preset(ops.paths, &id, &path),
                move |this, m, cx| {
                    let name = m.preset.unwrap_or_default();
                    this.reshade.message =
                        Some((tr!("Preset {name} imported. ReShade loads it the next time the game starts.", name), false));
                    cx.notify();
                },
            );
        }
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_spawn(async move { std::fs::read_to_string(&path).map(|t| (path, reshade::preset::active_effects(&t))) })
                .await;
            this.update(cx, |this, cx| {
                match read {
                    Ok(p) => {
                        this.reshade.preset = Some(p);
                        this.reshade.picked = None;
                        // The card grows: keep it in view.
                        this.reshade.setup = true;
                        this.reshade.reveal.set(true);
                    }
                    Err(e) => this.reshade.message = Some((e.to_string(), true)),
                }
                this.next_reshade_debug_after_view(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn reshade_id(&self) -> String {
        self.facts().map(|f| f.app_name.clone()).unwrap_or_default()
    }

    /// Install (or `replace` a manual one). Asks about anti-cheats first
    /// unless they were all accepted before.
    pub(super) fn start_reshade_install(&mut self, replace: bool, check: bool, cx: &mut Context<Self>) {
        let Some(o) = self.reshade.overview.clone() else { return };
        let ack = self.reshade_settings().anticheat_ack;
        if check && !o.risk.unacknowledged(&ack).is_empty() {
            self.reshade.confirm = Some(Confirm::AntiCheat { accepted: false, replace });
            self.reshade.reveal.set(true);
            return cx.notify();
        }
        let Some(target) = o.target.clone() else { return };
        let Some(version) = o.latest.clone() else {
            self.reshade.message = Some((tr!("ReShade's versions could not be read. Check the connection and try again.").into(), true));
            return cx.notify();
        };
        let picked = self.reshade_picked();
        let packages: Vec<Package> = o.packages.iter().filter(|p| picked.contains(&p.name)).cloned().collect();
        let manual = match &o.state {
            State::Manual { proxy, .. } => Some(proxy.clone()),
            _ => None,
        };
        let Some(proxy) = manual.clone().or_else(|| o.proxy.clone()) else { return };
        let wine = self.reshade_wine() == Some(true);
        let req = Request {
            game_id: self.reshade_id(),
            compiler: wine && target.api.is_some_and(|a| a.uses_d3dcompiler()),
            target,
            proxy,
            version: version.clone(),
            packages,
            preset: self.reshade.preset.as_ref().map(|(p, _)| p.clone()),
            compat: o.compat.clone(),
        };
        let replacing = replace && manual.is_some();
        // ReShade, the compiler, each package, the settings file.
        let total = 2 + usize::from(req.compiler) + req.packages.len();
        self.reshade_op(
            tr!("Downloading ReShade {version}…", version).to_string(),
            cx,
            move |ops| {
                let previous = match replacing {
                    true => Some(manage::adopt(ops.paths, &req.game_id, &req.target.dir, &req.proxy)?),
                    false => None,
                };
                install::install(ops, &req, previous.as_ref())
            },
            move |this, m, cx| {
                this.reshade.preset = None;
                this.reshade.picked = None;
                this.reshade.all_packages = false;
                this.reshade.setup = false;
                this.reshade.advanced = false;
                this.set(reshade::KEY_ENABLED, Value::Bool(true), cx);
                this.reshade.message =
                    Some((tr!("ReShade {version} is installed. Press Home in the game to open its menu.", version = m.version), false));
            },
        );
        if let Some(b) = &mut self.reshade.busy {
            b.total = total;
        }
    }

    /// The anti-cheat question was answered with "Install anyway".
    pub(super) fn accept_anticheat(&mut self, cx: &mut Context<Self>) {
        let Some(Confirm::AntiCheat { accepted: true, replace }) = self.reshade.confirm.clone() else { return };
        let names = self.reshade.overview.as_ref().map(|o| o.risk.names()).unwrap_or_default();
        self.set(reshade::KEY_ANTICHEAT_ACK, json!(names), cx);
        self.reshade.confirm = None;
        self.start_reshade_install(replace, false, cx);
    }

    pub(super) fn adopt_reshade(&mut self, cx: &mut Context<Self>) {
        let Some(o) = self.reshade.overview.clone() else { return };
        let (Some(t), State::Manual { proxy, .. }) = (o.target.clone(), o.state.clone()) else { return };
        let id = self.reshade_id();
        self.reshade_op(
            tr!("Taking over ReShade…").to_string(),
            cx,
            move |ops| manage::adopt(ops.paths, &id, &t.dir, &proxy),
            |this, m, cx| {
                this.set(reshade::KEY_ENABLED, Value::Bool(m.enabled), cx);
                this.reshade.message = Some((tr!("Heronite now manages this ReShade. Nothing was downloaded or changed.").into(), false));
            },
        );
    }

    pub(super) fn switch_reshade(&mut self, on: bool, cx: &mut Context<Self>) {
        let id = self.reshade_id();
        self.reshade_op(
            String::new(),
            cx,
            move |ops| manage::set_enabled(ops.paths, &id, on),
            |this, m, cx| this.set(reshade::KEY_ENABLED, Value::Bool(m.enabled), cx),
        );
    }

    pub(super) fn update_reshade(&mut self, cx: &mut Context<Self>) {
        let Some(version) = self.reshade.overview.as_ref().and_then(|o| o.latest.clone()) else { return };
        let id = self.reshade_id();
        self.reshade_op(
            tr!("Downloading ReShade {version}…", version).to_string(),
            cx,
            move |ops| manage::update_dll(ops, &id, &version),
            |this, m, _| {
                this.reshade.message =
                    Some((tr!("ReShade updated to {version}. Its settings and presets stay.", version = m.version), false))
            },
        );
    }

    /// Switches to the build with add-on support (`on`) or back.
    pub(super) fn set_reshade_addon(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(risk) = self.reshade.overview.as_ref().map(|o| o.risk.clone()) else { return };
        let id = self.reshade_id();
        let line = match on {
            true => tr!("Switching ReShade to the add-on version…"),
            false => tr!("Switching ReShade to the standard version…"),
        };
        self.reshade_op(
            line.to_string(),
            cx,
            move |ops| manage::set_addon(ops, &id, on, &risk),
            |this, m, _| {
                this.reshade.message = Some((
                    match m.addon {
                        true => tr!("ReShade now uses the add-on version. Its settings and presets were kept."),
                        false => tr!("ReShade is back to the standard version. Its settings and presets were kept."),
                    }
                    .into(),
                    false,
                ))
            },
        );
    }

    /// Installs the game's RenoDX mod (`refresh`: downloads it again).
    pub(super) fn install_renodx(&mut self, found: Found, refresh: bool, cx: &mut Context<Self>) {
        let Some(risk) = self.reshade.overview.as_ref().map(|o| o.risk.clone()) else { return };
        let id = self.reshade_id();
        let name = found.name.clone();
        self.reshade_op(
            tr!("Downloading the RenoDX mod: {name}…", name = name.clone()).to_string(),
            cx,
            move |ops| renodx::install(ops, &id, &found, &risk, refresh),
            move |this, _, _| {
                this.reshade.message = Some((tr!("HDR mod installed: {name}. Adjust it in ReShade's menu (Home key).", name), false))
            },
        );
    }

    pub(super) fn remove_renodx(&mut self, cx: &mut Context<Self>) {
        let id = self.reshade_id();
        self.reshade_op(
            String::new(),
            cx,
            move |ops| renodx::remove(ops.paths, &id),
            |this, (_, kept), _| {
                this.reshade.message = Some(match kept.is_empty() {
                    true => (tr!("HDR mod removed.").into(), false),
                    false => (tr!("HDR mod removed. {files} was kept because it was changed.", files = kept.join(", ")), false),
                })
            },
        );
    }

    pub(super) fn update_reshade_shaders(&mut self, cx: &mut Context<Self>) {
        let Some(o) = self.reshade.overview.clone() else { return };
        let id = self.reshade_id();
        self.reshade_op(
            tr!("Updating shaders…").to_string(),
            cx,
            move |ops| manage::update_shaders(ops, &id, &o.packages),
            |this, _, _| this.reshade.message = Some((tr!("Shaders updated. Files you changed were kept.").into(), false)),
        );
    }

    pub(super) fn add_reshade_package(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(p) = self.reshade.overview.as_ref().and_then(|o| o.packages.iter().find(|p| p.name == name).cloned()) else { return };
        let id = self.reshade_id();
        self.reshade_op(
            tr!("Installing shaders: {name}…", name).to_string(),
            cx,
            move |ops| manage::add_packages(ops, &id, std::slice::from_ref(&p)),
            |_, _, _| {},
        );
    }

    pub(super) fn pick_reshade_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(window, cx, true, tr!("Choose a folder of ReShade shaders"), |this: &mut Self, path, _, cx| {
            this.add_reshade_extra(path, cx)
        });
    }

    /// A folder or a repository address.
    pub(super) fn add_reshade_extra(&mut self, source: String, cx: &mut Context<Self>) {
        let id = self.reshade_id();
        let name = source.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("").to_string();
        self.reshade.adding_repo = false;
        self.reshade_op(
            tr!("Installing shaders: {name}…", name).to_string(),
            cx,
            move |ops| manage::add_extra(ops, &id, &source),
            move |this, _, _| this.reshade.message = Some((tr!("Shaders from {name} added.", name), false)),
        );
    }

    pub(super) fn add_reshade_repo(&mut self, cx: &mut Context<Self>) {
        let url = self.input_text("reshade.repo", cx).trim().to_string();
        if url.is_empty() {
            return;
        }
        self.add_reshade_extra(url, cx);
    }

    /// Asks first when the preset uses effects of the source.
    pub(super) fn ask_remove_source(&mut self, name: String, cx: &mut Context<Self>) {
        let used = self
            .reshade
            .overview
            .as_ref()
            .and_then(|o| o.sources.iter().find(|s| s.name == name))
            .map(|s| s.used.clone())
            .unwrap_or_default();
        match used.is_empty() {
            true => self.remove_reshade_source(name, cx),
            false => {
                self.reshade.confirm = Some(Confirm::RemoveSource { name, used });
                cx.notify();
            }
        }
    }

    pub(super) fn remove_reshade_source(&mut self, name: String, cx: &mut Context<Self>) {
        let id = self.reshade_id();
        self.reshade_op(
            tr!("Removing shaders…").to_string(),
            cx,
            move |ops| manage::remove_source(ops.paths, &id, &name),
            |this, (_, kept), _| {
                let mut text =
                    tr!("Removed. The effects disappear when ReShade reloads (Reload in its menu, or the next start).").to_string();
                if !kept.files.is_empty() {
                    text.push(' ');
                    text.push_str(&tr!("Kept because you changed them: {files}", files = kept.files.join(", ")));
                }
                this.reshade.message = Some((text, false));
            },
        );
    }

    pub(super) fn remove_reshade(&mut self, cx: &mut Context<Self>) {
        let id = self.reshade_id();
        // Gone from the next read: a Steam line's overrides go now.
        let dlls = self.reshade.overview.as_ref().and_then(|o| o.manifest.as_ref()).map(super::reshade_dlls).unwrap_or_default();
        self.reshade_op(
            tr!("Removing ReShade…").to_string(),
            cx,
            move |ops| manage::remove(ops.paths, &id, &|p: &Path| trash::delete(p).map_err(|e| format!("{}: {e}", p.display()))),
            move |this, removal, cx| {
                if this.reshade_wine() == Some(true) {
                    this.steam_reshade_overrides(&dlls, false, cx);
                }
                this.set_all(
                    vec![
                        (reshade::KEY_ENABLED, Value::Null),
                        (reshade::KEY_TARGET, Value::Null),
                        (reshade::KEY_API, Value::Null),
                        (reshade::KEY_ANTICHEAT_ACK, Value::Null),
                    ],
                    cx,
                );
                let mut text = match removal.trashed.is_empty() {
                    true => tr!("ReShade was removed.").to_string(),
                    false => tr!("ReShade was removed. ReShade.ini and the preset are in the trash.").to_string(),
                };
                if !removal.not_trashed.is_empty() {
                    text.push(' ');
                    text.push_str(&tr!(
                        "The trash refused {files}, so they stay in the game's folder.",
                        files = removal.not_trashed.join(", ")
                    ));
                }
                if !removal.kept.is_empty() {
                    text.push(' ');
                    text.push_str(&tr!("Kept because you changed them: {files}", files = removal.kept.join(", ")));
                }
                this.reshade.message = Some((text, false));
            },
        );
    }

    pub(super) fn open_reshade_folder(&self) {
        if let Some(m) = self.reshade.overview.as_ref().and_then(|o| o.manifest.as_ref()) {
            crate::ui::open_external(&m.target_dir);
        }
    }

    /// Drops the DLLs ReShade needs from the user's `WINEDLLOVERRIDES`
    /// (Heronite adds them while ReShade is on).
    pub(super) fn clean_reshade_overrides(&mut self, cx: &mut Context<Self>) {
        let Some(o) = self.reshade.overview.clone() else { return };
        let dlls: Vec<String> = o.user_overrides.iter().map(|d| format!("{d}.dll")).collect();
        let dlls: Vec<&str> = dlls.iter().map(String::as_str).collect();
        let Some(Value::Array(list)) = self.settings.get("enviromentOptions").cloned() else { return };
        let list: Vec<Value> = list
            .into_iter()
            .filter_map(|mut e| {
                let key = e.get("key").and_then(Value::as_str).unwrap_or("");
                if !key.eq_ignore_ascii_case("WINEDLLOVERRIDES") {
                    return Some(e);
                }
                let value = reshade::strip_overrides(e.get("value").and_then(Value::as_str).unwrap_or(""), &dlls);
                if value.is_empty() {
                    return None;
                }
                e["value"] = Value::String(value);
                Some(e)
            })
            .collect();
        self.set("enviromentOptions", Value::Array(list), cx);
        self.load_reshade(false, cx);
    }

    /// "Choose…" / "Other program…": any program of the game, picked in a
    /// file dialog.
    pub(super) fn browse_reshade_exe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(window, cx, false, tr!("Choose the program that draws the game (.exe)"), |this: &mut Self, path, _, cx| {
            let path = PathBuf::from(path);
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                this.reshade.message = Some((tr!("ReShade goes next to a Windows program: choose the game's .exe.").into(), true));
                return cx.notify();
            }
            this.reshade.message = None;
            this.pick_reshade_exe(Some(path), cx);
        });
    }

    /// "Change…": the executable ReShade is for.
    pub(super) fn pick_reshade_exe(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let v = path.map(|p| Value::String(p.to_string_lossy().into_owned())).unwrap_or(Value::String(String::new()));
        self.set(reshade::KEY_TARGET, v, cx);
        self.reshade.picked = None;
        self.load_reshade(false, cx);
    }

    pub(super) fn pick_reshade_api(&mut self, choice: &str, cx: &mut Context<Self>) {
        self.set(reshade::KEY_API, Value::String(choice.to_string()), cx);
        self.load_reshade(false, cx);
    }

    /// A debug step that only changed the view: go on with the next one.
    pub(super) fn next_reshade_debug_after_view(&mut self, cx: &mut Context<Self>) {
        self.next_reshade_debug(cx);
    }
}

pub(super) fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}
