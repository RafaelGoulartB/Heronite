//! Settings → Advanced extras (below the panel's switches): folders, clear
//! cache, the image cache limit and "copy all settings".

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::{self, ClearReport};
use heronite_data::settings as store;
use heronite_data::{game_config, DataPaths};
use heronite_i18n::{tr, trn};

use super::SettingsPage;
use crate::pages::{PageCtx, PageEvent};
use crate::ui::controls::{action_button, caption, card, row, ButtonKind};
use crate::ui::fields::{meta_text, select_field};

#[derive(Default)]
pub struct State {
    /// What "Clear cache" would remove (measured off the UI thread).
    size: Option<ClearReport>,
    confirm: bool,
    busy: bool,
    notice: Option<(SharedString, bool)>,
    copied: bool,
    limit_open: bool,
}

/// Image cache caps offered (MB; 0 = no cap).
const IMAGE_CACHE_LIMITS: [u32; 6] = [256, 512, 1024, 2048, 4096, 0];

fn limit_label(mb: u32) -> String {
    match mb {
        0 => tr!("No limit").into(),
        mb if mb % 1024 == 0 => format!("{} GB", mb / 1024),
        mb => format!("{mb} MB"),
    }
}

impl State {
    pub fn measure(&mut self, ctx: &PageCtx, cx: &mut Context<SettingsPage>) {
        let (data_paths, app) = (ctx.data_paths.clone(), ctx.app.clone());
        cx.spawn(async move |this, cx| {
            let size = cx.background_spawn(async move { maintenance::cache_size(&data_paths, &app) }).await;
            this.update(cx, |this, cx| {
                this.advanced.size = Some(size);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Heroic's log folder (`~/.local/state/Heroic/logs`), when it exists.
pub fn logs_dir(data_paths: &DataPaths) -> Option<PathBuf> {
    let dir = heronite_launch::log_writer::LogPaths::for_data(data_paths).base;
    dir.is_dir().then_some(dir)
}

pub fn open_folder(dir: &Path) -> std::io::Result<()> {
    Command::new("xdg-open").arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// The global settings as pretty JSON for the clipboard, API keys redacted.
pub fn settings_json(data_paths: &DataPaths, home: &str) -> String {
    let mut settings = game_config::load_global(data_paths, home);
    for (k, v) in settings.iter_mut() {
        if k.to_ascii_lowercase().contains("apikey") && v.as_str().is_some_and(|s| !s.is_empty()) {
            *v = serde_json::Value::String("<hidden>".into());
        }
    }
    serde_json::to_string_pretty(&settings).unwrap_or_default()
}

impl SettingsPage {
    fn open_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.advanced.notice = match open_folder(&dir) {
            Ok(()) => None,
            Err(err) => Some((tr!("Could not open {path}: {err}", path = dir.display(), err).into(), true)),
        };
        cx.notify();
    }

    pub(super) fn clear_cache(&mut self, cx: &mut Context<Self>) {
        self.advanced.confirm = false;
        self.advanced.busy = true;
        let (data_paths, app) = (self.ctx.data_paths.clone(), self.ctx.app.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { maintenance::clear_caches(&data_paths, &app) }).await;
            this.update(cx, |this, cx| {
                this.advanced.busy = false;
                match res {
                    Ok(r) => {
                        this.advanced.notice = Some((
                            trn!(
                                r.files,
                                "Cache cleared: {n} file, {size}.",
                                "Cache cleared: {n} files, {size}.",
                                size = maintenance::human_size(r.bytes)
                            )
                            .into(),
                            false,
                        ));
                        this.advanced.size = Some(ClearReport::default());
                        // The Epic library cache is rebuilt by its refresh.
                        cx.emit(PageEvent::LibraryChanged);
                    }
                    Err(err) => this.advanced.notice = Some((format!("{err:#}").into(), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_image_cache_limit(&mut self, mb: u32, cx: &mut Context<Self>) {
        let res = store::save_image_cache_limit(&self.ctx.data_paths, mb);
        self.saved(res, cx);
        if let Some(upkeep) = crate::services::upkeep::get(cx) {
            upkeep.update(cx, |u, cx| u.trim_caches(cx));
        }
    }

    pub(super) fn advanced_extras(&mut self, cx: &mut Context<Self>) -> Div {
        let data_paths = self.ctx.data_paths.clone();
        let logs = logs_dir(&data_paths);

        let (cfg, app_cfg) = (data_paths.root.clone(), self.ctx.app.config.clone());
        let logs_btn = match logs.clone() {
            Some(dir) => action_button(
                "adv-open-logs",
                ButtonKind::Glass,
                tr!("Open"),
                true,
                cx.listener(move |this, _, _, cx| this.open_dir(dir.clone(), cx)),
            ),
            None => action_button("adv-open-logs", ButtonKind::Glass, tr!("Open"), false, |_, _, _| {}),
        };
        let folders = card([
            row(
                tr!("Data folder"),
                Some(tr!("Library, settings, logins, Wine tools and logs.")),
                action_button(
                    "adv-open-config",
                    ButtonKind::Glass,
                    tr!("Open"),
                    true,
                    cx.listener(move |this, _, _, cx| this.open_dir(cfg.clone(), cx)),
                ),
            )
            .into_any_element(),
            row(
                tr!("Logs folder"),
                Some(if logs.is_some() {
                    tr!("Logs of this app, the runners and your games.")
                } else {
                    tr!("There is no logs folder yet: it appears when a game is launched.")
                }),
                logs_btn,
            )
            .into_any_element(),
            row(
                tr!("Settings folder"),
                Some(tr!("Where this app keeps its own data.")),
                action_button(
                    "adv-open-app",
                    ButtonKind::Glass,
                    tr!("Open"),
                    true,
                    cx.listener(move |this, _, _, cx| this.open_dir(app_cfg.clone(), cx)),
                ),
            )
            .into_any_element(),
        ]);

        let size_line = match self.advanced.size {
            Some(r) if r.files > 0 => trn!(
                r.files,
                "{n} file, {size} would be removed.",
                "{n} files, {size} would be removed.",
                size = maintenance::human_size(r.bytes)
            ),
            Some(_) => tr!("Nothing to clear.").into(),
            None => tr!("Measuring...").into(),
        };
        let has_cache = self.advanced.size.is_some_and(|r| r.files > 0);
        let control: AnyElement = if self.advanced.confirm {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(action_button(
                    "adv-cache-cancel",
                    ButtonKind::Ghost,
                    tr!("Cancel"),
                    true,
                    cx.listener(|this, _, _, cx| {
                        this.advanced.confirm = false;
                        cx.notify();
                    }),
                ))
                .child(action_button(
                    "adv-cache-yes",
                    ButtonKind::Danger,
                    tr!("Yes, clear the cache"),
                    true,
                    cx.listener(|this, _, _, cx| this.clear_cache(cx)),
                ))
                .into_any_element()
        } else {
            action_button(
                "adv-cache-clear",
                ButtonKind::Glass,
                if self.advanced.busy { tr!("Clearing...") } else { tr!("Clear cache") },
                has_cache && !self.advanced.busy,
                cx.listener(|this, _, _, cx| {
                    this.advanced.confirm = true;
                    cx.notify();
                }),
            )
            .into_any_element()
        };
        let cache = card([
            row(
                tr!("Clear cache"),
                Some(&format!("{size_line} {}", tr!("Clears third-party game info (scores, Steam compatibility, HowLongToBeat, PCGamingWiki), Epic library info and resized thumbnails; they are rebuilt as you browse. Store logins, installed games, game settings, the Collection data and the GOG and Amazon caches are kept."))),
                control,
            )
            .into_any_element(),
            self.image_cache_limit(cx).into_any_element(),
        ]);

        let copied = self.advanced.copied;
        let copy = card([row(
            tr!("Copy all settings"),
            Some(tr!("Puts the settings JSON on the clipboard. API keys are left out.")),
            action_button(
                "adv-copy",
                ButtonKind::Glass,
                if copied { tr!("Copied to clipboard!") } else { tr!("Copy to clipboard") },
                true,
                cx.listener(|this, _, _, cx| {
                    let text = settings_json(&this.ctx.data_paths, &std::env::var("HOME").unwrap_or_default());
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    this.advanced.copied = true;
                    cx.notify();
                }),
            ),
        )
        .into_any_element()]);

        let mut col = div()
            .flex()
            .flex_col()
            .child(caption(tr!("Folders")))
            .child(folders)
            .child(caption(tr!("Cache")))
            .child(cache)
            .child(caption(tr!("Settings")))
            .child(copy);
        if let Some((text, error)) = self.advanced.notice.clone() {
            col = col.child(super::library::notice_line(text, error));
        }
        col
    }
}

impl SettingsPage {
    fn image_cache_limit(&mut self, cx: &mut Context<Self>) -> Div {
        let now = self.settings.image_cache_limit_mb;
        let select = select_field(
            "adv-image-cache",
            Some(tr!("Image cache limit")),
            limit_label(now),
            false,
            self.advanced.limit_open,
            IMAGE_CACHE_LIMITS.iter().map(|mb| (limit_label(*mb).into(), *mb == now)).collect(),
            cx,
            |this: &mut SettingsPage, _, cx| {
                this.advanced.limit_open = !this.advanced.limit_open;
                cx.notify();
            },
            |this: &mut SettingsPage, i, _, cx| {
                this.advanced.limit_open = false;
                this.set_image_cache_limit(IMAGE_CACHE_LIMITS[i], cx);
            },
        );
        div().flex().flex_col().gap(px(6.)).py(px(11.)).child(select).child(meta_text(tr!(
            "Covers and backgrounds downloaded for the Collection and their resized copies. Past the limit, the ones not shown for the longest go; they are downloaded again when needed."
        )))
    }
}

/// Console mode rows of "Folders and cache".
pub(super) fn console_rows(p: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    use crate::icons::Icon;
    let a = &p.advanced;
    let size_line = match a.size {
        Some(r) if r.files > 0 => {
            trn!(r.files, "{n} file, {size}", "{n} files, {size}", size = maintenance::human_size(r.bytes))
        }
        Some(_) => tr!("Nothing to clear").into(),
        None => tr!("Measuring\u{2026}").into(),
    };
    let has_cache = a.size.is_some_and(|r| r.files > 0);
    let mut rows = vec![
        Row::header(tr!("Folders")),
        Row::action("open-config", tr!("Open the data folder"))
            .icon(Icon::FolderOpen)
            .desc(tr!("Library, settings, logins, Wine tools and logs")),
        Row::action("open-logs", tr!("Open the logs folder")).icon(Icon::FolderOpen).enabled(logs_dir(&p.ctx.data_paths).is_some()),
        Row::action("open-app", tr!("Open the settings folder")).icon(Icon::FolderOpen),
        Row::header(tr!("Cache")),
    ];
    if a.confirm {
        rows.push(Row::action("cache-no", tr!("Keep the cache")));
        rows.push(Row::action("cache-yes", tr!("Yes, clear the cache")).danger());
    } else {
        rows.push(
            Row::action("cache", if a.busy { tr!("Clearing\u{2026}") } else { tr!("Clear the cache") })
                .icon(Icon::DeleteOutline)
                .desc(size_line)
                .enabled(has_cache && !a.busy),
        );
    }
    let limit = p.settings.image_cache_limit_mb;
    rows.push(Row::choice(
        "image-cache",
        tr!("Image cache limit"),
        IMAGE_CACHE_LIMITS.iter().map(|mb| limit_label(*mb).into()).collect(),
        IMAGE_CACHE_LIMITS.iter().position(|mb| *mb == limit).unwrap_or(2),
    ));
    rows.push(Row::header(tr!("Settings")));
    rows.push(
        Row::action("copy", if a.copied { tr!("Copied to the clipboard") } else { tr!("Copy all settings") })
            .desc(tr!("API keys are left out")),
    );
    if let Some((text, _)) = &a.notice {
        rows.push(Row::info("notice", text.clone(), ""));
    }
    rows
}

pub(super) fn console_apply(p: &mut SettingsPage, outcome: crate::console::list::Outcome, cx: &mut Context<SettingsPage>) {
    use crate::console::list::{Outcome, Value};
    if let Outcome::Changed(id, Value::Choice { index, .. }) = &outcome {
        if id.as_ref() == "image-cache" {
            p.set_image_cache_limit(IMAGE_CACHE_LIMITS[*index], cx);
        }
        return;
    }
    let Outcome::Activate(id) = outcome else { return };
    match id.as_ref() {
        "open-config" => p.open_dir(p.ctx.data_paths.root.clone(), cx),
        "open-logs" => {
            if let Some(dir) = logs_dir(&p.ctx.data_paths) {
                p.open_dir(dir, cx);
            }
        }
        "open-app" => p.open_dir(p.ctx.app.config.clone(), cx),
        "cache" => p.advanced.confirm = true,
        "cache-no" => p.advanced.confirm = false,
        "cache-yes" => p.clear_cache(cx),
        "copy" => {
            let text = settings_json(&p.ctx.data_paths, &std::env::var("HOME").unwrap_or_default());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            p.advanced.copied = true;
        }
        _ => {}
    }
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::{settings_json, DataPaths};

    #[test]
    fn settings_copy_hides_api_keys() {
        let dir = std::env::temp_dir().join(format!("heronite-settings-test-{}", std::process::id()));
        let data_paths = DataPaths::at(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), r#"{"defaultSettings":{"steamGridDbApiKey":"SECRET","maxWorkers":3},"version":"v0"}"#)
            .unwrap();
        let json = settings_json(&data_paths, "/home/u");
        assert!(!json.contains("SECRET"));
        assert!(json.contains("<hidden>"));
        assert!(json.contains("\"maxWorkers\": 3"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
