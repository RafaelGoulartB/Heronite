//! Emulators page: the one place to manage emulators. Lists the configured
//! emulators with their health, the games using them and their ROM folders;
//! adds (catalog or custom), edits, duplicates and removes them; detects
//! emulators on this PC or in a folder; manages the ROM folders and scans them.
//!
//! Data: `local_library/emulators.json` and `rom-scanners.json` through
//! `heronite_emulation`. All reads and scans run off the UI thread.
//!
//! Debug: `HERONITE_DEBUG_EMU_PAGE=add|add:<definition>|edit|edit-profile|detect|folder|folder-edit|
//! remove|remove-folder|notice` opens that state (use a fixture dir).

use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::manager::{data_stamp, overview, EmulatorSummary, Health};
use heronite_emulation::{duplicate_user_emulator, run_rom_scanners, AutoScanReport, RomScanner, SystemEnv};

use super::{page_body, page_column, PageCtx, PageEvent};
use crate::dialogs::emulator_confirm::EmulatorConfirmDialog;
use crate::dialogs::emulator_detect::DetectEmulatorsDialog;
use crate::dialogs::emulator_edit::EmulatorEditDialog;
use crate::dialogs::rom_folder::RomFolderDialog;
use crate::dialogs::scan_roms::{profile_label, ScanInitial};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::fields::{meta_text, panel, paragraph};
use crate::ui::icon;

/// A message under the toolbar (result of a scan, an add, a removal...).
#[derive(Clone)]
struct Notice {
    lines: Vec<String>,
    error: bool,
}

pub struct EmulatorsPage {
    ctx: PageCtx,
    summaries: Vec<EmulatorSummary>,
    loaded: bool,
    stamp: Vec<Option<SystemTime>>,
    /// What is running in the background ("Scanning ROM folders...").
    busy: Option<SharedString>,
    notice: Option<Notice>,
    dialog: Option<AnyView>,
    dialog_sub: Option<Subscription>,
    /// Shown as the notice when the open confirmation succeeds.
    confirm_notice: Option<String>,
    debug: Option<String>,
}

impl EventEmitter<PageEvent> for EmulatorsPage {}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The notice for a finished scan of ROM folders.
fn scan_notice(report: &AutoScanReport) -> Notice {
    let mut lines = vec![format!(
        "Scanned {}: added {}.",
        plural(report.scanned, "folder", "folders"),
        plural(report.imported, "new game", "new games")
    )];
    lines.extend(report.errors.iter().cloned());
    Notice { lines, error: !report.errors.is_empty() && report.imported == 0 }
}

