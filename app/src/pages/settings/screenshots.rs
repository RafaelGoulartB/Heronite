//! Settings → Screenshots: folder (and the space it uses), in-game capture
//! shortcut, the controller's (the overlay's Guide + RB, shared with
//! Settings → Overlay), lossless compression, preview cache.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::{self, EffectiveSettings, DEFAULT_CAPTURE_ACCELERATOR};
use heronite_i18n::{grouped, tr, trn};
use heronite_media::{CacheInfo, ThumbnailCache};

use super::SettingsPage;
use crate::collection::screenshots_global;
use crate::pages::PageCtx;
use crate::services::compression::{self, Compression, Phase};
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, row, toggle_row, ButtonKind};
use crate::ui::fields::{accelerator_from_key, choose_path, hotkey_box, input_box, labeled, meta_text, new_input, on_commit, path_field};

pub struct State {
    folder: Entity<InputState>,
    limit: Entity<InputState>,
    hotkey: FocusHandle,
    cache: ThumbnailCache,
    info: Option<CacheInfo>,
    busy: bool,
    message: Option<SharedString>,
    /// The background compression service (absent in a one-shot launch).
    compression: Option<Entity<Compression>>,
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
            .unwrap_or_else(|| ThumbnailCache::new(ctx.data_paths.screenshot_thumbnails_dir(), s.screenshots.cache_limit_mb.get()));
        let compression = compression::get(cx);
        if let Some(c) = &compression {
            cx.observe(c, |_, _, cx| cx.notify()).detach();
        }
        Self { folder, limit, hotkey: cx.focus_handle(), cache, info: None, busy: false, message: None, compression }
    }

    /// Section shown: reads the cache size and measures the folder (off the
    /// UI thread).
    pub fn shown(&self, cx: &mut Context<SettingsPage>) {
        refresh_info(&self.cache, cx);
        if let Some(c) = &self.compression {
            c.update(cx, |c, cx| c.refresh_usage(cx));
        }
    }
}

