//! Settings → Screenshots: folder, in-game capture shortcut, preview cache.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::{self, EffectiveSettings, DEFAULT_CAPTURE_ACCELERATOR};
use heronite_media::{CacheInfo, ThumbnailCache};

use super::SettingsPage;
use crate::collection::screenshots_global;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, row, toggle_row, ButtonKind};
use crate::ui::fields::{choose_path, input_box, labeled, meta_text, new_input, on_commit, path_field};

pub struct State {
    folder: Entity<InputState>,
    limit: Entity<InputState>,
    hotkey: FocusHandle,
    cache: ThumbnailCache,
    info: Option<CacheInfo>,
    busy: bool,
    message: Option<SharedString>,
}

impl State {
    pub fn new(ctx: &PageCtx, s: &EffectiveSettings, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let folder = new_input(window, cx, &s.screenshots.folder, "");
        on_commit(&folder, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if value != d.settings.screenshots.folder {
                save(d, Some(value), None, None, None, cx);
            }
        });
        let limit = new_input(window, cx, &format_number(s.screenshots.cache_limit_mb.get()), "");
        on_commit(&limit, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if let Ok(mb) = value.parse::<f64>() {
                save(d, None, Some(mb), None, None, cx);
            }
        });
        let cache = screenshots_global(cx)
            .map(|g| g.cache)
            .unwrap_or_else(|| ThumbnailCache::new(ctx.heroic.screenshot_thumbnails_dir(), s.screenshots.cache_limit_mb.get()));
        Self { folder, limit, hotkey: cx.focus_handle(), cache, info: None, busy: false, message: None }
    }

    /// Section shown: reads the cache size (off the UI thread).
    pub fn shown(&self, cx: &mut Context<SettingsPage>) {
        refresh_info(&self.cache, cx);
    }
}

