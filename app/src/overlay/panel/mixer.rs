//! The volume mixer of Home and Audio: one block with a bar for the system's
//! output, one for the game's own streams and one per other app playing (a
//! browser tab, a music player; [`HOME_APPS`] on Home, [`AUDIO_APPS`] on
//! Audio), and the output under them. The bars are rows of the list
//! (`mix-system`, `mix-game`, `mix-app-<n>`), so ↑ ↓ move between them like
//! any row; ◀ ▶ change the focused one, A mutes it, Y switches output.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_overlay::audio::{self, AudioState};

use super::OverlayPanel;
use crate::console::kit::{self, u};
use crate::console::list::Row;
use crate::console::Pad;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

pub const SYSTEM: &str = "mix-system";
pub const GAME: &str = "mix-game";
const APP: &str = "mix-app-";

/// Apps shown on Home (the panel's height is fixed) and on Audio.
pub const HOME_APPS: usize = 2;
pub const AUDIO_APPS: usize = 6;

/// Output names are long ("Family 17h/19h HD Audio Controller Analog
/// Stereo"); the chip keeps the start.
const OUTPUT_CHARS: usize = 28;

/// The mixer rows (values for ◀ ▶; the block draws them): the game's only
/// with a game, then up to `apps` other apps.
pub fn rows(a: &AudioState, game: bool, apps: usize) -> Vec<Row> {
    let mut rows = vec![Row::number(SYSTEM, tr!("System"), i64::from(a.volume), 0, i64::from(a.volume.max(100)), 5, "%")];
    if game {
        rows.push(match a.game_volume {
            Some(v) => Row::number(GAME, tr!("Game"), i64::from(v), 0, i64::from(v.max(100)), 5, "%"),
            None => Row::info(GAME, tr!("Game"), tr!("No sound yet")),
        });
    }
    for (i, app) in a.apps.iter().take(apps).enumerate() {
        rows.push(Row::number(app_id(i), app.name.clone(), i64::from(app.volume), 0, i64::from(app.volume.max(100)), 5, "%"));
    }
    rows
}

fn app_id(i: usize) -> SharedString {
    format!("{APP}{i}").into()
}

/// The app of a mixer row id.
pub fn app_index(id: &str) -> Option<usize> {
    id.strip_prefix(APP)?.parse().ok()
}

pub fn is_mixer(id: &str) -> bool {
    id == SYSTEM || id == GAME || app_index(id).is_some()
}

/// A globe for browsers, a play arrow for the rest.
fn app_icon(app: &audio::AppAudio) -> Icon {
    const BROWSERS: [&str; 10] = ["chrome", "chromium", "firefox", "zen", "brave", "vivaldi", "edge", "librewolf", "opera", "floorp"];
    let name = format!("{} {}", app.name.to_lowercase(), app.binary);
    if BROWSERS.iter().any(|b| name.contains(b)) {
        Icon::Language
    } else {
        Icon::PlayArrow
    }
}

pub fn short(name: &str) -> String {
    short_to(name, OUTPUT_CHARS)
}

/// A device name in a row's value: shorter, so the row's label keeps room.
pub fn short_row(name: &str) -> String {
    short_to(name, 20)
}

fn short_to(name: &str, chars: usize) -> String {
    if name.chars().count() <= chars {
        return name.to_string();
    }
    let cut: String = name.chars().take(chars - 1).collect();
    format!("{}…", cut.trim_end())
}

impl OverlayPanel {
    /// A and Y on a mixer bar (◀ ▶ go through the list as numbers).
    pub(super) fn mixer_pad(&mut self, id: &str, pad: Pad, cx: &mut Context<Self>) -> bool {
        match pad {
            Pad::A if id == SYSTEM => {
                let muted = self.set_audio(|a| {
                    a.muted = !a.muted;
                    a.muted
                });
                if let Some(m) = muted {
                    self.run_audio(move || audio::set_muted(m), cx);
                }
                true
            }
            Pad::A if id == GAME => {
                let Some(pid) = self.game.as_ref().and_then(|g| g.game.pid) else { return true };
                let muted = self.set_audio(|a| {
                    a.game_volume.map(|_| {
                        a.game_muted = !a.game_muted;
                        a.game_muted
                    })
                });
                if let Some(Some(m)) = muted {
                    self.run_audio(move || audio::set_game_muted(pid, m), cx);
                }
                true
            }
            Pad::Y => {
                self.next_output(cx);
                true
            }
            Pad::A => {
                let Some(i) = app_index(id) else { return true };
                let change = self.set_audio(|a| {
                    a.apps.get_mut(i).map(|app| {
                        app.muted = !app.muted;
                        (app.streams.clone(), app.muted)
                    })
                });
                if let Some(Some((streams, m))) = change {
                    self.run_audio(move || audio::set_streams_muted(&streams, m), cx);
                }
                true
            }
            _ => false,
        }
    }