fn format_number(v: f64) -> String {
    if v.fract() == 0. {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// `5.1 GB`, `640 MB`, `12 KB` (powers of 1024, like the cache numbers).
fn format_size(bytes: u64) -> String {
    const GB: f64 = 1024. * 1024. * 1024.;
    if bytes as f64 >= GB {
        format!("{} GB", heronite_i18n::decimal(bytes as f64 / GB, 1))
    } else if bytes >= 1024 * 1024 {
        format!("{} MB", (bytes as f64 / (1024. * 1024.)).round() as u64)
    } else {
        format_cache_size(bytes)
    }
}

/// `1,025` (`1.025` in Portuguese).
fn format_count(n: u64) -> String {
    grouped(n as i64)
}

fn files_text(n: u64) -> String {
    trn!(n, "{count} file", "{count} files", count = format_count(n))
}

/// Share of the PNG bytes a conversion frees, for the estimate shown before
/// it runs (measured on real game captures: 44-46%).
const EXPECTED_SAVING: f64 = 0.44;

/// The "Space used" row: value on the right, details below the label.
fn usage_text(c: Option<&Compression>) -> (String, Option<String>) {
    let Some(u) = c.and_then(|c| c.usage) else { return (tr!("Measuring\u{2026}").into(), None) };
    let mut details = files_text(u.files);
    if u.compressed_files > 0 {
        details.push_str(&tr!(
            " \u{b7} {compressed} compressed, {freed} freed",
            compressed = format_count(u.compressed_files),
            freed = format_size(u.saved_bytes)
        ));
    }
    (format_size(u.total_bytes), Some(details))
}

/// Label and description of the compression status row.
fn compression_status(c: &Compression) -> (String, String) {
    let t = &c.totals;
    let failed = if t.failed > 0 {
        trn!(
            t.failed,
            " \u{b7} {count} file could not be compressed (see the logs)",
            " \u{b7} {count} files could not be compressed (see the logs)",
            count = format_count(t.failed as u64)
        )
    } else {
        String::new()
    };
    match &c.phase {
        Phase::Off => {
            let estimate = c.usage.filter(|u| u.png_files > 0).map(|u| {
                trn!(
                    u.png_files,
                    "{count} PNG capture ({size}) \u{b7} about {saved} could be freed",
                    "{count} PNG captures ({size}) \u{b7} about {saved} could be freed",
                    count = format_count(u.png_files),
                    size = format_size(u.png_bytes),
                    saved = format_size((u.png_bytes as f64 * EXPECTED_SAVING) as u64)
                )
            });
            (tr!("Off").into(), estimate.unwrap_or_else(|| tr!("Turn it on to convert the PNG captures.").into()))
        }
        Phase::NoFolder => (tr!("Waiting for a folder").into(), tr!("Choose the screenshots folder above.").into()),
        Phase::Waiting => (
            tr!("Starts in a moment").into(),
            tr!("It waits a minute after the app opens or a game closes, so it never competes with them.").into(),
        ),
        Phase::Scanning => (tr!("Looking for captures to compress\u{2026}").into(), String::new()),
        Phase::Working { done, total, current } => (
            tr!("Compressing {done} of {total}", done = format_count((*done + 1).min(*total) as u64), total = format_count(*total as u64)),
            tr!("{freed} freed so far \u{b7} {current}{failed}", freed = format_size(t.freed.max(0) as u64), current),
        ),
        Phase::Paused { left } => (
            tr!("Paused while a game is running").into(),
            match left {
                Some(n) if *n > 0 => trn!(
                    *n,
                    "{count} capture left; it resumes a minute after the game closes.",
                    "{count} captures left; it resumes a minute after the game closes.",
                    count = format_count(*n as u64)
                ),
                _ => tr!("It resumes a minute after the game closes.").into(),
            },
        ),
        Phase::Idle if t.converted > 0 => (
            trn!(
                t.converted,
                "Done: {count} capture compressed",
                "Done: {count} captures compressed",
                count = format_count(t.converted as u64)
            ),
            tr!("{freed} freed this time{failed}", freed = format_size(t.freed.max(0) as u64)),
        ),
        Phase::Idle => (tr!("All captures are compressed").into(), tr!("New captures are compressed after each game.{failed}", failed)),
    }
}

fn format_cache_size(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{} KB", (bytes as f64 / 1024.).round() as u64)
    } else {
        format!("{} MB", heronite_i18n::decimal(bytes as f64 / (1024. * 1024.), 1))
    }
}

/// Reads the cache size off the UI thread.
fn refresh_info(cache: &ThumbnailCache, cx: &mut Context<SettingsPage>) {
    let cache = cache.clone();
    // Screenshots: the hidden window does not repaint late results.
    if crate::env::flag("HERONITE_DEBUG_SYNC") {
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
    let old_folder = cur.folder.clone();
    let limit = limit.unwrap_or(cur.cache_limit_mb.get());
    d.screenshots.message = None;
    let res = settings::save_screenshots(
        &d.ctx.data_paths,
        &folder.unwrap_or(cur.folder),
        limit,
        enabled.unwrap_or(cur.capture_enabled),
        &accelerator.unwrap_or(cur.capture_accelerator),
    );
    if let (Ok(s), Some(c)) = (&res, d.screenshots.compression.clone()) {
        if s.screenshots.folder != old_folder {
            c.update(cx, |c, cx| c.folder_changed(cx));
        }
    }
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

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let s = d.settings.screenshots.clone();
    let st = &d.screenshots;
    let has_folder = !s.folder.is_empty();
    let accel = if s.capture_accelerator.is_empty() { DEFAULT_CAPTURE_ACCELERATOR.to_string() } else { s.capture_accelerator.clone() };
    let status = screenshots_global(cx).and_then(|g| g.capture).map(|c| c.status());
    let focused = st.hotkey.is_focused(window);

    let hotkey = hotkey_box(
        div().id("shot-hotkey").track_focus(&st.hotkey).cursor_pointer().on_key_down(cx.listener(|d, ev: &KeyDownEvent, _, cx| {
            cx.stop_propagation();
            if let Some(a) = accelerator_from_key(ev) {
                save(d, None, None, None, Some(a), cx);
            }
        })),
        accel,
        focused,
    );

    let (used, used_details) = usage_text(st.compression.as_ref().map(|c| c.read(cx)));
    let folder = card_box()
        .py(px(12.))
        .child(path_field(
            "shot-folder-icon",
            Some(tr!("Screenshots folder")),
            &st.folder,
            has_folder,
            false,
            cx.listener(move |d: &mut SettingsPage, _: &ClickEvent, window, cx| {
                // "Clear" empties the folder, "Browse" opens the chooser.
                if has_folder {
                    d.screenshots.folder.update(cx, |s, cx| s.set_value("", window, cx));
                    save(d, Some(String::new()), None, None, None, cx);
                } else {
                    choose_path(window, cx, true, tr!("Choose"), |d: &mut SettingsPage, path, window, cx| {
                        d.screenshots.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                        save(d, Some(path), None, None, None, cx);
                    });
                }
            }),
        ))
        .when(has_folder && st.compression.is_some(), |c| {
            c.child(div().mt(px(12.)).h(px(1.)).w_full().bg(theme::row_separator())).child(
                row(tr!("Space used"), used_details.as_deref(), div().text_size(px(14.)).text_color(theme::text_primary()).child(used))
                    .pb(px(0.)),
            )
        });

    let status_line: Option<(SharedString, bool)> = if s.capture_enabled {
        Some(match status {
            Some(status) if status.registered => (tr!("Shortcut registered and ready.").into(), false),
            Some(status) if status.error.is_some() => (status.error.unwrap_or_default().into(), true),
            _ => (tr!("The shortcut activates automatically when a game starts.").into(), false),
        })
    } else {
        None
    };
    let capture = card([
        toggle_row("shot-capture-enabled", tr!("Capture screenshots while a game is running"), None, s.capture_enabled)
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
            .child(labeled(tr!("Screenshot shortcut"), hotkey))
            .child(meta_text(tr!(
                "Click the shortcut field and press the desired key combination. Captures are saved in the matched game folder and appear automatically in Collection."
            )))
            .children(status_line.map(|(text, error)| {
                div().text_size(px(12.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
            }))
            .into_any_element(),
    ]);

    let compress_card = st.compression.as_ref().map(|c| {
        let c = c.read(cx);
        let (label, desc) = compression_status(c);
        let enabled = c.enabled();
        let progress = match &c.phase {
            Phase::Working { done, total, .. } if *total > 0 => Some(format!("{}%", done * 100 / total)),
            _ => None,
        };
        let can_start = enabled && !c.busy() && !c.game_running();
        let control = match progress {
            Some(p) => div().text_size(px(13.)).text_color(theme::text_dim()).child(p).into_any_element(),
            None if !enabled => div().into_any_element(),
            None => action_button(
                "shot-compress-now",
                ButtonKind::Glass,
                tr!("Compress now"),
                can_start,
                cx.listener(|d, _, _, cx| {
                    if let Some(c) = d.screenshots.compression.clone() {
                        c.update(cx, |c, cx| c.start(cx));
                    }
                }),
            )
            .into_any_element(),
        };
        card([
            toggle_row(
                "shot-compress",
                tr!("Compress screenshots without quality loss"),
                Some(tr!("Converts PNG captures to lossless WebP: every pixel stays identical and the files take about half the space.")),
                enabled,
            )
            .on_click(cx.listener(|d, _, _, cx| {
                if let Some(c) = d.screenshots.compression.clone() {
                    c.update(cx, |c, cx| {
                        let on = !c.enabled();
                        c.set_enabled(on, cx);
                    });
                }
            }))
            .into_any_element(),
            row(label, (!desc.is_empty()).then_some(desc.as_str()), control).into_any_element(),
        ])
    });

    let can_clear = !st.busy && st.info.is_some_and(|i| i.item_count > 0);
    let usage = st.info.map(|i| {
        trn!(
            i.item_count,
            "{size} used by {n} preview, {limit} limit.",
            "{size} used by {n} previews, {limit} limit.",
            size = format_cache_size(i.size_bytes),
            limit = format_cache_size(i.limit_bytes)
        )
    });
    let clear = action_button(
        "shot-clear-cache",
        ButtonKind::Glass,
        if st.busy { tr!("Clearing…") } else { tr!("Clear preview cache") },
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
                    d.screenshots.message = Some(tr!("Screenshot preview cache cleared.").into());
                    cx.notify();
                })
                .ok();
            })
            .detach();
            cx.notify();
        }),
    );
    let cache_card = card([
        div().py(px(12.)).child(labeled(tr!("Preview cache limit (MB)"), input_box(&st.limit, st.busy))).into_any_element(),
        row(tr!("Clear preview cache"), usage.as_deref(), clear).into_any_element(),
    ]);

    let mut col = div()
        .flex()
        .flex_col()
        .child(caption(tr!("Screenshots folder")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(
            tr!("Folder with a subfolder per game (Xbox Game Bar Captures, NVIDIA, etc.). Collection matches those names to the selected game even when they are not identical."),
        )))
        .child(folder);
    if let Some(card) = compress_card {
        col = col
            .child(caption(tr!("Lossless compression")))
            .child(div().px(px(6.)).pb(px(8.)).child(meta_text(
                tr!("Runs in the background at the lowest priority, one capture at a time, and pauses while a game is running. Each converted file is marked in its own metadata, so nothing is compressed twice; if the app closes midway, it continues at the next start."),
            )))
            .child(card);
    }
    let overlay_on = d.overlay_enabled();
    let pad_shot = super::overlay::pad_shot_card(d, cx);
    col = col
        .child(caption(tr!("In-game capture")))
        .child(capture)
        .child(caption(tr!("With the controller")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(if overlay_on {
            tr!("Works during a game opened by Heronite, through the in-game overlay.")
        } else {
            tr!("Needs the in-game overlay, which is off: turn it on in Settings → Overlay.")
        })))
        .child(div().when(!overlay_on, |d| d.opacity(0.5)).child(pad_shot))
        .child(caption(tr!("Preview cache")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Thumbnails generated so the screenshot gallery opens instantly. They are recreated on demand after clearing."
        ))))
        .child(cache_card);
    if let Some(m) = st.message.clone() {
        col = col.child(div().mt(px(10.)).mx(px(6.)).text_size(px(13.)).text_color(theme::success()).child(m));
    }
    col.into_any_element()
}

