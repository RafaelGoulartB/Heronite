//! Settings tab: Emulators. Emulators and ROM folders are managed on the
//! Emulators page (☰ menu); this tab only summarises them and opens it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::EffectiveSettings;
use heronite_emulation::{list_rom_scanners, list_user_emulators};

use super::SettingsDialog;
use crate::dialogs::{DialogCtx, DialogEvent, DialogRequest};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::fields::{meta_text, panel, paragraph_secondary};

#[derive(Default)]
pub struct State {
    /// Names of the configured emulators and the number of ROM folders
    /// (`None` while loading).
    summary: Option<(Vec<String>, usize)>,
}

impl State {
    pub fn new(
        ctx: &DialogCtx,
        _settings: &EffectiveSettings,
        _window: &mut Window,
        cx: &mut Context<SettingsDialog>,
    ) -> Self {
        let paths = ctx.heroic.clone();
        let load = move || {
            let names = list_user_emulators(&paths).unwrap_or_default().into_iter().map(|e| e.name).collect();
            (names, list_rom_scanners(&paths).map(|s| s.len()).unwrap_or(0))
        };
        // Screenshots: the hidden window does not repaint late results.
        if std::env::var_os("HERONITE_DEBUG_SYNC").is_some() {
            return Self { summary: Some(load()) };
        }
        cx.spawn(async move |this, cx| {
            let summary = cx.background_spawn(async move { load() }).await;
            this.update(cx, |d, cx| {
                d.emulators.summary = Some(summary);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self::default()
    }
}

/// "2 emulators, 3 ROM folders."
fn summary_line(emulators: usize, folders: usize) -> String {
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!("{}, {}.", plural(emulators, "emulator", "emulators"), plural(folders, "ROM folder", "ROM folders"))
}

pub fn render(d: &mut SettingsDialog, _window: &mut Window, cx: &mut Context<SettingsDialog>) -> AnyElement {
    let mut card = panel().gap(px(10.));
    match &d.emulators.summary {
        None => card = card.child(meta_text("Loading\u{2026}")),
        Some((names, folders)) => {
            card = card.child(div().text_size(px(16.)).text_color(white()).child(summary_line(names.len(), *folders)));
            if !names.is_empty() {
                card = card.child(div().flex().flex_wrap().gap(px(6.)).children(names.iter().map(|n| {
                    div()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(10.))
                        .bg(theme::white_a(0.1))
                        .text_size(px(12.))
                        .text_color(rgb(0xcaf3fd))
                        .child(n.clone())
                })));
            }
        }
    }
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(paragraph_secondary(
            "Emulators, their platforms and the ROM folders are managed on the Emulators page: add or detect emulators, \
             edit their profiles and scan folders. Games stay in Collection and launch through the emulator you picked.",
        ))
        .child(card)
        .child(div().flex().child(
            button("emu-open-manager", ButtonKind::Default, "Open emulator manager")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Open(DialogRequest::EmulatorManager)))),
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::summary_line;

    #[test]
    fn summary_reads_naturally() {
        assert_eq!(summary_line(1, 1), "1 emulator, 1 ROM folder.");
        assert_eq!(summary_line(2, 0), "2 emulators, 0 ROM folders.");
    }
}
