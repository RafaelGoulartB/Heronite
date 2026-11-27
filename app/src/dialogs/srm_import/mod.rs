//! "Import from Steam ROM Manager": reads SRM's `userConfigurations.json`
//! (read-only), lists its enabled Glob parsers, and imports the selected ones
//! as Heronite emulators and ROM folders. Used by the Emulators page.

mod console;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_emulation::srm::{find_config_file, import_srm_parsers, load_config, SrmImportResult, SrmParserItem, SrmScanResult};
use heronite_emulation::{list_rom_scanners, SystemEnv};
use heronite_i18n::{tr, trn};

use super::{text_of, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, mini_checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, meta_text, new_input, on_commit, panel, paragraph, path_field};

pub struct SrmImportDialog {
    paths: DataPaths,
    config_path: Entity<InputState>,
    pending_path: Option<String>,
    last_loaded_path: String,
    load_gen: u64,
    scan: Option<SrmScanResult>,
    parsers: Vec<SrmParserItem>,
    scan_now: bool,
    loading: bool,
    importing: bool,
    error: String,
    pub outcome: Option<SrmImportResult>,
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for SrmImportDialog {}

impl SrmImportDialog {
    pub fn new(paths: DataPaths, initial_path: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = if cfg!(windows) {
            r"%APPDATA%\steam-rom-manager\userData\userConfigurations.json"
        } else {
            "~/.config/steam-rom-manager/userData/userConfigurations.json"
        };
        let config_path = new_input(window, cx, initial_path.as_deref().unwrap_or(""), placeholder);
        on_commit(&config_path, window, cx, |this: &mut Self, _, _, cx| {
            let p = text_of(&this.config_path, cx);
            if !p.is_empty() && p.trim() != this.last_loaded_path.trim() {
                this.load_file(p, cx);
            }
        });
        let mut this = Self {
            paths: paths.clone(),
            config_path,
            pending_path: None,
            last_loaded_path: initial_path.clone().unwrap_or_default(),
            load_gen: 1,
            scan: None,
            parsers: vec![],
            scan_now: true,
            loading: true,
            importing: false,
            error: String::new(),
            outcome: None,
            console_list: Default::default(),
            layer: Default::default(),
        };

        let find = move || -> anyhow::Result<Option<SrmScanResult>> {
            let path = initial_path.filter(|p| !p.trim().is_empty()).or_else(|| find_config_file(&SystemEnv));
            let Some(path) = path else {
                return Ok(None);
            };
            let scanners = list_rom_scanners(&paths)?;
            Ok(Some(load_config(&path, &scanners)?))
        };

        if crate::env::flag("HERONITE_DEBUG_SYNC") {
            this.loading = false;
            match find() {
                Ok(Some(res)) => this.apply_scan(res),
                Ok(None) => {}
                Err(e) => this.error = format!("{e:#}"),
            }
            return this;
        }

        let gen = this.load_gen;
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { find() }).await;
            this.update(cx, |this, cx| {
                if this.load_gen != gen {
                    return;
                }
                this.loading = false;
                match found {
                    Ok(Some(res)) => this.apply_scan(res),
                    Ok(None) => {}
                    Err(e) => this.error = format!("{e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        this
    }

    fn apply_scan(&mut self, res: SrmScanResult) {
        self.last_loaded_path = res.config_path.clone();
        self.pending_path = Some(res.config_path.clone());
        self.parsers = res.parsers.clone();
        self.error = res.errors.join("\n");
        self.scan = Some(res);
    }

    pub(super) fn load_file(&mut self, path: String, cx: &mut Context<Self>) {
        if path.trim().is_empty() {
            return;
        }
        self.load_gen += 1;
        let gen = self.load_gen;
        self.last_loaded_path = path.trim().to_string();
        self.loading = true;
        self.error.clear();
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let scanners = list_rom_scanners(&paths)?;
                    load_config(&path, &scanners)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.load_gen != gen {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(scan) => this.apply_scan(scan),
                    Err(e) => {
                        this.scan = None;
                        this.parsers.clear();
                        this.error = format!("{e:#}");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn selected_count(&self) -> usize {
        self.parsers.iter().filter(|p| p.selected).count()
    }

    pub(super) fn import(&mut self, cx: &mut Context<Self>) {
        if self.importing || self.selected_count() == 0 {
            return;
        }
        self.importing = true;
        self.error.clear();
        let (paths, items, scan_now) = (self.paths.clone(), self.parsers.clone(), self.scan_now);
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { import_srm_parsers(&paths, &items, scan_now) }).await;
            this.update(cx, |this, cx| {
                this.importing = false;
                match res {
                    Ok(out) => {
                        this.outcome = Some(out);
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

    fn status_text(&self) -> String {
        if self.loading {
            return tr!("Looking for Steam ROM Manager configuration\u{2026}").to_string();
        }
        let Some(scan) = &self.scan else {
            return tr!(
                "Steam ROM Manager configuration was not found automatically. Choose userConfigurations.json to import its parsers."
            )
            .to_string();
        };
        if self.parsers.is_empty() {
            return tr!("No enabled Glob parsers were found in this file.").to_string();
        }
        let mut parts = vec![trn!(self.parsers.len(), "{n} Glob parser found", "{n} Glob parsers found")];
        if scan.disabled_count > 0 {
            parts.push(trn!(scan.disabled_count, "{n} disabled parser skipped", "{n} disabled parsers skipped"));
        }
        if scan.unsupported_count > 0 {
            parts.push(trn!(scan.unsupported_count, "{n} non-Glob parser skipped", "{n} non-Glob parsers skipped"));
        }
        parts.join(" \u{b7} ")
    }

    fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let mut list = div().id("srm-list").flex().flex_col().gap(px(2.)).max_h(px(340.)).overflow_y_scroll();
        for (i, item) in self.parsers.iter().enumerate() {
            let configured = item.already_configured;
            list = list.child(
                div()
                    .id(SharedString::from(format!("srm-row-{i}")))
                    .flex()
                    .gap(px(12.))
                    .items_start()
                    .px(px(8.))
                    .py(px(8.))
                    .rounded(px(theme::R_ITEM))
                    .when(configured, |d| d.opacity(0.65))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::menu_item_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(p) = this.parsers.get_mut(i) {
                            p.selected = !p.selected;
                        }
                        cx.notify();
                    }))
                    .child(mini_checkbox(SharedString::from(format!("srm-cb-{i}")), item.selected, false))
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
                                    .child(
                                        div()
                                            .font_weight(theme::W_BOLD)
                                            .text_size(px(14.))
                                            .text_color(theme::text_primary())
                                            .truncate()
                                            .child(item.title.clone()),
                                    )
                                    .child(crate::pages::tag_pill(if configured {
                                        tr!("Already configured").to_string()
                                    } else {
                                        item.emulator_name.clone()
                                    })),
                            )
                            .child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(item.rom_directory.clone()))
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(item.rom_glob.clone())),
                    ),
            );
        }
        crate::ui::controls::card_box().py(px(6.)).child(list)
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .child(button("srm-all", ButtonKind::Glass, tr!("Select all")).on_click(cx.listener(|this, _, _, cx| {
                for p in &mut this.parsers {
                    p.selected = true;
                }
                cx.notify();
            })))
            .child(button("srm-none", ButtonKind::Glass, tr!("Select none")).on_click(cx.listener(|this, _, _, cx| {
                for p in &mut this.parsers {
                    p.selected = false;
                }
                cx.notify();
            })))
    }

    fn render_footer(&self, selected: usize, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .gap(px(10.))
            .child(button("srm-close", ButtonKind::Ghost, tr!("Close")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "srm-import",
                ButtonKind::Primary,
                if self.importing {
                    tr!("Importing\u{2026}").to_string()
                } else {
                    trn!(selected, "Import {n} parser", "Import {n} parsers")
                },
                selected > 0 && !busy,
                cx.listener(|this, _, _, cx| this.import(cx)),
            ))
            .into_any_element()
    }
}

impl Render for SrmImportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(p) = self.pending_path.take() {
            self.config_path.update(cx, |s, cx| s.set_value(p, window, cx));
        }
        if crate::console::active() {
            return self.render_console(cx);
        }
        let busy = self.loading || self.importing;
        let selected = self.selected_count();
        let path_val = text_of(&self.config_path, cx);
        let has_path = !path_val.is_empty();

        let picker = path_field(
            "srm-path-pick",
            Some(tr!("Steam ROM Manager config (userConfigurations.json)")),
            &self.config_path,
            has_path,
            false,
            cx.listener(move |this, _, window, cx| {
                if has_path {
                    this.load_gen += 1;
                    this.loading = false;
                    this.last_loaded_path.clear();
                    this.pending_path = None;
                    this.config_path.update(cx, |s, cx| s.set_value("", window, cx));
                    this.scan = None;
                    this.parsers.clear();
                    this.error.clear();
                    cx.notify();
                } else {
                    choose_path(window, cx, false, tr!("Steam ROM Manager config"), |this: &mut Self, p, window, cx| {
                        this.config_path.update(cx, |s, cx| s.set_value(p.clone(), window, cx));
                        this.load_file(p, cx);
                    });
                }
            }),
        );

        let scan_toggle = panel().child(checkbox("srm-scan-now", self.scan_now, tr!("Scan imported folders right away")).on_click(
            cx.listener(|this, _, _, cx| {
                this.scan_now = !this.scan_now;
                cx.notify();
            }),
        ));

        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph(tr!(
                "Import emulator paths, ROM folders and glob patterns from Steam ROM Manager. Your Steam shortcuts are left untouched."
            )))
            .child(picker)
            .child(meta_text(self.status_text()))
            .children((!self.parsers.is_empty()).then(|| self.render_controls(cx)))
            .children((!self.parsers.is_empty()).then(|| self.render_list(cx)))
            .children((!self.parsers.is_empty()).then_some(scan_toggle))
            .children(
                (!self.error.is_empty()).then(|| div().text_color(theme::danger_text()).text_size(px(13.33)).child(self.error.clone())),
            );

        let footer = self.render_footer(selected, busy, cx);
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "srm-import".into(),
                title: tr!("Import from Steam ROM Manager").into(),
                width: 720f32.min(f32::from(crate::ui::content_size(window).width) * 0.9) + 64.,
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
