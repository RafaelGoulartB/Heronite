//! Editing the profile list of the emulator.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::catalog::emulator_definition;
use heronite_emulation::detect::profiles_for_definition;
use heronite_emulation::store::{catalog_profile, new_id};
use heronite_emulation::SystemEnv;
use heronite_i18n::tr;
use heronite_launch::emulators::UserEmulatorProfile;

use super::{text_of, EmulatorEditDialog, ProfileForm};
use crate::ui::fields::new_input;

fn split_extensions(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in text.split(|c: char| c == ',' || c.is_whitespace()) {
        let e = e.trim().trim_start_matches('.').to_lowercase();
        if !e.is_empty() && !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

impl EmulatorEditDialog {
    pub(super) fn core_relevant(&self, p: &UserEmulatorProfile) -> bool {
        p.core_path.is_some()
            || p.arguments.contains("{CorePath}")
            || emulator_definition(&self.definition_id)
                .and_then(|d| d.profiles.iter().find(|x| Some(&x.id) == p.definition_profile_id.as_ref()))
                .is_some_and(|d| d.core.is_some())
    }

    pub(super) fn edit_profile(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        let Some(p) = self.profiles.get(i).cloned() else { return };
        self.open_profile = Some(i);
        self.menu = None;
        self.form = Some(ProfileForm {
            name: new_input(window, cx, &p.name, tr!("Profile name")),
            platform: new_input(window, cx, p.platform_name.as_deref().unwrap_or(""), tr!("Platform (e.g. PlayStation 2)")),
            arguments: new_input(window, cx, &p.arguments, "{ImagePath}"),
            extensions: new_input(window, cx, &p.extensions.join(", "), "iso, cue, chd"),
            core: new_input(window, cx, p.core_path.as_deref().unwrap_or(""), "/path/to/core_libretro.so"),
            working_dir: p.working_dir.clone(),
        });
        cx.notify();
    }

    /// Writes the open profile's inputs back into `self.profiles`.
    pub(super) fn commit_profile(&mut self, cx: &App) {
        let (Some(i), Some(f)) = (self.open_profile, self.form.as_ref()) else { return };
        let Some(p) = self.profiles.get_mut(i) else { return };
        p.name = text_of(&f.name, cx);
        let platform = text_of(&f.platform, cx);
        p.platform_name = (!platform.is_empty()).then_some(platform);
        p.arguments = text_of(&f.arguments, cx);
        p.extensions = split_extensions(&text_of(&f.extensions, cx));
        let core = text_of(&f.core, cx);
        p.core_path = (!core.is_empty()).then_some(core);
        p.working_dir = f.working_dir.clone();
    }

    pub(super) fn close_form(&mut self, cx: &App) {
        self.commit_profile(cx);
        self.open_profile = None;
        self.form = None;
    }

    pub(super) fn add_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        self.profiles.push(UserEmulatorProfile {
            id: new_id(),
            name: "New profile".into(),
            platform_name: Some("Other".into()),
            platform_id: Some("other".into()),
            arguments: "{ImagePath}".into(),
            ..Default::default()
        });
        let i = self.profiles.len() - 1;
        self.edit_profile(i, window, cx);
    }

    /// Catalog profiles of this definition the emulator does not have yet.
    pub(super) fn missing_catalog_profiles(&self) -> Vec<String> {
        let Some(def) = emulator_definition(&self.definition_id) else { return vec![] };
        if def.id == "custom" {
            return vec![];
        }
        def.profiles
            .iter()
            .filter(|d| !self.profiles.iter().any(|p| p.definition_profile_id.as_deref() == Some(d.id.as_str())))
            .map(|d| d.id.clone())
            .collect()
    }

    pub(super) fn add_catalog_profiles(&mut self, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        let exe = text_of(&self.executable, cx);
        let kind = self.launch_kind.clone();
        let fresh = profiles_for_definition(&SystemEnv, &self.definition_id, &exe, &kind);
        for id in self.missing_catalog_profiles() {
            // Prefer the version with its core resolved.
            let p = fresh
                .iter()
                .find(|p| p.definition_profile_id.as_deref() == Some(id.as_str()))
                .cloned()
                .or_else(|| catalog_profile(&self.definition_id, &id));
            if let Some(p) = p {
                self.profiles.push(p);
            }
        }
        cx.notify();
    }

    pub(super) fn remove_profile(&mut self, i: usize, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        if self.profiles.len() <= 1 || i >= self.profiles.len() {
            return;
        }
        self.profiles.remove(i);
        self.open_profile = None;
        self.form = None;
        cx.notify();
    }

    pub(super) fn profile_users(&self, p: &UserEmulatorProfile) -> usize {
        if self.create {
            return 0;
        }
        heronite_emulation::manager::scanners_using_profile(&self.scanners, &self.base.id, &p.id)
    }
}

#[cfg(test)]
mod tests {
    use super::split_extensions;

    #[test]
    fn extensions_are_normalised() {
        assert_eq!(split_extensions(".ISO, cue  chd,,iso"), ["iso", "cue", "chd"]);
        assert!(split_extensions("  ").is_empty());
    }
}
