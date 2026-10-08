//! `ImportPlayniteDialog`: pick a Playnite library folder, map Windows
//! drives, preview and import. Backend: `heronite_playnite::Playnite`
//! (blocking, so every call runs in `background_spawn`).

use std::collections::HashMap;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_playnite::{DriveRemap, Playnite, PlayniteImportArgs, PlayniteImportPreview, PlayniteImportResult};

use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{button, checkbox, section_title, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, field_label, new_input, paragraph, path_field, text_default};

pub struct ImportPlayniteDialog {
    ctx: DialogCtx,
    path: Entity<InputState>,
    preview: Option<PlayniteImportPreview>,
    drives: Vec<(String, Entity<InputState>)>,
    /// Drive map saved by the previous import (prefills the rows).
    saved_map: HashMap<String, String>,
    fetch_covers: bool,
    merge_existing: bool,
    loading: bool,
    importing: bool,
    result: Option<PlayniteImportResult>,
    error: Option<SharedString>,
}

impl EventEmitter<DialogEvent> for ImportPlayniteDialog {}

/// A [`button`] that reacts only when `enabled`; the disabled look is Heroic's
/// `.button:disabled` (no background, secondary text, no hover).
pub(super) fn action(
    id: &'static str,
    kind: ButtonKind,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    if enabled {
        return button(id, kind, label).on_click(on_click).into_any_element();
    }
    let label: SharedString = label.into();
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .px(px(10.))
        .py(px(if kind == ButtonKind::Outline { 8. } else { 7. }))
        .cursor_not_allowed()
        .text_size(px(13.33))
        .text_color(white())
        .child(label.to_uppercase())
        .into_any_element()
}

/// Disc-less bullet list shared by the Playnite dialogs.
pub(super) fn bullet_list(items: Vec<String>) -> Div {
    let mut list = div().flex().flex_col().pl(px(8.));
    for item in items {
        list = list.child(
            div()
                .flex()
                .gap(px(8.))
                .text_size(px(16.))
                .line_height(px(22.))
                .text_color(text_default())
                .child(div().flex_none().child("\u{2022}"))
                .child(div().min_w_0().child(item)),
        );
    }
    list
}

impl ImportPlayniteDialog {
    pub fn new(ctx: DialogCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Tiny reads of the saved import context (path + drive map).
        let pn = Playnite::new(ctx.heroic.clone());
        let last = pn.last_library_path().ok().flatten().unwrap_or_default();
        let saved_map = pn
            .last_drive_map()
            .unwrap_or_default()
            .into_iter()
            .map(|d| (d.windows_root, d.linux_path))
            .collect();
        let path = new_input(window, cx, &last, "");
        cx.subscribe(&path, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let mut this = Self {
            ctx,
            path,
            preview: None,
            drives: Vec::new(),
            saved_map,
            fetch_covers: true,
            merge_existing: false,
            loading: false,
            importing: false,
            result: None,
            error: None,
        };
        // Screenshot helper: `HERONITE_DEBUG_PLAYNITE=preview|import` runs the steps.
        if std::env::var("HERONITE_DEBUG_PLAYNITE").is_ok() {
            this.run_preview(window, cx);
        }
        this
    }

    fn path_value(&self, cx: &App) -> String {
        self.path.read(cx).value().trim().to_string()
    }

    fn remaps(&self, cx: &App) -> Vec<DriveRemap> {
        self.drives
            .iter()
            .filter_map(|(root, input)| {
                let linux = input.read(cx).value().trim().to_string();
                (!linux.is_empty()).then(|| DriveRemap { windows_root: root.clone(), linux_path: linux })
            })
            .collect()
    }

    fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path.clone();
        choose_path(window, cx, true, "Choose", move |_, p, window, cx| {
            path.update(cx, |s, cx| s.set_value(p, window, cx));
        });
    }

