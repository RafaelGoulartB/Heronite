//! What the manager's buttons do: calls into the mods service, the file
//! chooser, the browser and the file manager.

use std::path::PathBuf;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{Confirm, ModsDialog, ModsDialogEvent, Rename, RowMenu, View};
use crate::ui::fields;

impl ModsDialog {
    /// Archives or folders dropped on the drawer, or picked.
    pub(super) fn install_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let key = self.key.clone();
        for p in paths {
            self.service.update(cx, |s, cx| s.install_path(&key, p, cx));
        }
        self.tab = super::Tab::Installed;
        cx.notify();
    }

    /// "Add from file…" (`folder` = false: archives) and "Add a folder…".
    pub(super) fn choose_mod(&mut self, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = if folder { tr!("Add a folder as a mod") } else { tr!("Add a mod from a file") };
        fields::choose_path(window, cx, folder, prompt, |this, path, _, cx| this.install_paths(vec![PathBuf::from(path)], cx));
    }

    /// Opens a folder of ours in the file manager, creating it first.
    pub(super) fn open_dir(&self, dir: PathBuf, cx: &mut Context<Self>) {
        cx.background_spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            crate::ui::open_external(&dir.to_string_lossy());
        })
        .detach();
    }

    pub(super) fn open_url(&self, url: String, cx: &mut Context<Self>) {
        cx.background_spawn(async move { crate::ui::open_external(&url) }).detach();
    }

    pub(super) fn mod_dir(&self, id: &str, cx: &App) -> PathBuf {
        self.service.read(cx).root().game(&self.key).mod_dir(id)
    }

    pub(super) fn store_dir(&self, cx: &App) -> PathBuf {
        self.service.read(cx).root().game(&self.key).root().to_path_buf()
    }

    pub(super) fn open_menu(&mut self, id: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = (self.menu.as_ref().map(|m| m.id.as_str()) != Some(id)).then(|| RowMenu { id: id.to_string(), at });
        cx.notify();
    }

    pub(super) fn set_enabled(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        let key = self.key.clone();
        self.service.update(cx, |s, cx| s.set_enabled(&key, id, on, cx));
    }

    pub(super) fn move_to(&mut self, id: &str, index: usize, cx: &mut Context<Self>) {
        let key = self.key.clone();
        self.service.update(cx, |s, cx| s.move_mod(&key, id, index, cx));
    }

    pub(super) fn remove_mod(&mut self, id: &str, cx: &mut Context<Self>) {
        let key = self.key.clone();
        self.confirm = None;
        self.expanded.remove(id);
        self.service.update(cx, |s, cx| s.remove(&key, id, cx));
        cx.notify();
    }

    pub(super) fn start_rename(&mut self, id: &str, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let input = fields::new_input(window, cx, name, tr!("Name"));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        let target = id.to_string();
        fields::on_commit(&input, window, cx, move |this, value, _, cx| this.finish_rename(&target, value, cx));
        self.rename = Some(Rename { id: id.to_string(), input });
        cx.notify();
    }

    fn finish_rename(&mut self, id: &str, value: String, cx: &mut Context<Self>) {
        if self.rename.as_ref().is_none_or(|r| r.id != id) {
            return;
        }
        self.rename = None;
        let key = self.key.clone();
        if !value.is_empty() {
            self.service.update(cx, |s, cx| s.rename(&key, id, value, cx));
        }
        cx.notify();
    }

    pub(super) fn ask(&mut self, what: Confirm, cx: &mut Context<Self>) {
        self.menu = None;
        self.naming_overwrite = None;
        self.confirm = Some(what);
        cx.notify();
    }

    pub(super) fn clear_overwrite(&mut self, cx: &mut Context<Self>) {
        let key = self.key.clone();
        self.confirm = None;
        self.service.update(cx, |s, cx| s.clear_overwrite(&key, cx));
        cx.notify();
    }

    pub(super) fn start_naming_overwrite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        let input = fields::new_input(window, cx, tr!("Game files"), tr!("Name"));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.naming_overwrite = Some(input);
        cx.notify();
    }

    pub(super) fn overwrite_into_mod(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let key = self.key.clone();
        self.naming_overwrite = None;
        self.service.update(cx, |s, cx| s.overwrite_into_mod(&key, name, cx));
        cx.notify();
    }

    pub(super) fn forget_game(&mut self, cx: &mut Context<Self>) {
        let key = self.key.clone();
        self.confirm = None;
        self.service.update(cx, |s, cx| s.forget_game(&key, cx));
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        cx.emit(ModsDialogEvent::OpenSettings);
    }

    pub(super) fn check_updates(&mut self, cx: &mut Context<Self>) {
        let key = self.key.clone();
        self.service.update(cx, |s, cx| s.check_updates(&key, cx));
    }

    /// Whether the game has any job that is still going.
    pub(super) fn busy(v: &View) -> bool {
        v.jobs.iter().any(|j| super::format::is_active(j.stage))
    }
}
