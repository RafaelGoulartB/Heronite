//! Settings → Advanced extras (below the panel's switches): folders, clear
//! cache and "copy all settings".

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::{self, ClearReport};
use heronite_data::{game_config, HeroicPaths};

use super::SettingsPage;
use crate::pages::{PageCtx, PageEvent};
use crate::ui::controls::{action_button, caption, card, row, ButtonKind};

#[derive(Default)]
pub struct State {
    /// What "Clear cache" would remove (measured off the UI thread).
    size: Option<ClearReport>,
    confirm: bool,
    busy: bool,
    notice: Option<(SharedString, bool)>,
    copied: bool,
}

impl State {
    pub fn measure(&mut self, ctx: &PageCtx, cx: &mut Context<SettingsPage>) {
        let (heroic, app) = (ctx.heroic.clone(), ctx.app.clone());
        cx.spawn(async move |this, cx| {
            let size = cx.background_spawn(async move { maintenance::cache_size(&heroic, &app) }).await;
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
pub fn logs_dir(heroic: &HeroicPaths) -> Option<PathBuf> {
    let dir = heronite_launch::log_writer::LogPaths::for_heroic(heroic).base;
    dir.is_dir().then_some(dir)
}

pub fn open_folder(dir: &Path) -> std::io::Result<()> {
    Command::new("xdg-open").arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// The global settings as pretty JSON for the clipboard, API keys redacted.
pub fn settings_json(heroic: &HeroicPaths, home: &str) -> String {
    let mut settings = game_config::load_global(heroic, home);
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
            Err(err) => Some((format!("Could not open {}: {err}", dir.display()).into(), true)),
        };
        cx.notify();
    }

    pub(super) fn clear_cache(&mut self, cx: &mut Context<Self>) {
        self.advanced.confirm = false;
        self.advanced.busy = true;
        let (heroic, app) = (self.ctx.heroic.clone(), self.ctx.app.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { maintenance::clear_caches(&heroic, &app) }).await;
            this.update(cx, |this, cx| {
                this.advanced.busy = false;
                match res {
                    Ok(r) => {
                        this.advanced.notice = Some((
                            format!("Cache cleared: {} file(s), {}.", r.files, maintenance::human_size(r.bytes)).into(),
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

    pub(super) fn advanced_extras(&mut self, cx: &mut Context<Self>) -> Div {
        let heroic = self.ctx.heroic.clone();
        let logs = logs_dir(&heroic);

        let (cfg, app_cfg) = (heroic.root.clone(), self.ctx.app.config.clone());
        let logs_btn = match logs.clone() {
            Some(dir) => action_button(
                "adv-open-logs",
                ButtonKind::Glass,
                "Open",
                true,
                cx.listener(move |this, _, _, cx| this.open_dir(dir.clone(), cx)),
            ),
            None => action_button("adv-open-logs", ButtonKind::Glass, "Open", false, |_, _, _| {}),
        };
        let folders = card([
            row(
                "Config folder",
                Some("Shared with Heroic and Heronite."),
                action_button("adv-open-config", ButtonKind::Glass, "Open", true, cx.listener(move |this, _, _, cx| this.open_dir(cfg.clone(), cx))),
            )
            .into_any_element(),
            row(
                "Logs folder",
                Some(if logs.is_some() { "Logs of this app, Heroic and your games." } else { "There is no logs folder yet: it appears when a game is launched." }),
                logs_btn,
            )
            .into_any_element(),
            row(
                "App data folder",
                Some("Where this app keeps its own data."),
                action_button("adv-open-app", ButtonKind::Glass, "Open", true, cx.listener(move |this, _, _, cx| this.open_dir(app_cfg.clone(), cx))),
            )
            .into_any_element(),
        ]);

        let size_line = match self.advanced.size {
            Some(r) if r.files > 0 => format!("{} file(s), {} would be removed.", r.files, maintenance::human_size(r.bytes)),
            Some(_) => "Nothing to clear.".into(),
            None => "Measuring...".into(),
        };
        let has_cache = self.advanced.size.is_some_and(|r| r.files > 0);
        let control: AnyElement = if self.advanced.confirm {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(action_button("adv-cache-cancel", ButtonKind::Ghost, "Cancel", true, cx.listener(|this, _, _, cx| {
                    this.advanced.confirm = false;
                    cx.notify();
                })))
                .child(action_button("adv-cache-yes", ButtonKind::Danger, "Yes, clear the cache", true, cx.listener(|this, _, _, cx| this.clear_cache(cx))))
                .into_any_element()
        } else {
            action_button(
                "adv-cache-clear",
                ButtonKind::Glass,
                if self.advanced.busy { "Clearing..." } else { "Clear cache" },
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
                "Clear cache",
                Some(&format!("{size_line} Clears third-party game info (scores, Steam compatibility, HowLongToBeat, PCGamingWiki), Epic library info and resized thumbnails; they are rebuilt as you browse. Store logins, installed games, game settings, the Collection data and the GOG and Amazon caches are kept.")),
                control,
            )
            .into_any_element(),
        ]);

        let copied = self.advanced.copied;
        let copy = card([row(
            "Copy all settings",
            Some("Puts the settings JSON on the clipboard. API keys are left out."),
            action_button(
                "adv-copy",
                ButtonKind::Glass,
                if copied { "Copied to clipboard!" } else { "Copy to clipboard" },
                true,
                cx.listener(|this, _, _, cx| {
                    let text = settings_json(&this.ctx.heroic, &std::env::var("HOME").unwrap_or_default());
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
            .child(caption("Folders"))
            .child(folders)
            .child(caption("Cache"))
            .child(cache)
            .child(caption("Settings"))
            .child(copy);
        if let Some((text, error)) = self.advanced.notice.clone() {
            col = col.child(super::library::notice_line(text, error));
        }
        col
    }
}

/// Console mode rows of "Folders and cache".
pub(super) fn console_rows(p: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    use crate::icons::Icon;
    let a = &p.advanced;
    let size_line = match a.size {
        Some(r) if r.files > 0 => format!("{} file(s), {}", r.files, maintenance::human_size(r.bytes)),
        Some(_) => "Nothing to clear".into(),
        None => "Measuring\u{2026}".into(),
    };
    let has_cache = a.size.is_some_and(|r| r.files > 0);
    let mut rows = vec![
        Row::header("Folders"),
        Row::action("open-config", "Open the config folder").icon(Icon::FolderOpen).desc("Shared with Heroic and Heronite"),
        Row::action("open-logs", "Open the logs folder").icon(Icon::FolderOpen).enabled(logs_dir(&p.ctx.heroic).is_some()),
        Row::action("open-app", "Open the app data folder").icon(Icon::FolderOpen),
        Row::header("Cache"),
    ];
    if a.confirm {
        rows.push(Row::action("cache-no", "Keep the cache"));
        rows.push(Row::action("cache-yes", "Yes, clear the cache").danger());
    } else {
        rows.push(Row::action("cache", if a.busy { "Clearing\u{2026}" } else { "Clear the cache" }).icon(Icon::DeleteOutline).desc(size_line).enabled(has_cache && !a.busy));
    }
    rows.push(Row::header("Settings"));
    rows.push(Row::action("copy", if a.copied { "Copied to the clipboard" } else { "Copy all settings" }).desc("API keys are left out"));
    if let Some((text, _)) = &a.notice {
        rows.push(Row::info("notice", text.clone(), ""));
    }
    rows
}

pub(super) fn console_apply(p: &mut SettingsPage, outcome: crate::console::list::Outcome, cx: &mut Context<SettingsPage>) {
    let crate::console::list::Outcome::Activate(id) = outcome else { return };
    match id.as_ref() {
        "open-config" => p.open_dir(p.ctx.heroic.root.clone(), cx),
        "open-logs" => {
            if let Some(dir) = logs_dir(&p.ctx.heroic) {
                p.open_dir(dir, cx);
            }
        }
        "open-app" => p.open_dir(p.ctx.app.config.clone(), cx),
        "cache" => p.advanced.confirm = true,
        "cache-no" => p.advanced.confirm = false,
        "cache-yes" => p.clear_cache(cx),
        "copy" => {
            let text = settings_json(&p.ctx.heroic, &std::env::var("HOME").unwrap_or_default());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            p.advanced.copied = true;
        }
        _ => {}
    }
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::{settings_json, HeroicPaths};

    #[test]
    fn settings_copy_hides_api_keys() {
        let dir = std::env::temp_dir().join(format!("heronite-settings-test-{}", std::process::id()));
        let heroic = HeroicPaths::at(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"defaultSettings":{"steamGridDbApiKey":"SECRET","maxWorkers":3},"version":"v0"}"#,
        )
        .unwrap();
        let json = settings_json(&heroic, "/home/u");
        assert!(!json.contains("SECRET"));
        assert!(json.contains("<hidden>"));
        assert!(json.contains("\"maxWorkers\": 3"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
