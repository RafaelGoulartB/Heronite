//! Settings → Cloud: what this PC sends to the cloud, measured on this PC
//! (`heronite_sync::usage`; the account is never asked), at the top of the
//! card of what goes. A bar of each kind (faded when it is off), the total of
//! what is on, how fast the screenshots grow, and, on demand, a treemap of
//! what takes the space by game.

use std::path::Path;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_data::screenshot_prefs::ScreenshotPrefs;
use heronite_data::settings::DEFAULT_BACKUP_KEEP;
use heronite_i18n::tr;
use heronite_sync::usage::{self, Kind, Local, Usage};
use heronite_sync::SyncPrefs;

use crate::pages::settings::storage::treemap::squarify;
use crate::pages::settings::SettingsPage;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::fields::meta_text;

/// Tiles of the treemap; the rest share one.
const TILES: usize = 24;
const MAP_W: f32 = 808.;
const MAP_H: f32 = 240.;

pub(super) fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Library => tr!("Library"),
        Kind::Backups => tr!("Backups"),
        Kind::Saves => tr!("Saves"),
        Kind::Captures => tr!("Screenshots"),
    }
}

pub(super) fn kind_color(kind: Kind) -> Hsla {
    match kind {
        Kind::Library => theme::chart_series(0),
        Kind::Backups => theme::chart_series(2),
        Kind::Saves => theme::chart_series(1),
        Kind::Captures => theme::chart_series(3),
    }
    .into()
}

/// Whether the kind goes to the cloud (not off).
fn is_on(kind: Kind, p: &SyncPrefs) -> bool {
    match kind {
        Kind::Library => p.library.is_on(),
        Kind::Backups => p.backups.is_on(),
        Kind::Saves => p.saves.is_on(),
        Kind::Captures => p.captures.is_on(),
    }
}

/// Faded while the kind is off.
fn color(kind: Kind, p: &SyncPrefs) -> Hsla {
    if is_on(kind, p) {
        kind_color(kind)
    } else {
        kind_color(kind).opacity(0.3)
    }
}