    fn next_output(&mut self, cx: &mut Context<Self>) {
        let name = self.set_audio(|a| {
            let n = a.outputs.len();
            if n < 2 {
                return None;
            }
            let next = a.default_output.map_or(0, |i| (i + 1) % n);
            a.default_output = Some(next);
            Some(a.outputs[next].name.clone())
        });
        if let Some(Some(name)) = name {
            self.run_audio(move || audio::set_default_output(&name), cx);
            // The new output has its own volume.
            self.load_audio(cx);
        }
    }

    /// The block, with up to `apps` other apps; `focus` is the focused mixer
    /// row, if one is.
    pub(super) fn render_mixer(&self, a: &AudioState, focus: Option<&str>, loading: bool, apps: usize) -> Div {
        let focused = focus.is_some();
        let line = |id: &str, ic: Icon, label: SharedString, level: Option<u16>, muted: bool| {
            let on = focus == Some(id);
            let fg: Hsla = if focused { theme::console_on_focus().into() } else { theme::text_primary().into() };
            let fill = match (muted, focused) {
                (true, _) => theme::overlay_bar_muted(),
                (false, true) => theme::overlay_bar_focus(),
                (false, false) => theme::console_btn_x(),
            };
            let value = match level {
                _ if loading => "\u{2026}".to_string(),
                None => tr!("No sound yet").to_string(),
                Some(_) if muted => tr!("Muted").to_string(),
                Some(v) => format!("{v}%"),
            };
            let frac = if loading { 0. } else { level.map_or(0., |v| (f32::from(v.min(100)) / 100.).clamp(0., 1.)) };
            div()
                .flex()
                .items_center()
                .gap(u(12.))
                .px(u(12.))
                .py(u(10.))
                .rounded(u(12.))
                .when(on, |d| d.bg(theme::black_a(0.08)))
                .child(icon(if muted { Icon::VolumeOff } else { ic }, u(20.), fg))
                .child(div().w(u(108.)).flex_none().min_w_0().truncate().text_size(u(16.)).text_color(fg).child(label))
                .child(
                    div()
                        .flex_1()
                        .h(u(8.))
                        .rounded_full()
                        .bg(if focused { theme::black_a(0.14) } else { theme::white_a(0.14) })
                        .child(div().h_full().w(relative(frac)).rounded_full().bg(fill)),
                )
                .child(
                    div()
                        .w(u(84.))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .text_size(u(14.))
                        .font_weight(theme::W_BOLD)
                        .text_color(fg)
                        .whitespace_nowrap()
                        .child(value),
                )
        };
        // While loading, a placeholder keeps the line (and the block's height).
        let output = if loading {
            Some("\u{2026}".to_string())
        } else {
            a.default_output.and_then(|i| a.outputs.get(i)).map(|o| short(&o.description))
        };
        let muted_color: Hsla = if focused { theme::console_on_focus().into() } else { theme::text_muted().into() };
        let mut block = div()
            .relative()
            .flex()
            .flex_col()
            .gap(u(2.))
            .p(u(6.))
            .rounded(u(16.))
            .border_2()
            .child(line(SYSTEM, Icon::VolumeUp, tr!("System").into(), Some(a.volume), a.muted))
            .when(self.game.is_some(), |d| d.child(line(GAME, Icon::SportsEsports, tr!("Game").into(), a.game_volume, a.game_muted)))
            .children(
                a.apps
                    .iter()
                    .take(apps)
                    .enumerate()
                    .map(|(i, app)| line(&app_id(i), app_icon(app), app.name.clone().into(), Some(app.volume), app.muted)),
            )
            .children(output.map(|o| {
                div()
                    .flex()
                    .items_center()
                    .gap(u(8.))
                    .px(u(12.))
                    .pt(u(4.))
                    .pb(u(6.))
                    .text_size(u(13.))
                    .text_color(muted_color)
                    .child(icon(Icon::Headphones, u(15.), muted_color))
                    .child(div().truncate().child(o))
                    .when(focused && a.outputs.len() > 1, |d| {
                        d.child(
                            div()
                                .ml_auto()
                                .rounded(u(6.))
                                .when(self.keyboard, |d| d.bg(theme::black_a(0.7)))
                                .child(self.glyph(crate::console::Btn::Y)),
                        )
                    })
            }));
        block = if focused {
            block.bg(theme::console_focus()).border_color(theme::white_a(0.55)).shadow(kit::shadow(0.45, 8., 24.))
        } else {
            block.bg(theme::white_a(0.05)).border_color(theme::white_a(0.06))
        };
        block
    }
}
