//! Settings tab: Emulators (`EmulatorManager`): detect emulators, list them
//! with their ROM folders (scanners), rescan saved folders, remove entries.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::EffectiveSettings;
use heronite_emulation::{
    add_detected_emulators, get_emulation_state, remove_rom_scanner, remove_user_emulator, run_auto_rom_scanners,
    EmulationState, SystemEnv,
};

use super::SettingsDialog;
use crate::dialogs::scan_roms::ScanInitial;
use crate::dialogs::{DialogCtx, DialogEvent, DialogRequest};
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::fields::paragraph_secondary;

#[derive(Default)]
pub struct State {
    state: Option<EmulationState>,
    busy: bool,
    message: String,
    error: String,
}

impl State {
    pub fn new(
        _ctx: &DialogCtx,
        _settings: &EffectiveSettings,
        _window: &mut Window,
        cx: &mut Context<SettingsDialog>,
    ) -> Self {
        // Detection runs `flatpak info`; keep it off the UI thread.
        reload(cx, _ctx.clone(), true);
        Self::default()
    }
}

fn reload(cx: &mut Context<SettingsDialog>, ctx: DialogCtx, detect: bool) {
    cx.spawn(async move |this, cx| {
        let res = cx.background_spawn(async move { get_emulation_state(&ctx.heroic, &SystemEnv, detect) }).await;
        this.update(cx, |d, cx| {
            match res {
                Ok(s) => d.emulators.state = Some(s),
                Err(e) => d.emulators.error = format!("{e:#}"),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Runs `job` off the UI thread, then reloads and shows `done(result)`.
fn run<T: Send + 'static>(
    d: &mut SettingsDialog,
    cx: &mut Context<SettingsDialog>,
    job: impl FnOnce(&heronite_data::HeroicPaths) -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(T) -> String + 'static,
    detect: bool,
) {
    d.emulators.busy = true;
    d.emulators.error.clear();
    d.emulators.message.clear();
    let ctx = d.ctx.clone();
    cx.notify();
    cx.spawn(async move |this, cx| {
        let paths = ctx.heroic.clone();
        let res = cx.background_spawn(async move { job(&paths) }).await;
        this.update(cx, |d, cx| {
            d.emulators.busy = false;
            match res {
                Ok(v) => {
                    d.emulators.message = done(v);
                    reload(cx, ctx, detect);
                    cx.emit(DialogEvent::Changed);
                }
                Err(e) => d.emulators.error = format!("{e:#}"),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn render(d: &mut SettingsDialog, _window: &mut Window, cx: &mut Context<SettingsDialog>) -> AnyElement {
    let e = &d.emulators;
    let busy = e.busy;
    let scanners_empty = e.state.as_ref().map(|s| s.scanners.is_empty()).unwrap_or(true);

    let actions = div()
        .flex()
        .flex_wrap()
        .gap(px(8.))
        .child(action_button(
            "emu-detect",
            ButtonKind::Default,
            if busy { "Detecting…" } else { "Detect on this PC" },
            !busy,
            cx.listener(|d, _, _, cx| {
                run(
                    d,
                    cx,
                    |p| add_detected_emulators(p, &SystemEnv),
                    |added| {
                        if added.is_empty() {
                            "No emulators were found on PATH or Flatpak. Add one manually.".to_string()
                        } else {
                            format!("{} emulator(s) saved.", added.len())
                        }
                    },
                    true,
                )
            }),
        ))
        .child(action_button(
            "emu-scan-saved",
            ButtonKind::Outline,
            "Scan saved folders",
            !busy && !scanners_empty,
            cx.listener(|d, _, _, cx| {
                run(d, cx, run_auto_rom_scanners, |n| format!("Added {n} new game(s) from saved folders."), false)
            }),
        ))
        .child(button("emu-add-games", ButtonKind::Outline, "Add emulated games…").on_click(cx.listener(
            |_, _, _, cx| cx.emit(DialogEvent::Open(DialogRequest::ScanRoms(ScanInitial::default()))),
        )));

    let mut list = div().flex().flex_col().gap(px(12.));
    if let Some(state) = &e.state {
        if state.emulators.is_empty() {
            list = list.child(crate::ui::fields::panel().child(paragraph_secondary(
                "No emulators configured yet. Detect them on this PC, or add a custom executable when scanning ROMs.",
            )));
        }
        for emulator in &state.emulators {
            let eid = emulator.id.clone();
            let location = if emulator.launch_kind == "flatpak" {
                format!("Flatpak · {}", emulator.flatpak_id.clone().unwrap_or_default())
            } else {
                emulator.executable.clone()
            };
            let platforms = emulator
                .profiles
                .iter()
                .map(|p| p.platform_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| p.name.clone()))
                .collect::<Vec<_>>()
                .join(" · ");
            let mut card = crate::ui::fields::panel()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_start()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(div().font_weight(theme::W_BOLD).text_size(px(16.)).child(emulator.name.clone()))
                                .child(div().text_size(px(13.33)).text_color(white()).child(location)),
                        )
                        .child(action_button(
                            SharedString::from(format!("emu-rm-{eid}")),
                            ButtonKind::Outline,
                            "Remove",
                            !busy,
                            {
                                let eid = eid.clone();
                                cx.listener(move |d, _, _, cx| {
                                    let eid = eid.clone();
                                    run(d, cx, move |p| remove_user_emulator(p, &eid), |_| String::new(), false)
                                })
                            },
                        )),
                )
                .child(div().text_size(px(13.33)).text_color(white()).child(platforms));
            for scanner in state.scanners.iter().filter(|s| s.emulator_id == emulator.id) {
                let sid = scanner.id.clone();
                let (init_e, init_p, init_d) =
                    (scanner.emulator_id.clone(), scanner.profile_id.clone(), scanner.directory.clone());
                card = card.child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap(px(12.))
                        .p(px(8.))
                        .rounded(px(4.))
                        .bg(theme::black_a(0.2))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(div().font_weight(theme::W_BOLD).text_size(px(16.)).child(scanner.name.clone()))
                                .child(div().text_size(px(13.33)).text_color(white()).child(scanner.directory.clone())),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(
                                    button(SharedString::from(format!("emu-scan-{sid}")), ButtonKind::Outline, "Scan folder")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(DialogEvent::Open(DialogRequest::ScanRoms(ScanInitial {
                                                emulator_id: Some(init_e.clone()),
                                                profile_id: Some(init_p.clone()),
                                                directory: Some(init_d.clone()),
                                            })))
                                        })),
                                )
                                .child(action_button(
                                    SharedString::from(format!("emu-rmscan-{sid}")),
                                    ButtonKind::Outline,
                                    "Remove",
                                    !busy,
                                    {
                                        let sid = sid.clone();
                                        cx.listener(move |d, _, _, cx| {
                                            let sid = sid.clone();
                                            run(d, cx, move |p| remove_rom_scanner(p, &sid), |_| String::new(), false)
                                        })
                                    },
                                )),
                        ),
                );
            }
            let eid2 = emulator.id.clone();
            card = card.child(
                button(SharedString::from(format!("emu-addfolder-{eid2}")), ButtonKind::Outline, "Add ROM folder…")
                    .w_full()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(DialogEvent::Open(DialogRequest::ScanRoms(ScanInitial {
                            emulator_id: Some(eid2.clone()),
                            ..Default::default()
                        })))
                    })),
            );
            list = list.child(card);
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(paragraph_secondary(
            "Detect Linux/Flatpak emulators, then scan a ROM folder. Games stay in Collection and launch through the emulator you picked.",
        ))
        .child(actions)
        .children((!e.message.is_empty()).then(|| div().text_color(theme::success()).child(e.message.clone())))
        .children((!e.error.is_empty()).then(|| div().text_color(theme::danger()).child(e.error.clone())))
        .child(list)
        .into_any_element()
}