/// Shortcuts offered in console mode (a controller cannot record one).
const CONSOLE_SHORTCUTS: [&str; 5] = ["F12", "Shift+F12", "CommandOrControl+F12", "Alt+F12", "CommandOrControl+Shift+S"];

fn console_shortcuts(current: &str) -> Vec<String> {
    let mut v: Vec<String> = CONSOLE_SHORTCUTS.iter().map(|s| s.to_string()).collect();
    if !v.iter().any(|s| s == current) {
        v.insert(0, current.to_string());
    }
    v
}

/// Console mode rows.
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    use crate::icons::Icon;
    let s = &d.settings.screenshots;
    let st = &d.screenshots;
    let accel = if s.capture_accelerator.is_empty() { DEFAULT_CAPTURE_ACCELERATOR.to_string() } else { s.capture_accelerator.clone() };
    let shortcuts = console_shortcuts(&accel);
    let si = shortcuts.iter().position(|x| *x == accel).unwrap_or(0);
    let usage = st
        .info
        .map(|i| trn!(i.item_count, "{size} used by {n} preview", "{size} used by {n} previews", size = format_cache_size(i.size_bytes)))
        .unwrap_or_default();
    let mut rows = vec![Row::text("folder", tr!("Screenshots folder"), st.folder.read(cx).value().to_string()).icon(Icon::FolderOpen)];
    if let Some(c) = st.compression.as_ref().map(|c| c.read(cx)) {
        if !s.folder.is_empty() {
            let (used, details) = usage_text(Some(c));
            rows.push(Row::info("usage", tr!("Space used"), used).desc(details.unwrap_or_default()));
        }
        let (label, desc) = compression_status(c);
        rows.extend([
            Row::header(tr!("Lossless compression")),
            Row::toggle("compress", tr!("Compress without quality loss"), c.enabled())
                .desc(tr!("PNG captures become lossless WebP: same pixels, about half the space")),
            Row::info("compress-status", label, "").desc(desc),
            Row::action("compress-now", tr!("Compress now")).enabled(c.enabled() && !c.busy() && !c.game_running()),
        ]);
    }
    rows.extend([
        Row::header(tr!("Capture")),
        Row::toggle("capture", tr!("Capture screenshots while a game runs"), s.capture_enabled),
        Row::choice("shortcut", tr!("Screenshot shortcut"), shortcuts.into_iter().map(SharedString::from).collect(), si)
            .desc(tr!("Captures go to the game's folder and show up in the Collection")),
        Row::header(tr!("With the controller")),
    ]);
    rows.extend(super::overlay::console_pad_shot_rows(d));
    rows.extend([
        Row::header(tr!("Preview cache")),
        Row::number("limit", tr!("Cache limit"), s.cache_limit_mb.get() as i64, 50, 10_000, 50, " MB"),
        Row::action("clear", if st.busy { tr!("Clearing\u{2026}") } else { tr!("Clear the preview cache") })
            .desc(usage)
            .enabled(!st.busy && st.info.is_some_and(|i| i.item_count > 0)),
    ]);
    if let Some(m) = &st.message {
        rows.push(Row::info("message", m.clone(), ""));
    }
    rows
}

