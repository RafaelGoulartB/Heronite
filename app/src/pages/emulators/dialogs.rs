//! Dialogs opened from the Emulators page and what the page says when they
//! save something.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::RomScanner;

use super::{plural, EmulatorsPage, Notice};
use crate::dialogs::emulator_confirm::EmulatorConfirmDialog;
use crate::dialogs::emulator_detect::DetectEmulatorsDialog;
use crate::dialogs::emulator_edit::EmulatorEditDialog;
use crate::dialogs::rom_folder::RomFolderDialog;
use crate::dialogs::DialogEvent;
use crate::pages::PageEvent;

impl EmulatorsPage {
    fn open_dialog<V>(&mut self, view: Entity<V>, window: &mut Window, cx: &mut Context<Self>)
    where
        V: Render + EventEmitter<DialogEvent>,
    {
        self.dialog_sub = Some(cx.subscribe_in(&view, window, |this, view, ev: &DialogEvent, _window, cx| {
            match ev {
                DialogEvent::Close => {
                    this.dialog = None;
                    this.dialog_sub = None;
                }
                DialogEvent::Changed => this.reload(cx),
                DialogEvent::ChangedAndClose => {
                    this.dialog = None;
                    this.dialog_sub = None;
                    this.reload(cx);
                    this.dialog_done(view.clone().into_any(), cx);
                }
            }
            cx.notify();
        }));
        self.dialog = Some(view.into());
        cx.notify();
    }

    /// What to tell the user after a dialog saved something.
    fn dialog_done(&mut self, view: AnyEntity, cx: &mut Context<Self>) {
        let view = match view.downcast::<EmulatorEditDialog>() {
            Ok(d) => {
                if let Some(e) = d.update(cx, |d, _| d.saved.take()) {
                    self.notice = Some(Notice { lines: vec![format!("Saved {}.", e.name)], error: false });
                }
                return;
            }
            Err(v) => v,
        };
        let view = match view.downcast::<DetectEmulatorsDialog>() {
            Ok(d) => {
                let n = d.update(cx, |d, _| d.added);
                self.notice = Some(Notice { lines: vec![format!("Added {}.", plural(n, "emulator", "emulators"))], error: false });
                return;
            }
            Err(v) => v,
        };
        let view = match view.downcast::<RomFolderDialog>() {
            Ok(d) => {
                if let Some(o) = d.update(cx, |d, _| d.outcome.take()) {
                    let mut lines = vec![if o.scanned {
                        format!("Saved {}. Added {}.", o.directory, plural(o.imported, "new game", "new games"))
                    } else {
                        format!("Saved {}.", o.directory)
                    }];
                    lines.extend(o.errors.iter().cloned());
                    self.notice = Some(Notice { lines, error: !o.errors.is_empty() && o.imported == 0 });
                    if o.imported > 0 {
                        cx.emit(PageEvent::LibraryChanged);
                    }
                }
                return;
            }
            Err(v) => v,
        };
        if view.downcast::<EmulatorConfirmDialog>().is_ok() {
            if let Some(text) = self.confirm_notice.take() {
                self.notice = Some(Notice { lines: vec![text], error: false });
            }
        }
    }

    pub(super) fn open_add_emulator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        let view = cx.new(|cx| EmulatorEditDialog::new(paths, None, window, cx));
        self.open_dialog(view, window, cx);
    }

    pub(super) fn open_edit_emulator(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.summaries.iter().find(|s| s.emulator.id == id).map(|s| s.emulator.clone()) else { return };
        let paths = self.ctx.heroic.clone();
        let view = cx.new(|cx| EmulatorEditDialog::new(paths, Some(e), window, cx));
        self.open_dialog(view, window, cx);
    }

    pub(super) fn open_detect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        let view = cx.new(|cx| DetectEmulatorsDialog::new(paths, cx));
        self.open_dialog(view, window, cx);
    }

    pub(super) fn open_folder_dialog(
        &mut self,
        emulator_id: Option<String>,
        scanner: Option<RomScanner>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.summaries.is_empty() {
            self.notice = Some(Notice { lines: vec!["Add an emulator first, then add its ROM folders.".into()], error: true });
            cx.notify();
            return;
        }
        let (paths, emulators) = (self.ctx.heroic.clone(), self.emulators());
        let view = cx.new(|cx| RomFolderDialog::new(paths, emulators, emulator_id, scanner, window, cx));
        self.open_dialog(view, window, cx);
    }

    pub(super) fn confirm_remove_emulator(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.summaries.iter().find(|s| s.emulator.id == id) else { return };
        self.confirm_notice = Some(format!("Removed {}.", s.emulator.name));
        let view = cx.new(|_| {
            EmulatorConfirmDialog::remove_emulator(
                self.ctx.heroic.clone(),
                s.emulator.id.clone(),
                &s.emulator.name,
                s.games,
                s.scanners.len(),
            )
        });
        self.open_dialog(view, window, cx);
    }

    pub(super) fn confirm_remove_folder(&mut self, scanner: &RomScanner, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_notice = Some(format!("Removed ROM folder {}.", scanner.directory));
        let paths = self.ctx.heroic.clone();
        let (id, dir) = (scanner.id.clone(), scanner.directory.clone());
        let view = cx.new(|_| EmulatorConfirmDialog::remove_scanner(paths, id, &dir));
        self.open_dialog(view, window, cx);
    }

    /// The open dialog, when it is a `V`.
    pub(super) fn open_dialog_as<V: 'static>(&self) -> Option<Entity<V>> {
        self.dialog.clone().and_then(|v| v.downcast::<V>().ok())
    }
}