/// Measures what this PC sends, off the UI thread (walks the folders).
pub(super) fn measure(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.cloud.measuring {
        return;
    }
    d.cloud.measuring = true;
    let (data, app) = (d.ctx.data_paths.clone(), d.ctx.app.clone());
    let s = d.settings.clone();
    let replays = d.cloud.prefs.as_ref().is_some_and(|p| p.replays);
    cx.spawn(async move |this, cx| {
        let measured = cx
            .background_spawn(async move {
                let captures = s.screenshots_folder().map(Path::to_path_buf).or_else(|| data.default_screenshots());
                let saves = s.ludusavi.last_backup_path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf);
                let local = Local {
                    captures: captures.as_deref(),
                    pending_png: ScreenshotPrefs::load(&app).compress_lossless,
                    replays,
                    saves: saves.as_deref(),
                    backups_kept: s.backup.keep.unwrap_or(DEFAULT_BACKUP_KEEP),
                    latest_backup: s
                        .backup
                        .last_backup_path
                        .as_deref()
                        .filter(|p| Path::new(p).is_dir())
                        .and_then(|p| Path::new(p).file_name())
                        .map(|n| n.to_string_lossy().into_owned()),
                };
                usage::measure(&data, &app, &local)
            })
            .await;
        this.update(cx, |d, cx| {
            d.cloud.measuring = false;
            d.cloud.usage = Some(measured);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn bar(u: &Usage, p: &SyncPrefs) -> Div {
    let whole = Kind::ALL.iter().map(|k| u.total(*k)).sum::<u64>().max(1) as f32;
    div().flex().w_full().h(px(10.)).rounded(px(5.)).overflow_hidden().bg(theme::chart_empty()).children(
        Kind::ALL.into_iter().filter(|k| u.total(*k) > 0).map(|kind| {
            div().h_full().w(relative(u.total(kind) as f32 / whole)).bg(color(kind, p)).border_r_1().border_color(theme::black_a(0.4))
        }),
    )
}

/// What the kinds that are on take in all, and how the screenshots grow.
fn summary(u: &Usage, p: &SyncPrefs) -> String {
    let on: u64 = Kind::ALL.iter().filter(|k| is_on(**k, p)).map(|k| u.total(*k)).sum();
    let mut text = if on == 0 {
        tr!("Nothing goes to the cloud yet: choose below what does.").to_owned()
    } else {
        tr!("What is turned on takes about {size} in the cloud.", size = human_size(on))
    };
    if p.captures.is_on() && u.monthly > 0 {
        text.push(' ');
        text.push_str(&tr!(
            "Screenshots add about {monthly} a month, {yearly} a year.",
            monthly = human_size(u.monthly),
            yearly = human_size(u.monthly * 12)
        ));
    }
    text
}

/// The name of a share in the treemap.
fn share_label(kind: Kind, name: &str) -> String {
    match (kind, name) {
        (Kind::Library, _) => tr!("Library").to_owned(),
        (Kind::Backups, "zip") => tr!("Backup zips").to_owned(),
        (Kind::Backups, "art") => tr!("Your artwork").to_owned(),
        (_, name) => name.to_owned(),
    }
}

/// What takes the space by game (screenshots, saves) or part (backups), of
/// the kinds that are on (all of them while none is).
fn map(u: &Usage, p: &SyncPrefs) -> Div {
    let any_on = Kind::ALL.iter().any(|k| is_on(*k, p) && u.total(*k) > 0);
    let mut items: Vec<(Kind, &str, u64)> = u
        .shares
        .iter()
        .filter(|(k, _)| !any_on || is_on(**k, p))
        .flat_map(|(k, m)| m.iter().map(move |(name, bytes)| (*k, name.as_str(), *bytes)))
        .filter(|(.., b)| *b > 0)
        .collect();
    items.sort_by_key(|(.., b)| std::cmp::Reverse(*b));
    let total: u64 = items.iter().map(|(.., b)| b).sum();
    let (own, rest) = items.split_at(items.len().min(TILES));
    let rest_bytes: u64 = rest.iter().map(|(.., b)| b).sum();
    let mut values: Vec<f64> = own.iter().map(|(.., b)| *b as f64).collect();
    if !rest.is_empty() {
        values.push((rest_bytes as f64).max(total as f64 * 0.04));
    }
    let mut area = div().relative().w(px(MAP_W)).h(px(MAP_H));
    for t in squarify(&values, MAP_W, MAP_H) {
        let (x, y, w, h) = (t.x + 2., t.y + 2., (t.w - 4.).max(0.), (t.h - 4.).max(0.));
        let (bg, title, size) = match own.get(t.index) {
            Some((k, name, bytes)) => (kind_color(*k).opacity(0.8), share_label(*k, name), human_size(*bytes)),
            None => (theme::ink_a(0.08), format!("+{}", rest.len()), human_size(rest_bytes)),
        };
        let label = h >= 40. && w >= 70.;
        area = area.child(div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).rounded(px(8.)).overflow_hidden().bg(bg).when(
            label,
            |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(9.))
                        .right(px(9.))
                        .bottom(px(7.))
                        .flex()
                        .flex_col()
                        .child(
                            div().font_weight(theme::W_BOLD).text_size(px(12.5)).text_color(theme::text_bright()).truncate().child(title),
                        )
                        .child(div().text_size(px(11.5)).text_color(theme::text_primary()).truncate().child(size)),
                )
            },
        ));
    }
    area
}

/// The top of the card of what goes: what is on takes in all and how the
/// screenshots grow, "Show what takes space" (the treemap) and "Measure
/// again", and the bar of each kind (the lines below carry its colours).
pub(super) fn render(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let col = div().flex().flex_col().gap(px(10.)).py(px(12.));
    let Some(u) = d.cloud.usage.clone() else {
        return col.child(meta_text(tr!("Measuring…")));
    };
    let open = d.cloud.map_open;
    let toggle = button_sm("cloud-map", ButtonKind::Ghost, if open { tr!("Hide what takes space") } else { tr!("Show what takes space") })
        .on_click(cx.listener(move |d, _, _, cx| {
            d.cloud.map_open = !open;
            cx.notify();
        }));
    let again = button_sm("cloud-measure", ButtonKind::Ghost, if d.cloud.measuring { tr!("Measuring…") } else { tr!("Measure again") })
        .when(!d.cloud.measuring, |b| b.on_click(cx.listener(|d, _, _, cx| measure(d, cx))));
    let head = div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_x(px(12.))
        .gap_y(px(4.))
        .child(
            div()
                .flex_grow(1.)
                .flex_basis(px(240.))
                .min_w_0()
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(theme::text_muted())
                .child(summary(&u, p)),
        )
        .child(div().flex().flex_none().ml_auto().gap(px(4.)).child(toggle).child(again));
    col.child(head).child(bar(&u, p)).when(open, |c| c.child(map(&u, p)))
}
