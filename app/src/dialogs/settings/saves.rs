//! Settings tab: Save backup (Ludusavi after-play backups).

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::EffectiveSettings;
use heronite_emulation::ludusavi::{detect_ludusavi, LudusaviDetected, SystemRunner};
use heronite_emulation::settings_ext::{set_ludusavi_settings, LudusaviSettingsPatch};
use heronite_launch::SystemProbe;

use super::backup::format_time;
use super::SettingsDialog;
use crate::dialogs::DialogCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{checkbox, help_text};
use crate::ui::fields::{
    choose_path, new_input, on_commit, panel, paragraph_secondary, path_field, select_field,
};
use crate::ui::icon;

const FORMATS: [(&str, &str); 2] = [("zip", "Zip"), ("simple", "Simple folder")];
const COMPRESSIONS: [(&str, &str); 4] = [("none", "None"), ("deflate", "Deflate"), ("bzip2", "Bzip2"), ("zstd", "Zstd")];

pub struct State {
    /// `None` while detecting, `Some(None)` when Ludusavi was not found.
    detected: Option<Option<LudusaviDetected>>,
    binary: Entity<InputState>,
    folder: Entity<InputState>,
    details_open: Option<bool>,
    format_open: bool,
    compression_open: bool,
}

impl State {
    pub fn new(
        ctx: &DialogCtx,
        s: &EffectiveSettings,
        window: &mut Window,
        cx: &mut Context<SettingsDialog>,
    ) -> Self {
        let binary = new_input(window, cx, &s.ludusavi.binary_path, "");
        let folder = new_input(window, cx, &s.ludusavi.backup_path, "");
        on_commit(&binary, window, cx, |d: &mut SettingsDialog, v, window, cx| {
            if v != d.settings.ludusavi.binary_path {
                save(d, LudusaviSettingsPatch { binary_path: Some(v), ..Default::default() }, cx);
                detect(d, window, cx);
            }
        });
        on_commit(&folder, window, cx, |d: &mut SettingsDialog, v, _, cx| {
            if v != d.settings.ludusavi.backup_path {
                save(d, LudusaviSettingsPatch { backup_path: Some(v), ..Default::default() }, cx);
            }
        });
        let this = Self { detected: None, binary, folder, details_open: None, format_open: false, compression_open: false };
        detect_with(ctx.lib.settings.ludusavi.binary_path.clone(), window, cx);
        this
    }
}

fn detect(d: &mut SettingsDialog, window: &mut Window, cx: &mut Context<SettingsDialog>) {
    d.saves.detected = None;
    detect_with(d.settings.ludusavi.binary_path.clone(), window, cx);
}

/// Runs `ludusavi --version` / `config path` off the UI thread.
fn detect_with(preferred: String, window: &mut Window, cx: &mut Context<SettingsDialog>) {
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
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn save(d: &mut SettingsDialog, patch: LudusaviSettingsPatch, cx: &mut Context<SettingsDialog>) {
    let res = set_ludusavi_settings(&d.ctx.heroic, &patch);
    d.saved(res, cx);
}

pub fn render(d: &mut SettingsDialog, _window: &mut Window, cx: &mut Context<SettingsDialog>) -> AnyElement {
    let l = d.settings.ludusavi.clone();
    let detected = d.saves.detected.clone();
    let config_known = detected.as_ref().and_then(|x| x.as_ref()).map(|x| x.config_path.is_some()).unwrap_or(false);
    let locked = l.use_installed_config && config_known;
    let details_open = d.saves.details_open.unwrap_or(!locked);

    let callout: AnyElement = match &detected {
        None => panel().child(help_text("Looking for Ludusavi…")).into_any_element(),
        Some(Some(f)) => {
            let compression = if f.format.as_deref() == Some("zip") {
                f.compression.as_ref().map(|c| format!(" / {c}")).unwrap_or_default()
            } else {
                String::new()
            };
            panel()
                .py(px(6.))
                .child(div().text_size(px(13.33)).text_color(white()).child(format!(
                    "Found Ludusavi {} at {}. Backups go to {} ({}{}).",
                    f.version.clone().unwrap_or_default(),
                    f.binary,
                    f.backup_path.clone().unwrap_or_else(|| "unknown".into()),
                    f.format.clone().unwrap_or_else(|| "simple".into()),
                    compression
                )))
                .into_any_element()
        }
        Some(None) => div()
            .text_color(rgb(0xf0a202))
            .text_size(px(14.))
            .child("Ludusavi was not found automatically. Set the binary and backup folder below.")
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
                choose_path(window, cx, false, "Ludusavi binary", |d: &mut SettingsDialog, p, window, cx| {
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
                choose_path(window, cx, true, "Save backup folder", |d: &mut SettingsDialog, p, window, cx| {
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
        |d: &mut SettingsDialog, _, cx| {
            d.saves.format_open = !d.saves.format_open;
            d.saves.compression_open = false;
            cx.notify();
        },
        |d: &mut SettingsDialog, i, _, cx| {
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
        |d: &mut SettingsDialog, _, cx| {
            d.saves.compression_open = !d.saves.compression_open;
            d.saves.format_open = false;
            cx.notify();
        },
        |d: &mut SettingsDialog, i, _, cx| {
            d.saves.compression_open = false;
            save(d, LudusaviSettingsPatch { compression: Some(COMPRESSIONS[i].0.into()), ..Default::default() }, cx);
        },
    );

    let summary = div()
        .id("ludusavi-details")
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .on_click(cx.listener(move |d, _, _, cx| {
            d.saves.details_open = Some(!details_open);
            cx.notify();
        }))
        .child(
            icon(Icon::PlayArrow, px(22.), white()).with_transformation(Transformation::rotate(percentage(if details_open { 0.25 } else { 0. }))),
        )
        .child(div().text_size(px(19.2)).font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child("Backup location & format"));

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

    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(paragraph_secondary(
            "When enabled, Heroic Local backs up the game save with Ludusavi after you close a game. Right-click a Collection card to back up that game now.",
        ))
        .child(checkbox("ludusavi-enabled", l.enabled, "Back up saves when a game closes").on_click(cx.listener(
            move |d, _, _, cx| {
                let on = !d.settings.ludusavi.enabled;
                save(d, LudusaviSettingsPatch { enabled: Some(on), ..Default::default() }, cx);
            },
        )))
        .child(callout)
        .child(
            div()
                .when(!config_known, |d| d.opacity(0.5))
                .child(checkbox("ludusavi-use-config", l.use_installed_config, "Use installed Ludusavi settings (path, zip, compression)").when(config_known, |c| {
                    c.on_click(cx.listener(move |d, _, _, cx| {
                        let on = !d.settings.ludusavi.use_installed_config;
                        save(d, LudusaviSettingsPatch { use_installed_config: Some(on), ..Default::default() }, cx);
                    }))
                })),
        )
        .child(summary)
        .children(details_open.then(|| div().flex().flex_col().gap(px(12.)).child(binary).child(folder).child(format).child(compression)))
        .child(div().text_size(px(13.33)).text_color(white()).child(last))
        .children(l.last_error.clone().map(|e| div().text_color(theme::danger()).text_size(px(14.)).child(e)))
        .into_any_element()
}