    fn handle_icon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path_value(cx).is_empty() {
            self.pick_folder(window, cx);
        } else {
            self.path.update(cx, |s, cx| s.set_value("", window, cx));
            self.preview = None;
            self.drives.clear();
            cx.notify();
        }
    }

    fn run_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.path_value(cx);
        if library.is_empty() || self.loading {
            return;
        }
        self.error = None;
        self.result = None;
        self.loading = true;
        let heroic = self.ctx.heroic.clone();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_executor().spawn(async move { Playnite::new(heroic).preview_import(&library) }).await;
            this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match res {
                    Ok(preview) => {
                        // Keep what the user already typed for known drives.
                        let mut typed: HashMap<String, String> = this
                            .drives
                            .iter()
                            .map(|(root, input)| (root.clone(), input.read(cx).value().to_string()))
                            .collect();
                        this.drives = preview
                            .detected_drives
                            .iter()
                            .map(|root| {
                                let value = typed
                                    .remove(root)
                                    .filter(|v| !v.is_empty())
                                    .or_else(|| this.saved_map.get(root).cloned())
                                    .unwrap_or_default();
                                (root.clone(), new_input(window, cx, &value, "/run/media/..."))
                            })
                            .collect();
                        if !preview.errors.is_empty() {
                            this.error = Some(preview.errors.join("\n").into());
                        }
                        this.preview = Some(preview);
                        if std::env::var("HERONITE_DEBUG_PLAYNITE").as_deref() == Ok("import") {
                            this.run_import(window, cx);
                        }
                    }
                    Err(err) => {
                        this.preview = None;
                        this.error = Some(format!("{err:#}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn run_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = &self.preview else { return };
        if self.loading {
            return;
        }
        let args = PlayniteImportArgs {
            library_path: preview.library_path.clone(),
            drive_map: self.remaps(cx),
            fetch_covers: Some(self.fetch_covers),
            merge_existing: self.merge_existing,
        };
        self.error = None;
        self.loading = true;
        self.importing = true;
        let heroic = self.ctx.heroic.clone();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_executor().spawn(async move { Playnite::new(heroic).import(&args) }).await;
            this.update_in(cx, |this, _, cx| {
                this.loading = false;
                this.importing = false;
                match res {
                    Ok(result) => {
                        this.result = Some(result);
                        // The Collection reloads now; the dialog stays open
                        // to show the summary.
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn count_line(label: &str, n: usize) -> String {
    format!("{label}: {n}")
}

impl Render for ImportPlayniteDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_path = !self.path_value(cx).is_empty();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(paragraph(
                "Select the Playnite library folder (the one that contains games.db). Steam, emulators, and manual games \
                 become Local titles. Epic and GOG playtime is matched onto games already in Heroic. Covers are fetched \
                 from Steam or SteamGridDB.",
            ))
            .child(path_field(
                "pn-path-icon",
                Some("Playnite library folder"),
                &self.path,
                has_path,
                self.loading,
                cx.listener(|this, _, w, cx| this.handle_icon(w, cx)),
            ));

        if !self.drives.is_empty() {
            let mut drives = div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(section_title("Windows drive mapping (optional)"));
            for (root, input) in &self.drives {
                drives = drives.child(crate::ui::fields::text_field(root, input));
            }
            body = body.child(drives);
        }

        body = body
            .child(
                checkbox("pn-covers", self.fetch_covers, "Download covers from Steam / SteamGridDB")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fetch_covers = !this.fetch_covers;
                        cx.notify();
                    })),
            )
            .child(
                checkbox("pn-merge", self.merge_existing, "Merge into games already imported (keep my status and edits)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.merge_existing = !this.merge_existing;
                        cx.notify();
                    })),
            );

        if let Some(p) = &self.preview {
            if p.errors.is_empty() {
                body = body.child(bullet_list(vec![
                    count_line("Games", p.game_count),
                    count_line("Session files", p.session_file_count),
                    count_line("Steam", p.counts.steam),
                    count_line("Emulators", p.counts.emulator),
                    count_line("Manual", p.counts.manual),
                    count_line("Epic playtime matches", p.counts.epic_match),
                    count_line("GOG playtime matches", p.counts.gog_match),
                    count_line("Unmatched store games", p.counts.skipped_store),
                ]));
            }
        }
        if let Some(r) = &self.result {
            body = body.child(div().text_size(px(16.)).text_color(theme::success()).child(format!(
                "Imported {}, updated {}, matched store playtime {}, skipped {}, covers {}, sessions {}.",
                r.imported, r.updated, r.matched_store, r.skipped, r.covers_fetched, r.sessions_imported
            )));
            if !r.errors.is_empty() {
                body = body.child(
                    div().text_size(px(13.33)).text_color(theme::danger()).child(
                        r.errors.iter().take(6).cloned().collect::<Vec<_>>().join("\n"),
                    ),
                );
            }
        }
        if let Some(e) = self.error.clone() {
            body = body.child(div().text_size(px(16.)).text_color(theme::danger()).child(e));
        }
        if self.loading && !self.importing {
            body = body.child(field_label("Reading Playnite library\u{2026}"));
        }

        let done = self.result.is_some();
        let busy = self.loading;
        let footer = div()
            .flex()
            .gap(px(16.))
            .child(action(
                "pn-preview",
                ButtonKind::Outline,
                "Preview",
                !busy && has_path,
                cx.listener(|this, _, w, cx| this.run_preview(w, cx)),
            ))
            .child(if done {
                action("pn-close", ButtonKind::Primary, "Close", true, cx.listener(|_, _, _, cx| {
                    cx.emit(DialogEvent::ChangedAndClose)
                }))
            } else {
                action(
                    "pn-import",
                    ButtonKind::Primary,
                    if self.importing { "Working\u{2026}" } else { "Import" },
                    !busy && self.preview.as_ref().is_some_and(|p| p.errors.is_empty()),
                    cx.listener(|this, _, w, cx| this.run_import(w, cx)),
                )
            })
            .into_any_element();

        let entity = cx.entity().downgrade();
        let vw = f32::from(window.viewport_size().width);
        dialog(
            DialogProps {
                id: "import-playnite".into(),
                title: "Import Playnite library".into(),
                width: (640f32).min(vw * 0.9) + 64.,
                height: None,
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity
                    .update(cx, |this, cx| {
                        cx.emit(if this.result.is_some() { DialogEvent::ChangedAndClose } else { DialogEvent::Close })
                    })
                    .ok();
            },
        )
    }
}
