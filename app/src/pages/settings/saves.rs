//! Settings → Backups, first card: Ludusavi after-play save backups.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::EffectiveSettings;
use heronite_emulation::ludusavi::{detect_ludusavi, LudusaviDetected, SystemRunner};
use heronite_emulation::settings_ext::{set_ludusavi_settings, LudusaviSettingsPatch};
use heronite_launch::SystemProbe;

use super::backup::format_time;
use super::SettingsPage;
use crate::pages::PageCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{caption, card, card_box, help_text, toggle_row};
use crate::ui::fields::{choose_path, meta_text, new_input, on_commit, path_field, select_field};
use crate::ui::icon;

const FORMATS: [(&str, &str); 2] = [("zip", "Zip"), ("simple", "Simple folder")];
const COMPRESSIONS: [(&str, &str); 4] = [("none", "None"), ("deflate", "Deflate"), ("bzip2", "Bzip2"), ("zstd", "Zstd")];

pub struct State {
    /// `None` while detecting, `Some(None)` when Ludusavi was not found.
    detected: Option<Option<LudusaviDetected>>,
    detecting: bool,
    binary: Entity<InputState>,
    folder: Entity<InputState>,
    details_open: Option<bool>,
    format_open: bool,
    compression_open: bool,
}

impl State {
    pub fn new(
        _ctx: &PageCtx,
        s: &EffectiveSettings,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Self {
        let binary = new_input(window, cx, &s.ludusavi.binary_path, "");
        let folder = new_input(window, cx, &s.ludusavi.backup_path, "");
        on_commit(&binary, window, cx, |d: &mut SettingsPage, v, window, cx| {
            if v != d.settings.ludusavi.binary_path {
                save(d, LudusaviSettingsPatch { binary_path: Some(v), ..Default::default() }, cx);
                detect(d, window, cx);
            }
        });
        on_commit(&folder, window, cx, |d: &mut SettingsPage, v, _, cx| {
            if v != d.settings.ludusavi.backup_path {
                save(d, LudusaviSettingsPatch { backup_path: Some(v), ..Default::default() }, cx);
            }
        });
        Self { detected: None, detecting: false, binary, folder, details_open: None, format_open: false, compression_open: false }
    }

    /// Section shown: looks for Ludusavi once (it runs `ludusavi --version`,
    /// so never at page creation).
    pub fn shown(page: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
        if page.saves.detected.is_none() && !page.saves.detecting {
            detect(page, window, cx);
        }
    }
}

fn detect(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    d.saves.detected = None;
    d.saves.detecting = true;
    detect_with(d.settings.ludusavi.binary_path.clone(), window, cx);
}

/// Runs `ludusavi --version` / `config path` off the UI thread.
fn detect_with(preferred: String, window: &mut Window, cx: &mut Context<SettingsPage>) {
    cx.spawn_in(window, async move |this, cx| {
        let found = cx
            .background_spawn(async move {
                detect_ludusavi(&SystemRunner::default(), &SystemProbe, Some(&preferred).filter(|p| !p.is_empty()).map(String::as_str))
            })
            .await;
        this.update_in(cx, |d, window, cx| {
            if let Some(f) = &found {
                let (binary, folder) = (f.binary.clone(), f.backup_path.clone().unwrap_or_default());
                d.saves.binary.update(cx, |s, cx| s.set_placeholder(binary, window, cx));
                d.saves.folder.update(cx, |s, cx| s.set_placeholder(folder, window, cx));
            }
            d.saves.detected = Some(found);
            d.saves.detecting = false;
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn save(d: &mut SettingsPage, patch: LudusaviSettingsPatch, cx: &mut Context<SettingsPage>) {
    let res = set_ludusavi_settings(&d.ctx.heroic, &patch);
    d.saved(res, cx);
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let l = d.settings.ludusavi.clone();
    let detected = d.saves.detected.clone();
    let config_known = detected.as_ref().and_then(|x| x.as_ref()).map(|x| x.config_path.is_some()).unwrap_or(false);
    let locked = l.use_installed_config && config_known;
    let details_open = d.saves.details_open.unwrap_or(!locked);

    let callout: AnyElement = match &detected {
        None => card_box().py(px(12.)).child(help_text("Looking for Ludusavi…")).into_any_element(),
        Some(Some(f)) => {
            let compression = if f.format.as_deref() == Some("zip") {
                f.compression.as_ref().map(|c| format!(" / {c}")).unwrap_or_default()
            } else {
                String::new()
            };
            card_box()
                .py(px(12.))
                .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(format!(
                    "Found Ludusavi {} at {}. Backups go to {} ({}{}).",
                    f.version.clone().unwrap_or_default(),
                    f.binary,
                    f.backup_path.clone().unwrap_or_else(|| "unknown".into()),
                    f.format.clone().unwrap_or_else(|| "simple".into()),
                    compression
                )))
                .into_any_element()
        }
        Some(None) => card_box()
            .py(px(12.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::danger_text())
                    .child("Ludusavi was not found automatically. Set the binary and backup folder below."),
            )
            .into_any_element(),
    };

    let (has_bin, has_dir) = (!l.binary_path.is_empty(), !l.backup_path.is_empty());
    let binary = path_field(
        "ludusavi-binary-pick",
        Some("Ludusavi binary"),
        &d.saves.binary,
        has_bin,
        false,
        cx.listener(move |d, _, window, cx| {
            if has_bin {
                d.saves.binary.update(cx, |s, cx| s.set_value("", window, cx));
                save(d, LudusaviSettingsPatch { binary_path: Some(String::new()), ..Default::default() }, cx);
                detect(d, window, cx);
            } else {
                choose_path(window, cx, false, "Ludusavi binary", |d: &mut SettingsPage, p, window, cx| {
                    d.saves.binary.update(cx, |s, cx| s.set_value(p.clone(), window, cx));
                    save(d, LudusaviSettingsPatch { binary_path: Some(p), ..Default::default() }, cx);
                    detect(d, window, cx);
                });
            }
        }),
    );
    let folder = path_field(
        "ludusavi-folder-pick",
        Some("Save backup folder"),
        &d.saves.folder,
        has_dir,
        locked,
        cx.listener(move |d, _, window, cx| {
            if has_dir {
                d.saves.folder.update(cx, |s, cx| s.set_value("", window, cx));
                save(d, LudusaviSettingsPatch { backup_path: Some(String::new()), ..Default::default() }, cx);
            } else {
                choose_path(window, cx, true, "Save backup folder", |d: &mut SettingsPage, p, window, cx| {
                    d.saves.folder.update(cx, |s, cx| s.set_value(p.clone(), window, cx));
                    save(d, LudusaviSettingsPatch { backup_path: Some(p), ..Default::default() }, cx);
                });
            }
        }),
    );
    let label_of = |table: &[(&'static str, &'static str)], v: &str| {
        table.iter().find(|(k, _)| *k == v).map(|(_, l)| *l).unwrap_or("").to_string()
    };
    let format = select_field(
        "ludusavi-format",
        Some("Backup format"),
        label_of(&FORMATS, &l.format),
        locked,
        d.saves.format_open,
        FORMATS.iter().map(|(k, lbl)| ((*lbl).into(), *k == l.format)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.saves.format_open = !d.saves.format_open;
            d.saves.compression_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.saves.format_open = false;
            save(d, LudusaviSettingsPatch { format: Some(FORMATS[i].0.into()), ..Default::default() }, cx);
        },
    );
    let compression = select_field(
        "ludusavi-compression",
        Some("Zip compression"),
        label_of(&COMPRESSIONS, &l.compression),
        locked || l.format != "zip",
        d.saves.compression_open,
        COMPRESSIONS.iter().map(|(k, lbl)| ((*lbl).into(), *k == l.compression)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.saves.compression_open = !d.saves.compression_open;
            d.saves.format_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.saves.compression_open = false;
            save(d, LudusaviSettingsPatch { compression: Some(COMPRESSIONS[i].0.into()), ..Default::default() }, cx);
        },
    );

    let summary = div()
        .id("ludusavi-details")
        .flex()
        .items_center()
        .gap(px(6.))
        .mt(px(14.))
        .mx(px(6.))
        .mb(px(6.))
        .cursor_pointer()
        .on_click(cx.listener(move |d, _, _, cx| {
            d.saves.details_open = Some(!details_open);
            cx.notify();
        }))
        .child(icon(if details_open { Icon::ExpandMore } else { Icon::ChevronRight }, px(16.), theme::text_muted()))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child("Backup location & format"));

    let mut last = match &l.last_backup_at {
        Some(at) => format!("Last save backup: {}", format_time(at)),
        None => "Last save backup: Never".to_string(),
    };
    if let Some(g) = &l.last_backup_game {
        last.push_str(&format!(" · {g}"));
    }
    if let Some(p) = &l.last_backup_path {
        last.push_str(&format!(" · {p}"));
    }

    let enable = card([
        toggle_row("ludusavi-enabled", "Back up saves when a game closes", None, l.enabled)
            .on_click(cx.listener(move |d, _, _, cx| {
                let on = !d.settings.ludusavi.enabled;
                save(d, LudusaviSettingsPatch { enabled: Some(on), ..Default::default() }, cx);
            }))
            .into_any_element(),
        div()
            .when(!config_known, |d| d.opacity(0.5))
            .child(
                toggle_row(
                    "ludusavi-use-config",
                    "Use installed Ludusavi settings",
                    Some("Path, zip and compression come from Ludusavi's own configuration."),
                    l.use_installed_config,
                )
                .when(config_known, |c| {
                    c.on_click(cx.listener(move |d, _, _, cx| {
                        let on = !d.settings.ludusavi.use_installed_config;
                        save(d, LudusaviSettingsPatch { use_installed_config: Some(on), ..Default::default() }, cx);
                    }))
                }),
            )
            .into_any_element(),
    ]);

    div()
        .flex()
        .flex_col()
        .child(caption("Save backups (Ludusavi)"))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(
            "When enabled, Heroic Local backs up the game save with Ludusavi after you close a game. Right-click a Collection card to back up that game now.",
        )))
        .child(enable)
        .child(div().mt(px(10.)).child(callout))
        .child(summary)
        .children(details_open.then(|| {
            card_box().py(px(12.)).gap(px(12.)).child(binary).child(folder).child(format).child(compression)
        }))
        .child(div().mt(px(12.)).mx(px(6.)).child(meta_text(last)))
        .children(l.last_error.clone().map(|e| div().mt(px(8.)).mx(px(6.)).text_color(theme::danger_text()).text_size(px(13.)).child(e)))
        .into_any_element()
}

/// Console mode rows of Backups: Ludusavi, then the Collection backup.
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    use crate::icons::Icon;
    let l = &d.settings.ludusavi;
    let detected = d.saves.detected.clone();
    let config_known = detected.as_ref().and_then(|x| x.as_ref()).map(|x| x.config_path.is_some()).unwrap_or(false);
    let locked = l.use_installed_config && config_known;
    let found = match &detected {
        None => "Looking for Ludusavi\u{2026}".to_string(),
        Some(Some(f)) => format!("Found: {}", f.binary),
        Some(None) => "Ludusavi was not found".into(),
    };
    let mut rows = vec![
        Row::header("Save backups (Ludusavi)"),
        Row::toggle("lu-enabled", "Back up saves when a game closes", l.enabled).desc(found),
        Row::toggle("lu-config", "Use Ludusavi's own settings", l.use_installed_config).enabled(config_known).desc("Path, zip and compression come from Ludusavi's configuration"),
    ];
    if !locked {
        rows.push(Row::text("lu-binary", "Ludusavi program", d.saves.binary.read(cx).value().to_string()).icon(Icon::Description));
        rows.push(Row::text("lu-folder", "Backup folder", d.saves.folder.read(cx).value().to_string()).icon(Icon::FolderOpen));
        let fi = FORMATS.iter().position(|(k, _)| *k == l.format).unwrap_or(0);
        rows.push(Row::choice("lu-format", "Backup format", FORMATS.iter().map(|(_, x)| SharedString::from(*x)).collect(), fi));
        let ci = COMPRESSIONS.iter().position(|(k, _)| *k == l.compression).unwrap_or(0);
        rows.push(Row::choice("lu-compression", "Compression", COMPRESSIONS.iter().map(|(_, x)| SharedString::from(*x)).collect(), ci).enabled(l.format == "zip"));
    }
    if let Some(e) = &l.last_error {
        rows.push(Row::info("lu-error", e.clone(), "").danger());
    }
    rows.extend(super::backup::console_rows(d, cx));
    rows
}

pub(super) fn console_apply(d: &mut SettingsPage, outcome: crate::console::list::Outcome, window: &mut Window, cx: &mut Context<SettingsPage>) {
    use crate::console::browser::PickKind;
    use crate::console::list::{Outcome, Value};
    match &outcome {
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "lu-enabled" => save(d, LudusaviSettingsPatch { enabled: Some(*on), ..Default::default() }, cx),
            "lu-config" => save(d, LudusaviSettingsPatch { use_installed_config: Some(*on), ..Default::default() }, cx),
            _ => {}
        },
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "lu-format" => {
            save(d, LudusaviSettingsPatch { format: Some(FORMATS[*index].0.into()), ..Default::default() }, cx)
        }
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "lu-compression" => {
            save(d, LudusaviSettingsPatch { compression: Some(COMPRESSIONS[*index].0.into()), ..Default::default() }, cx)
        }
        Outcome::Activate(id) if id.as_ref() == "lu-binary" => {
            d.console_browse("Ludusavi program", PickKind::File(Vec::new()), None, window, cx, |d, path, window, cx| {
                d.saves.binary.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                save(d, LudusaviSettingsPatch { binary_path: Some(path), ..Default::default() }, cx);
                detect(d, window, cx);
            });
        }
        Outcome::Activate(id) if id.as_ref() == "lu-folder" => {
            let start = Some(std::path::PathBuf::from(d.saves.folder.read(cx).value().to_string()));
            d.console_browse("Ludusavi backup folder", PickKind::Folder, start, window, cx, |d, path, window, cx| {
                d.saves.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                save(d, LudusaviSettingsPatch { backup_path: Some(path), ..Default::default() }, cx);
            });
        }
        _ => super::backup::console_apply(d, outcome, window, cx),
    }
    cx.notify();
}
