//! `HERONITE_DEBUG_EMU_PAGE`: opens a dialog or a notice for screenshots and
//! tests (use a fixture dir). Dialogs need a `Window`, so the request waits
//! for the next render.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::AutoScanReport;

use super::{scan_notice, EmulatorsPage};
use crate::dialogs::emulator_confirm::EmulatorConfirmDialog;
use crate::dialogs::emulator_edit::EmulatorEditDialog;
use crate::dialogs::rom_folder::RomFolderDialog;

impl EmulatorsPage {
    pub(super) fn run_debug(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.debug.take() else { return };
        let (kind, arg) = which.split_once(':').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((which, None));
        let first = self.summaries.first().map(|s| s.emulator.id.clone());
        match kind.as_str() {
            "add" => {
                self.open_add_emulator(window, cx);
                if let Some(def) = arg {
                    if let Some(d) = self.open_dialog_as::<EmulatorEditDialog>() {
                        d.update(cx, |d, cx| d.debug_select_definition(&def, cx));
                    }
                }
            }
            "edit" | "edit-profile" => {
                if let Some(id) = arg.or(first) {
                    self.open_edit_emulator(&id, window, cx);
                    if kind == "edit-profile" {
                        if let Some(d) = self.open_dialog_as::<EmulatorEditDialog>() {
                            d.update(cx, |d, cx| d.open_first_profile(window, cx));
                        }
                    }
                }
            }
            "add-save" => {
                // `add-save:<definition>,<executable>[,<name>]`
                let arg = arg.unwrap_or_default();
                let mut it = arg.splitn(3, ',');
                let (def, exe, name) =
                    (it.next().unwrap_or("custom").to_string(), it.next().unwrap_or("").to_string(), it.next().map(str::to_string));
                self.open_add_emulator(window, cx);
                if let Some(d) = self.open_dialog_as::<EmulatorEditDialog>() {
                    d.update(cx, |d, cx| d.debug_fill_and_save(&def, &exe, name.as_deref(), cx));
                }
            }
            "edit-save" => {
                // `edit-save:<executable>`: edits the first emulator.
                if let Some(id) = first {
                    self.open_edit_emulator(&id, window, cx);
                    if let Some(d) = self.open_dialog_as::<EmulatorEditDialog>() {
                        let exe = arg.unwrap_or_default();
                        d.update(cx, |d, cx| d.debug_fill_and_save("", &exe, Some("Renamed emulator"), cx));
                    }
                }
            }
            "dup" => {
                if let Some(id) = first {
                    self.duplicate(&id, cx);
                }
            }
            "scan-all" => self.scan_all(cx),
            "remove-yes" | "remove-folder-yes" => {
                if kind == "remove-yes" {
                    if let Some(id) = arg.or(first) {
                        self.confirm_remove_emulator(&id, window, cx);
                    }
                } else if let Some(s) = self.summaries.iter().flat_map(|s| s.scanners.last().cloned()).next() {
                    self.confirm_remove_folder(&s, window, cx);
                }
                if let Some(d) = self.open_dialog_as::<EmulatorConfirmDialog>() {
                    d.update(cx, |d, cx| d.debug_yes(cx));
                }
            }
            "folder-save" => {
                // `folder-save:<directory>`
                self.open_folder_dialog(first, None, window, cx);
                if let Some(d) = self.open_dialog_as::<RomFolderDialog>() {
                    let dir = arg.unwrap_or_default();
                    d.update(cx, |d, cx| d.debug_fill_and_save(&dir, window, cx));
                }
            }
            "pick-rom" => {
                // Dialog of the Collection's launch flow with sample discs.
                let roms = vec![
                    ("Disc 1".to_string(), "/tmp/roms/Final Fantasy X (Disc 1).iso".to_string()),
                    ("Disc 2".to_string(), "/tmp/roms/Final Fantasy X (Disc 2).iso".to_string()),
                ];
                let view = cx.new(|_| {
                    crate::dialogs::pick_rom::PickRomDialog::new("Final Fantasy X".into(), roms, None, std::rc::Rc::new(|_, _| {}))
                });
                self.open_dialog(view, window, cx);
            }
            "launch-state" => {
                use crate::dialogs::launch_state::{LaunchStateDialog, StateRow, ThumbRaw};
                let thumb = |shade: u8| ThumbRaw {
                    width: 96,
                    height: 72,
                    bgra: (0..96 * 72).flat_map(|i| [shade, 90 + (i % 96) as u8, 60, 255]).collect(),
                };
                let at = |ago: i64| (chrono::Utc::now() - chrono::Duration::hours(ago)).to_rfc3339();
                let states = vec![
                    StateRow { path: "/tmp/s/resume".into(), resume: true, slot: None, modified_at: at(2), thumb: Some(thumb(40)) },
                    StateRow { path: "/tmp/s/1".into(), resume: false, slot: Some(1), modified_at: at(30), thumb: Some(thumb(160)) },
                    StateRow { path: "/tmp/s/2".into(), resume: false, slot: Some(2), modified_at: at(400), thumb: None },
                ];
                let view = cx.new(|_| LaunchStateDialog::new("Gran Turismo 4".into(), "PCSX2".into(), states, std::rc::Rc::new(|_, _| {})));
                self.open_dialog(view, window, cx);
            }
            "detect" => self.open_detect(window, cx),
            "folder" => self.open_folder_dialog(first, None, window, cx),
            "folder-edit" => {
                let scanner = self.summaries.iter().flat_map(|s| s.scanners.first().cloned()).next();
                self.open_folder_dialog(None, scanner, window, cx);
            }
            "remove" => {
                if let Some(id) = first {
                    self.confirm_remove_emulator(&id, window, cx);
                }
            }
            "remove-folder" => {
                if let Some(s) = self.summaries.iter().flat_map(|s| s.scanners.first().cloned()).next() {
                    self.confirm_remove_folder(&s, window, cx);
                }
            }
            "notice" => {
                self.notice = Some(scan_notice(&AutoScanReport {
                    scanned: 2,
                    imported: 3,
                    errors: vec!["PS3 \u{b7} RPCS3: Folder does not exist: /mnt/games/ps3".into()],
                }));
            }
            _ => {}
        }
        cx.notify();
    }
}
