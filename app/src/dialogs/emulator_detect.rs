//! "Detect emulators": what is installed on this PC (PATH, Flatpak, AppImages
//! and binaries in the usual folders) plus emulators found by searching a
//! folder the user picks. Pick which ones to add; they get the catalog
//! profiles. Used by the Emulators page.

mod console;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_emulation::discover::{add_discovered, discover_emulators, discover_in_folder};
use heronite_emulation::{list_user_emulators, DetectedEmulator, SystemEnv};

use crate::ui::controls::mini_checkbox;
use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, meta_text, paragraph};

struct Row {
    item: DetectedEmulator,
    selected: bool,
}

pub struct DetectEmulatorsDialog {
    paths: DataPaths,
    rows: Vec<Row>,
    /// The automatic search is running.
    scanning: bool,
    /// Folder being searched.
    folder_scanning: Option<String>,
    folders_done: Vec<String>,
    adding: bool,
    error: String,
    /// Emulators added (read by the page once the dialog closes).
    pub added: usize,
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for DetectEmulatorsDialog {}

/// Appends `items` that are not listed yet. New not-yet-configured ones are
/// ticked, except further versions of an emulator that is already ticked.
fn merge_rows(rows: &mut Vec<Row>, items: Vec<DetectedEmulator>) {
    for item in items {
        if rows.iter().any(|r| r.item.executable == item.executable && r.item.flatpak_id == item.flatpak_id) {
            continue;
        }
        let taken = rows.iter().any(|r| r.selected && r.item.definition_id == item.definition_id && r.item.name == item.name);
        let selected = !item.already_added && !taken;
        rows.push(Row { item, selected });
    }
}

impl DetectEmulatorsDialog {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let p = paths.clone();
        let find = move || {
            let existing = list_user_emulators(&p)?;
            anyhow::Ok(discover_emulators(&SystemEnv, &existing))
        };
        let mut this = Self {
            paths,
            rows: vec![],
            scanning: true,
            folder_scanning: None,
            folders_done: vec![],
            adding: false,
            error: String::new(),
            added: 0,
            console_list: Default::default(),
            layer: Default::default(),
        };
        // Screenshots: the hidden window does not repaint late results.
        if crate::env::flag("HERONITE_DEBUG_SYNC") {
            this.scanning = false;
            match find() {
                Ok(items) => merge_rows(&mut this.rows, items),
                Err(e) => this.error = format!("{e:#}"),
            }
            return this;
        }
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { find() }).await;
            this.update(cx, |this, cx| {
                this.scanning = false;
                match found {
                    Ok(items) => merge_rows(&mut this.rows, items),
                    Err(e) => this.error = format!("{e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        this
    }

    /// Opens the folder chooser; the folder picked is searched for emulators.
    pub fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(window, cx, true, "Folder with emulators", |this: &mut Self, p, _, cx| this.scan_folder(p, cx));
    }

    fn scan_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        self.folder_scanning = Some(folder.clone());
        self.error.clear();
        let (p, f) = (self.paths.clone(), folder.clone());
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    let existing = list_user_emulators(&p)?;
                    anyhow::Ok(discover_in_folder(&SystemEnv, &f, &existing))
                })
                .await;
            this.update(cx, |this, cx| {
                this.folder_scanning = None;
                match found {
                    Ok(items) => {
                        let before = this.rows.len();
                        merge_rows(&mut this.rows, items);
                        if this.rows.len() == before {
                            this.error = format!("No new emulators found in {folder}.");
                        }
                        this.folders_done.push(folder);
                    }
                    Err(e) => this.error = format!("{e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn selected_count(&self) -> usize {
        self.rows.iter().filter(|r| r.selected && !r.item.already_added).count()
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let items: Vec<DetectedEmulator> =
            self.rows.iter().filter(|r| r.selected && !r.item.already_added).map(|r| r.item.clone()).collect();
        if items.is_empty() || self.adding {
            return;
        }
        self.adding = true;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { add_discovered(&paths, &items) }).await;
            this.update(cx, |this, cx| {
                this.adding = false;
                match res {
                    Ok(added) => {
                        this.added = added.len();
                        cx.emit(DialogEvent::ChangedAndClose);
                    }
                    Err(e) => this.error = format!("{e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

fn profiles_summary(item: &DetectedEmulator) -> String {
    let names: Vec<&str> = item.profiles.iter().map(|p| p.platform_name.as_str()).collect();
    if names.len() <= 4 {
        names.join(" \u{b7} ")
    } else {
        format!("{} \u{b7} +{} more", names[..4].join(" \u{b7} "), names.len() - 4)
    }
}

impl Render for DetectEmulatorsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console();
        }
        let busy = self.scanning || self.folder_scanning.is_some() || self.adding;
        let selected = self.selected_count();

        let mut list = div().id("detect-list").flex().flex_col().gap(px(2.)).max_h(px(400.)).overflow_y_scroll();
        for (i, r) in self.rows.iter().enumerate() {
            let added = r.item.already_added;
            let loc = if r.item.launch_kind == "flatpak" {
                format!("Flatpak \u{b7} {}", r.item.flatpak_id.clone().unwrap_or_default())
            } else {
                r.item.executable.clone()
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("detect-row-{i}")))
                    .flex()
                    .gap(px(12.))
                    .items_start()
                    .px(px(8.))
                    .py(px(8.))
                    .rounded(px(theme::R_ITEM))
                    .when(added, |d| d.opacity(0.6))
                    .when(!added, |d| {
                        d.cursor_pointer().hover(|s| s.bg(theme::menu_item_hover())).on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(r) = this.rows.get_mut(i) {
                                r.selected = !r.selected;
                            }
                            cx.notify();
                        }))
                    })
                    .child(mini_checkbox(SharedString::from(format!("detect-cb-{i}")), r.selected && !added, added))
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
                                    .child(div().font_weight(theme::W_BOLD).text_size(px(14.)).text_color(theme::text_primary()).child(r.item.name.clone()))
                                    .child(crate::pages::tag_pill(if added { "Already added".to_string() } else { r.item.source.clone() })),
                            )
                            .child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(loc))
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(profiles_summary(&r.item))),
                    ),
            );
        }

        let status = if self.scanning {
            "Looking in PATH, Flatpak and the usual folders\u{2026}".to_string()
        } else if let Some(f) = &self.folder_scanning {
            format!("Searching {f}\u{2026}")
        } else if self.rows.is_empty() {
            "No emulators found automatically. Scan a folder that contains them (for example your Emulation folder).".to_string()
        } else {
            format!(
                "{} found, {} not added yet.",
                self.rows.len(),
                self.rows.iter().filter(|r| !r.item.already_added).count()
            )
        };

        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph(
                "Emulators found on this PC. Tick the ones to add; each starts with the platforms of its catalog entry.",
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(action_button(
                        "detect-folder",
                        ButtonKind::Glass,
                        if self.folder_scanning.is_some() { "Searching\u{2026}" } else { "Scan a folder\u{2026}" },
                        !busy,
                        cx.listener(|this, _, window, cx| this.pick_folder(window, cx)),
                    ))
                    .child(button("detect-all", ButtonKind::Glass, "Select all").on_click(cx.listener(|this, _, _, cx| {
                        for r in this.rows.iter_mut().filter(|r| !r.item.already_added) {
                            r.selected = true;
                        }
                        cx.notify();
                    })))
                    .child(button("detect-none", ButtonKind::Glass, "Select none").on_click(cx.listener(|this, _, _, cx| {
                        for r in &mut this.rows {
                            r.selected = false;
                        }
                        cx.notify();
                    }))),
            )
            .child(meta_text(status))
            .children((!self.rows.is_empty()).then(|| crate::ui::controls::card_box().py(px(6.)).child(list)))
            .children((!self.error.is_empty()).then(|| div().text_color(theme::danger_text()).text_size(px(13.33)).child(self.error.clone())));

        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("detect-close", ButtonKind::Ghost, "Close").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "detect-add",
                ButtonKind::Primary,
                if self.adding { "Adding\u{2026}".to_string() } else { format!("Add {selected} emulator(s)") },
                selected > 0 && !busy,
                cx.listener(|this, _, _, cx| this.add(cx)),
            ))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "detect-emulators".into(),
                title: "Detect emulators".into(),
                width: 720f32.min(f32::from(window.viewport_size().width) * 0.9) + 64.,
                height: None,
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_rows, Row};
    use heronite_emulation::{DetectedEmulator, DetectedEmulatorProfile};

    fn item(name: &str, exe: &str, added: bool) -> DetectedEmulator {
        DetectedEmulator {
            definition_id: name.to_lowercase(),
            name: name.into(),
            executable: exe.into(),
            launch_kind: "native".into(),
            flatpak_id: None,
            profiles: vec![DetectedEmulatorProfile::default()],
            already_added: added,
            source: String::new(),
        }
    }

    #[test]
    fn merging_ticks_new_emulators_once_per_kind() {
        let mut rows: Vec<Row> = vec![];
        merge_rows(
            &mut rows,
            vec![item("PCSX2", "/a/pcsx2-2.8", false), item("PCSX2", "/a/pcsx2-2.7", false), item("Dolphin", "/d", true)],
        );
        let ticks: Vec<bool> = rows.iter().map(|r| r.selected).collect();
        assert_eq!(ticks, [true, false, false]);
        // Already listed executables are not duplicated.
        merge_rows(&mut rows, vec![item("PCSX2", "/a/pcsx2-2.8", false), item("RPCS3", "/r", false)]);
        assert_eq!(rows.len(), 4);
        assert!(rows[3].selected);
    }
}
