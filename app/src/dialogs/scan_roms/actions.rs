//! Background work of the dialog: detecting emulators, saving a custom one, scanning and importing.

use gpui_kit::prelude::*;
use heronite_emulation::{
    add_detected_emulators, import_scanned_roms, preview_rom_scan, upsert_user_emulator, RomScanArgs, RomScanImportArgs, SystemEnv,
    UpsertUserEmulatorArgs,
};
use heronite_i18n::tr;
use heronite_launch::emulators::UserEmulatorProfile;

use super::{text_of, ScanInitial, ScanRomsDialog};
use crate::dialogs::DialogEvent;

impl ScanRomsDialog {
    pub(super) fn detect(&mut self, cx: &mut Context<Self>) {
        self.detecting = true;
        self.error.clear();
        let paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { add_detected_emulators(&paths, &SystemEnv) }).await;
            this.update(cx, |this, cx| {
                this.detecting = false;
                match res {
                    Ok(list) => {
                        this.reload(true, false, cx);
                        if list.is_empty() {
                            this.error = tr!("No emulators were found on PATH or Flatpak. Add one manually.").into();
                        }
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn add_custom(&mut self, cx: &mut Context<Self>) {
        let exe = text_of(&self.custom_exe, cx);
        if exe.is_empty() {
            return;
        }
        let name = text_of(&self.custom_name, cx);
        let extensions: Vec<String> = text_of(&self.custom_ext, cx)
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(|e| e.trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        let args = text_of(&self.custom_args, cx);
        let paths = self.ctx.data_paths.clone();
        self.loading = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let display = if name.is_empty() { "Custom emulator".to_string() } else { name.clone() };
                    upsert_user_emulator(
                        &paths,
                        UpsertUserEmulatorArgs {
                            definition_id: "custom".into(),
                            name: display,
                            launch_kind: if exe.to_lowercase().ends_with(".exe") { "wine" } else { "native" }.into(),
                            executable: exe,
                            profiles: Some(vec![UserEmulatorProfile {
                                id: "custom".into(),
                                definition_profile_id: Some("custom".into()),
                                name: if name.is_empty() { "Custom".into() } else { name },
                                platform_name: Some("Other".into()),
                                arguments: if args.is_empty() { "{ImagePath}".into() } else { args },
                                extensions,
                                ..Default::default()
                            }]),
                            ..Default::default()
                        },
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(e) => {
                        this.custom_open = false;
                        // Select it as soon as the state is reloaded.
                        this.initial = ScanInitial {
                            emulator_id: Some(e.id.clone()),
                            profile_id: e.profiles.first().map(|p| p.id.clone()),
                            directory: None,
                        };
                        this.reload_then_initial(cx);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn reload_then_initial(&self, cx: &mut Context<Self>) {
        self.reload(false, true, cx);
    }

    pub(super) fn scan(&mut self, cx: &mut Context<Self>) {
        let directory = text_of(&self.directory, cx);
        let Some(profile) = self.profile().map(|p| p.id.clone()) else { return };
        if self.emulator_id.is_empty() || directory.is_empty() {
            return;
        }
        let args = RomScanArgs {
            emulator_id: self.emulator_id.clone(),
            profile_id: profile,
            directory,
            scan_subfolders: Some(self.scan_subfolders),
            merge_related_files: Some(self.merge_related),
        };
        let paths = self.ctx.data_paths.clone();
        self.loading = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { preview_rom_scan(&paths, &args) }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(p) => {
                        if !p.errors.is_empty() {
                            this.error = p.errors.join("\n");
                        }
                        this.preview = Some(p);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn import(&mut self, cx: &mut Context<Self>) {
        let (Some(preview), Some(profile)) = (self.preview.clone(), self.profile().map(|p| p.id.clone())) else {
            return;
        };
        let args = RomScanImportArgs {
            emulator_id: self.emulator_id.clone(),
            profile_id: profile,
            directory: text_of(&self.directory, cx),
            scanner_id: self.scanner_id.clone(),
            scan_subfolders: Some(self.scan_subfolders),
            merge_related_files: Some(self.merge_related),
            auto_scan_on_refresh: Some(self.auto_scan),
            games: preview.games,
        };
        let paths = self.ctx.data_paths.clone();
        self.loading = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { import_scanned_roms(&paths, &args) }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(r) if r.errors.is_empty() => cx.emit(DialogEvent::ChangedAndClose),
                    Ok(r) => {
                        this.error = r.errors.join("\n");
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn toggle_all(&mut self, on: bool) {
        if let Some(p) = self.preview.as_mut() {
            for g in p.games.iter_mut().filter(|g| !g.already_imported) {
                g.import = on;
            }
        }
    }
}