impl EmulatorsPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = Self {
            ctx,
            summaries: vec![],
            loaded: false,
            stamp: vec![],
            busy: None,
            notice: None,
            dialog: None,
            dialog_sub: None,
            confirm_notice: None,
            debug: std::env::var("HERONITE_DEBUG_EMU_PAGE").ok(),
        };
        this.reload(cx);
        // Heronite, the Collection refresh and the scan dialogs write the same
        // files: pick the changes up (a few stat calls every 2 seconds).
        let paths = this.ctx.heroic.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            let p = paths.clone();
            let stamp = cx.background_spawn(async move { data_stamp(&p) }).await;
            let Ok(changed) = this.update(cx, |this, _| this.loaded && this.stamp != stamp) else { break };
            if changed && this.update(cx, |this, cx| this.reload(cx)).is_err() {
                break;
            }
        })
        .detach();
        this
    }

    fn reload(&self, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let (res, stamp) = cx
                .background_spawn(async move {
                    let stamp = data_stamp(&paths);
                    (overview(&paths, &SystemEnv), stamp)
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(s) => {
                        this.summaries = s;
                        this.stamp = stamp;
                        this.loaded = true;
                    }
                    Err(e) => this.notice = Some(Notice { lines: vec![format!("{e:#}")], error: true }),
                }
                if let Some(d) = this.debug.take() {
                    this.apply_debug(&d, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toast(&self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast(text.into(), error));
    }

    fn emulators(&self) -> Vec<heronite_launch::emulators::UserEmulator> {
        self.summaries.iter().map(|s| s.emulator.clone()).collect()
    }

    fn total_folders(&self) -> usize {
        self.summaries.iter().map(|s| s.scanners.len()).sum()
    }

    // ---- dialogs ---------------------------------------------------------

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
                DialogEvent::Open(_) => {}
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

    fn open_add_emulator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        let view = cx.new(|cx| EmulatorEditDialog::new(paths, None, window, cx));
        self.open_dialog(view, window, cx);
    }

    fn open_edit_emulator(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.summaries.iter().find(|s| s.emulator.id == id).map(|s| s.emulator.clone()) else { return };
        let paths = self.ctx.heroic.clone();
        let view = cx.new(|cx| EmulatorEditDialog::new(paths, Some(e), window, cx));
        self.open_dialog(view, window, cx);
    }

    fn open_detect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        let view = cx.new(|cx| DetectEmulatorsDialog::new(paths, cx));
        self.open_dialog(view, window, cx);
    }

    fn open_folder_dialog(
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

    fn confirm_remove_emulator(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn confirm_remove_folder(&mut self, scanner: &RomScanner, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_notice = Some(format!("Removed ROM folder {}.", scanner.directory));
        let paths = self.ctx.heroic.clone();
        let (id, dir) = (scanner.id.clone(), scanner.directory.clone());
        let view = cx.new(|_| EmulatorConfirmDialog::remove_scanner(paths, id, &dir));
        self.open_dialog(view, window, cx);
    }

    // ---- actions ---------------------------------------------------------

    fn duplicate(&mut self, id: &str, cx: &mut Context<Self>) {
        let (paths, id) = (self.ctx.heroic.clone(), id.to_string());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { duplicate_user_emulator(&paths, &id) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(e) => this.notice = Some(Notice { lines: vec![format!("Created {}.", e.name)], error: false }),
                    Err(e) => this.notice = Some(Notice { lines: vec![format!("{e:#}")], error: true }),
                }
                this.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Scans the given saved folders now (even those with "scan on refresh"
    /// off) and imports the games not imported yet.
    fn scan(&mut self, ids: Vec<String>, label: &'static str, cx: &mut Context<Self>) {
        if self.busy.is_some() || ids.is_empty() {
            return;
        }
        self.busy = Some(label.into());
        self.notice = None;
        let paths = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let report = cx
                .background_spawn(async move {
                    let mut total = AutoScanReport::default();
                    for id in ids {
                        match run_rom_scanners(&paths, Some(&id)) {
                            Ok(r) => {
                                total.scanned += r.scanned;
                                total.imported += r.imported;
                                total.errors.extend(r.errors);
                            }
                            Err(e) => total.errors.push(format!("{e:#}")),
                        }
                    }
                    total
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                let notice = scan_notice(&report);
                if report.imported > 0 {
                    this.toast(format!("Added {} new game(s)", report.imported), false, cx);
                    cx.emit(PageEvent::LibraryChanged);
                }
                this.notice = Some(notice);
                this.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn scan_all(&mut self, cx: &mut Context<Self>) {
        let ids = self.summaries.iter().flat_map(|s| s.scanners.iter().map(|x| x.id.clone())).collect();
        self.scan(ids, "Scanning all ROM folders\u{2026}", cx);
    }

    fn apply_debug(&mut self, which: &str, cx: &mut Context<Self>) {
        // Dialogs need a `Window`; they are opened from the next render.
        self.debug = Some(which.to_string());
        cx.notify();
    }

    fn run_debug(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.debug.take() else { return };
        let (kind, arg) = which.split_once(':').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((which, None));
        let first = self.summaries.first().map(|s| s.emulator.id.clone());
        match kind.as_str() {
            "add" => {
                self.open_add_emulator(window, cx);
                if let Some(def) = arg {
                    if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<EmulatorEditDialog>().ok()) {
                        d.update(cx, |d, cx| d.debug_select_definition(&def, cx));
                    }
                }
            }
            "edit" | "edit-profile" => {
                if let Some(id) = arg.or(first) {
                    self.open_edit_emulator(&id, window, cx);
                    if kind == "edit-profile" {
                        if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<EmulatorEditDialog>().ok()) {
                            d.update(cx, |d, cx| d.open_first_profile(window, cx));
                        }
                    }
                }
            }
            "add-save" => {
                // `add-save:<definition>,<executable>[,<name>]`
                let arg = arg.unwrap_or_default();
                let mut it = arg.splitn(3, ',');
                let (def, exe, name) = (it.next().unwrap_or("custom").to_string(), it.next().unwrap_or("").to_string(), it.next().map(str::to_string));
                self.open_add_emulator(window, cx);
                if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<EmulatorEditDialog>().ok()) {
                    d.update(cx, |d, cx| d.debug_fill_and_save(&def, &exe, name.as_deref(), cx));
                }
            }
            "edit-save" => {
                // `edit-save:<executable>`: edits the first emulator.
                if let Some(id) = first {
                    self.open_edit_emulator(&id, window, cx);
                    if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<EmulatorEditDialog>().ok()) {
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
                if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<EmulatorConfirmDialog>().ok()) {
                    d.update(cx, |d, cx| d.debug_yes(cx));
                }
            }
            "folder-save" => {
                // `folder-save:<directory>`
                self.open_folder_dialog(first, None, window, cx);
                if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<RomFolderDialog>().ok()) {
                    let dir = arg.unwrap_or_default();
                    d.update(cx, |d, cx| d.debug_fill_and_save(&dir, window, cx));
                }
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

fn heading(text: &str) -> Div {
    div()
        .font_family(theme::FONT_HEADING)
        .text_size(px(18.))
        .font_weight(theme::W_BOLD)
        .text_color(rgb(0xcaf3fd))
        .child(text.to_string())
}

fn chip(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(8.))
        .py(px(2.))
        .rounded(px(10.))
        .bg(theme::white_a(0.1))
        .text_size(px(12.))
        .text_color(rgb(0xcaf3fd))
        .child(text.into())
}

fn health_badge(health: Health) -> Div {
    let (color, glyph) = if health.is_ok() {
        (theme::success(), Icon::Check)
    } else {
        (theme::danger(), Icon::Warning)
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(13.33))
        .text_color(color)
        .child(icon(glyph, px(14.), color))
        .child(health.label())
}

fn launch_label(kind: &str) -> &'static str {
    match kind {
        "flatpak" => "Flatpak",
        "wine" => "Wine",
        _ => "Native",
    }
}

fn folder_flags(s: &RomScanner) -> String {
    let mut flags = vec![];
    flags.push(if s.scan_subfolders { "Subfolders" } else { "Top folder only" });
    if s.merge_related_files {
        flags.push("Discs merged");
    }
    flags.push(if s.auto_scan_on_refresh { "Scanned on refresh" } else { "Manual scan" });
    flags.join(" \u{b7} ")
}

impl EmulatorsPage {
    fn render_card(&self, s: &EmulatorSummary, cx: &mut Context<Self>) -> Div {
        let e = &s.emulator;
        let id = e.id.clone();
        let busy = self.busy.is_some();
        let location = if e.launch_kind == "flatpak" {
            format!("Flatpak \u{b7} {}", e.flatpak_id.clone().unwrap_or_default())
        } else if e.executable.is_empty() {
            "No executable set".to_string()
        } else {
            e.executable.clone()
        };
        let mut platforms: Vec<String> = e
            .profiles
            .iter()
            .map(|p| p.platform_name.clone().filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("other")).unwrap_or_else(|| p.name.clone()))
            .collect();
        platforms.dedup();
        let extra = platforms.len().saturating_sub(8);
        platforms.truncate(8);

        let actions = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .justify_end()
            .gap(px(8.))
            .child({
                let id = id.clone();
                button(SharedString::from(format!("emu-edit-{id}")), ButtonKind::Outline, "Edit")
                    .on_click(cx.listener(move |this, _, window, cx| this.open_edit_emulator(&id, window, cx)))
            })
            .child({
                let id = id.clone();
                button(SharedString::from(format!("emu-dup-{id}")), ButtonKind::Outline, "Duplicate")
                    .on_click(cx.listener(move |this, _, _, cx| this.duplicate(&id, cx)))
            })
            .child({
                let id = id.clone();
                button(SharedString::from(format!("emu-rm-{id}")), ButtonKind::Outline, "Remove")
                    .on_click(cx.listener(move |this, _, window, cx| this.confirm_remove_emulator(&id, window, cx)))
            });

        let mut card = panel()
            .gap(px(10.))
            .p(px(16.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .font_family(theme::FONT_HEADING)
                                            .font_weight(theme::W_BOLD)
                                            .text_size(px(18.))
                                            .text_color(white())
                                            .child(e.name.clone()),
                                    )
                                    .child(chip(launch_label(&e.launch_kind)))
                                    .child(health_badge(s.health)),
                            )
                            .child(div().text_size(px(13.33)).text_color(white()).child(location))
                            .child(
                                div()
                                    .text_size(px(13.33))
                                    .text_color(theme::white_a(0.7))
                                    .child(format!("{} using it", plural(s.games, "game", "games"))),
                            ),
                    )
                    .child(actions),
            )
            .child(
                div().flex().flex_wrap().items_center().gap(px(6.)).children(platforms.into_iter().map(chip)).children(
                    (extra > 0).then(|| chip(format!("+{extra} more"))),
                ),
            );

        // ROM folders.
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mt(px(4.))
                .child(div().text_size(px(16.)).font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child("ROM folders"))
                .child({
                    let id = id.clone();
                    button(SharedString::from(format!("emu-addfolder-{id}")), ButtonKind::Outline, "Add ROM folder")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_folder_dialog(Some(id.clone()), None, window, cx)
                        }))
                }),
        );
        if s.scanners.is_empty() {
            card = card.child(meta_text("No ROM folders yet. Add one and its games are added to the Collection."));
        }
        for sc in &s.scanners {
            let sid = sc.id.clone();
            let missing = s.missing_folders.contains(&sc.directory);
            let profile = e.profile(&sc.profile_id).map(profile_label).unwrap_or_else(|| "Unknown profile".into());
            let init = ScanInitial {
                emulator_id: Some(sc.emulator_id.clone()),
                profile_id: Some(sc.profile_id.clone()),
                directory: Some(sc.directory.clone()),
            };
            let scanner = sc.clone();
            card = card.child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap(px(12.))
                    .p(px(8.))
                    .rounded(px(4.))
                    .bg(theme::black_a(0.2))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().font_weight(theme::W_BOLD).text_size(px(15.)).truncate().child(sc.directory.clone()))
                                    .children(missing.then(|| div().text_size(px(13.33)).text_color(theme::danger()).child("Folder not found"))),
                            )
                            .child(
                                div()
                                    .text_size(px(13.33))
                                    .text_color(white())
                                    .child(format!("{profile}  \u{b7}  {}", folder_flags(sc))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(8.))
                            .child({
                                let sid = sid.clone();
                                action_button(
                                    SharedString::from(format!("scan-now-{sid}")),
                                    ButtonKind::Default,
                                    "Scan now",
                                    !busy && !missing,
                                    cx.listener(move |this, _, _, cx| this.scan(vec![sid.clone()], "Scanning\u{2026}", cx)),
                                )
                            })
                            .child(
                                button(SharedString::from(format!("scan-review-{sid}")), ButtonKind::Outline, "Review\u{2026}")
                                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::ScanRoms(init.clone())))),
                            )
                            .child({
                                let scanner = scanner.clone();
                                button(SharedString::from(format!("scan-edit-{sid}")), ButtonKind::Outline, "Edit").on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.open_folder_dialog(None, Some(scanner.clone()), window, cx)
                                    }),
                                )
                            })
                            .child({
                                let scanner = scanner.clone();
                                button(SharedString::from(format!("scan-rm-{sid}")), ButtonKind::Outline, "Remove").on_click(
                                    cx.listener(move |this, _, window, cx| this.confirm_remove_folder(&scanner, window, cx)),
                                )
                            }),
                    ),
            );
        }
        card
    }
}