pub(super) fn console_apply(
    d: &mut SettingsPage,
    outcome: crate::console::list::Outcome,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) {
    use crate::console::list::{Outcome, Value};
    if super::overlay::console_apply_pad_shot(d, &outcome, cx) {
        cx.notify();
        return;
    }
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            "folder" => {
                let start = Some(std::path::PathBuf::from(d.screenshots.folder.read(cx).value().to_string()));
                d.console_browse(
                    tr!("Screenshots folder"),
                    crate::console::browser::PickKind::Folder,
                    start,
                    window,
                    cx,
                    |d, path, window, cx| {
                        d.screenshots.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                        save(d, Some(path), None, None, None, cx);
                    },
                );
            }
            "compress-now" => {
                if let Some(c) = d.screenshots.compression.clone() {
                    c.update(cx, |c, cx| c.start(cx));
                }
            }
            "clear" => {
                d.screenshots.busy = true;
                d.screenshots.message = None;
                let cache = d.screenshots.cache.clone();
                cx.spawn(async move |this, cx| {
                    let info = cx.background_spawn(async move { cache.clear() }).await;
                    this.update(cx, |d, cx| {
                        d.screenshots.busy = false;
                        d.screenshots.info = Some(info);
                        d.screenshots.message = Some(tr!("Screenshot preview cache cleared.").into());
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            _ => {}
        },
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "capture" => save(d, None, None, Some(on), None, cx),
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "compress" => {
            if let Some(c) = d.screenshots.compression.clone() {
                c.update(cx, |c, cx| c.set_enabled(on, cx));
            }
        }
        Outcome::Changed(id, Value::Choice { options, index }) if id.as_ref() == "shortcut" => {
            if let Some(a) = options.get(index) {
                save(d, None, None, None, Some(a.to_string()), cx);
            }
        }
        Outcome::Changed(id, Value::Number { value, .. }) if id.as_ref() == "limit" => {
            let text = value.to_string();
            d.screenshots.limit.update(cx, |s, cx| s.set_value(text, window, cx));
            save(d, None, Some(value as f64), None, None, cx);
        }
        _ => {}
    }
    cx.notify();
}