fn format_number(v: f64) -> String {
    if v.fract() == 0. {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn format_cache_size(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{} KB", (bytes as f64 / 1024.).round() as u64)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024. * 1024.))
    }
}

/// Reads the cache size off the UI thread.
fn refresh_info(cache: &ThumbnailCache, cx: &mut Context<SettingsPage>) {
    let cache = cache.clone();
    // Screenshots: the hidden window does not repaint late results.
    if std::env::var_os("HERONITE_DEBUG_SYNC").is_some() {
        let info = cache.info();
        cx.spawn(async move |this, cx| {
            this.update(cx, |d, cx| {
                d.screenshots.info = Some(info);
                cx.notify();
            })
            .ok();
        })
        .detach();
        return;
    }
    cx.spawn(async move |this, cx| {
        let info = cx.background_spawn(async move { cache.info() }).await;
        this.update(cx, |d, cx| {
            d.screenshots.info = Some(info);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// `persistScreenshots`: writes the settings with some fields replaced.
fn save(
    d: &mut SettingsPage,
    folder: Option<String>,
    limit: Option<f64>,
    enabled: Option<bool>,
    accelerator: Option<String>,
    cx: &mut Context<SettingsPage>,
) {
    let cur = d.settings.screenshots.clone();
    let limit = limit.unwrap_or(cur.cache_limit_mb.get());
    d.screenshots.message = None;
    let res = settings::save_screenshots(
        &d.ctx.heroic,
        &folder.unwrap_or(cur.folder),
        limit,
        enabled.unwrap_or(cur.capture_enabled),
        &accelerator.unwrap_or(cur.capture_accelerator),
    );
    if let Ok(s) = &res {
        // Apply the new budget to the shared cache and refresh the numbers.
        let cache = d.screenshots.cache.clone();
        cache.set_limit_mb(s.screenshots.cache_limit_mb.get());
        cx.spawn(async move |this, cx| {
            let info = cx.background_spawn(async move { cache.maintain() }).await;
            this.update(cx, |d, cx| {
                d.screenshots.info = Some(info);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    d.saved(res, cx);
}

/// Electron-style accelerator from a key press (`acceleratorFromKey`).
fn accelerator_from_key(ev: &KeyDownEvent) -> Option<String> {
    let k = ev.keystroke.key.as_str();
    if matches!(k, "control" | "ctrl" | "alt" | "shift" | "meta" | "super" | "platform" | "cmd" | "function") {
        return None;
    }
    let key = match k {
        "space" => "Space".to_string(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "escape" => "Esc".into(),
        "print" | "printscreen" => "PrintScreen".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    };
    let m = &ev.keystroke.modifiers;
    let mut parts = Vec::new();
    if m.control || m.platform {
        parts.push("CommandOrControl".to_string());
    }
    if m.alt {
        parts.push("Alt".into());
    }
    if m.shift {
        parts.push("Shift".into());
    }
    parts.push(key);
    Some(parts.join("+"))
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let s = d.settings.screenshots.clone();
    let st = &d.screenshots;
    let has_folder = !s.folder.is_empty();
    let accel = if s.capture_accelerator.is_empty() { DEFAULT_CAPTURE_ACCELERATOR.to_string() } else { s.capture_accelerator.clone() };
    let status = screenshots_global(cx).and_then(|g| g.capture).map(|c| c.status());
    let focused = st.hotkey.is_focused(window);

    let hotkey = input_box_text(
        div()
            .id("shot-hotkey")
            .track_focus(&st.hotkey)
            .cursor_pointer()
            .on_key_down(cx.listener(|d, ev: &KeyDownEvent, _, cx| {
                cx.stop_propagation();
                if let Some(a) = accelerator_from_key(ev) {
                    save(d, None, None, None, Some(a), cx);
                }
            })),
        accel,
        focused,
    );

    let folder = card_box().py(px(12.)).child(path_field(
        "shot-folder-icon",
        Some("Screenshots folder"),
        &st.folder,
        has_folder,
        false,
        cx.listener(move |d: &mut SettingsPage, _: &ClickEvent, window, cx| {
            // "Clear" empties the folder, "Browse" opens the chooser.
            if has_folder {
                d.screenshots.folder.update(cx, |s, cx| s.set_value("", window, cx));
                save(d, Some(String::new()), None, None, None, cx);
            } else {
                choose_path(window, cx, true, "Choose", |d: &mut SettingsPage, path, window, cx| {
                    d.screenshots.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                    save(d, Some(path), None, None, None, cx);
                });
            }
        }),
    ));

    let status_line: Option<(SharedString, bool)> = if s.capture_enabled {
        Some(match status {
            Some(status) if status.registered => ("Shortcut registered and ready.".into(), false),
            Some(status) if status.error.is_some() => (status.error.unwrap_or_default().into(), true),
            _ => ("The shortcut activates automatically when a game starts.".into(), false),
        })
    } else {
        None
    };
    let capture = card([
        toggle_row("shot-capture-enabled", "Capture screenshots while a game is running", None, s.capture_enabled)
            .on_click(cx.listener(move |d, _, _, cx| {
                let next = !d.settings.screenshots.capture_enabled;
                save(d, None, None, Some(next), None, cx);
            }))
            .into_any_element(),
        div()
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(labeled("Screenshot shortcut", hotkey))
            .child(meta_text(
                "Click the shortcut field and press the desired key combination. Captures are saved in the matched game folder and appear automatically in Collection.",
            ))
            .children(status_line.map(|(text, error)| {
                div().text_size(px(12.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
            }))
            .into_any_element(),
    ]);

    let can_clear = !st.busy && st.info.is_some_and(|i| i.item_count > 0);
    let usage = st.info.map(|i| {
        format!(
            "{} used by {} previews, {} limit.",
            format_cache_size(i.size_bytes),
            i.item_count,
            format_cache_size(i.limit_bytes)
        )
    });
    let clear = action_button(
        "shot-clear-cache",
        ButtonKind::Glass,
        if st.busy { "Clearing…" } else { "Clear preview cache" },
        can_clear,
        cx.listener(|d, _, _, cx| {
            d.screenshots.busy = true;
            d.screenshots.message = None;
            let cache = d.screenshots.cache.clone();
            cx.spawn(async move |this, cx| {
                let info = cx.background_spawn(async move { cache.clear() }).await;
                this.update(cx, |d, cx| {
                    d.screenshots.busy = false;
                    d.screenshots.info = Some(info);
                    d.screenshots.message = Some("Screenshot preview cache cleared.".into());
                    cx.notify();
                })
                .ok();
            })
            .detach();
            cx.notify();
        }),
    );
    let cache_card = card([
        div().py(px(12.)).child(labeled("Preview cache limit (MB)", input_box(&st.limit, st.busy))).into_any_element(),
        row("Clear preview cache", usage.as_deref(), clear).into_any_element(),
    ]);

    let mut col = div()
        .flex()
        .flex_col()
        .child(caption("Screenshots folder"))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(
            "Folder with a subfolder per game (Xbox Game Bar Captures, NVIDIA, etc.). Collection matches those names to the selected game even when they are not identical.",
        )))
        .child(folder)
        .child(caption("In-game capture"))
        .child(capture)
        .child(caption("Preview cache"))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(
            "Thumbnails generated so the screenshot gallery opens instantly. They are recreated on demand after clearing.",
        )))
        .child(cache_card);
    if let Some(m) = st.message.clone() {
        col = col.child(div().mt(px(10.)).mx(px(6.)).text_size(px(13.)).text_color(theme::success()).child(m));
    }
    col.into_any_element()
}

/// Read-only input look with the accelerator text (`input_box` needs an
/// `InputState`; this one is a plain focusable box).
fn input_box_text(base: Stateful<Div>, text: String, focused: bool) -> Stateful<Div> {
    base.flex()
        .items_center()
        .w_full()
        .h(px(theme::INPUT_H))
        .px(px(14.))
        .rounded(px(theme::R_INPUT))
        .bg(theme::input_fill())
        .border_1()
        .border_color(if focused { theme::input_border_focus() } else { theme::input_border() })
        .font_family(theme::FONT_BODY)
        .text_size(px(14.))
        .text_color(theme::text_primary())
        .child(text)
}