impl Render for EmulatorsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.debug.is_some() && self.loaded {
            self.run_debug(window, cx);
        }
        let busy = self.busy.is_some();
        let folders = self.total_folders();

        let toolbar = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .child(button("emus-add", ButtonKind::Default, "Add emulator").on_click(cx.listener(|this, _, window, cx| {
                this.open_add_emulator(window, cx)
            })))
            .child(button("emus-detect", ButtonKind::Outline, "Detect emulators\u{2026}").on_click(cx.listener(
                |this, _, window, cx| this.open_detect(window, cx),
            )))
            .child(button("emus-scan-folder", ButtonKind::Outline, "Scan a folder for emulators\u{2026}").on_click(
                cx.listener(|this, _, window, cx| {
                    this.open_detect(window, cx);
                    if let Some(d) = this.dialog.clone().and_then(|v| v.downcast::<DetectEmulatorsDialog>().ok()) {
                        d.update(cx, |d, cx| d.pick_folder(window, cx));
                    }
                }),
            ))
            .child(button("emus-add-folder", ButtonKind::Outline, "Add ROM folder").on_click(cx.listener(|this, _, window, cx| {
                this.open_folder_dialog(None, None, window, cx)
            })))
            .child(action_button(
                "emus-scan-all",
                ButtonKind::Outline,
                "Scan all folders now",
                !busy && folders > 0,
                cx.listener(|this, _, _, cx| this.scan_all(cx)),
            ));

        let mut column = page_column()
            .max_w(px(980.))
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(12.))
                    .child(heading("Emulators"))
                    .child(meta_text(if self.loaded {
                        format!("{} \u{b7} {}", plural(self.summaries.len(), "emulator", "emulators"), plural(folders, "ROM folder", "ROM folders"))
                    } else {
                        "Loading\u{2026}".to_string()
                    })),
            )
            .child(paragraph(
                "Emulators launch the games you add from ROM folders. Folders set to scan on refresh add new games whenever the Collection refreshes.",
            ))
            .child(toolbar);

        if let Some(label) = self.busy.clone() {
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .text_color(theme::accent())
                    .child(icon(Icon::Refresh, px(16.), theme::accent()))
                    .child(label),
            );
        }
        if let Some(n) = self.notice.clone() {
            let color: Hsla = if n.error { theme::danger().into() } else { theme::success().into() };
            let mut lines = div().flex().flex_col().gap(px(2.)).flex_1().min_w_0();
            for (i, l) in n.lines.iter().enumerate() {
                lines = lines.child(div().text_size(px(14.)).text_color(if i == 0 { color } else { theme::white_a(0.8) }).child(l.clone()));
            }
            column = column.child(
                panel().flex_row().items_start().gap(px(12.)).child(lines).child(
                    button("emus-notice-x", ButtonKind::Text, "Dismiss").on_click(cx.listener(|this, _, _, cx| {
                        this.notice = None;
                        cx.notify();
                    })),
                ),
            );
        }
        if self.loaded && self.summaries.is_empty() {
            column = column.child(
                panel().p(px(18.)).gap(px(10.)).child(paragraph(
                    "No emulators configured yet. Detect the ones installed on this PC, or add one from the catalog or as a custom program.",
                )),
            );
        }
        for s in self.summaries.clone() {
            column = column.child(self.render_card(&s, cx));
        }

        div()
            .size_full()
            .relative()
            .child(page_body("page-emulators").child(column))
            .children(self.dialog.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{plural, scan_notice};
    use heronite_emulation::AutoScanReport;

    #[test]
    fn scan_notices() {
        assert_eq!(plural(1, "folder", "folders"), "1 folder");
        assert_eq!(plural(0, "new game", "new games"), "0 new games");
        let n = scan_notice(&AutoScanReport { scanned: 2, imported: 3, errors: vec![] });
        assert_eq!(n.lines, ["Scanned 2 folders: added 3 new games."]);
        assert!(!n.error);
        let n = scan_notice(&AutoScanReport { scanned: 1, imported: 0, errors: vec!["Gone: Folder does not exist: /x".into()] });
        assert!(n.error);
        assert_eq!(n.lines.len(), 2);
    }
}
