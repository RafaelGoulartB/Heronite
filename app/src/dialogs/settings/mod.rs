//! `CollectionSettingsDialog`: six tabs, each in its own file. A tab module
//! exposes `pub struct State` (built in `State::new`) and
//! `pub fn render(&mut SettingsDialog, &mut Window, &mut Context<SettingsDialog>) -> AnyElement`.
//! Reference screenshots: `docs/reference/dialogs/settings-*.png`.

pub mod appearance;
pub mod backup;
pub mod emulators;
pub mod heroic;
pub mod metadata;
pub mod saves;
pub mod screenshots;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::EffectiveSettings;

use super::heroic_settings::debug_dialog_height;
use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{button, tabs, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Appearance,
    Screenshots,
    Metadata,
    Emulators,
    Backup,
    Saves,
    /// Heroic's global settings (`heroic_settings`).
    General,
    GameDefaults,
}

pub struct SettingsDialog {
    pub ctx: DialogCtx,
    pub tab: Tab,
    pub settings: EffectiveSettings,
    pub error: Option<SharedString>,
    pub appearance: appearance::State,
    pub screenshots: screenshots::State,
    pub metadata: metadata::State,
    pub emulators: emulators::State,
    pub backup: backup::State,
    pub saves: saves::State,
    pub heroic: heroic::State,
}

impl EventEmitter<DialogEvent> for SettingsDialog {}

impl SettingsDialog {
    /// `preview_chain`: stage artwork of the focused game (live preview).
    pub fn new(
        ctx: DialogCtx,
        preview_chain: Vec<heronite_images::ImageRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = ctx.lib.settings.clone();
        let appearance = appearance::State::new(&ctx, &settings, preview_chain, window, cx);
        let screenshots = screenshots::State::new(&ctx, &settings, window, cx);
        let metadata = metadata::State::new(&ctx, &settings, window, cx);
        let emulators = emulators::State::new(&ctx, &settings, window, cx);
        let backup = backup::State::new(&ctx, &settings, window, cx);
        let saves = saves::State::new(&ctx, &settings, window, cx);
        Self {
            ctx,
            tab: Tab::Appearance,
            settings,
            error: None,
            appearance,
            screenshots,
            metadata,
            emulators,
            backup,
            saves,
            heroic: Default::default(),
        }
    }

    /// Records the result of a settings write and tells the Collection.
    pub fn saved(&mut self, result: anyhow::Result<EffectiveSettings>, cx: &mut Context<Self>) {
        match result {
            Ok(s) => {
                self.settings = s;
                self.error = None;
                cx.emit(DialogEvent::Changed);
            }
            Err(err) => self.error = Some(format!("{err:#}").into()),
        }
        cx.notify();
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_bar = tabs(
            "settings",
            &[
                (Tab::Appearance, "Appearance"),
                (Tab::Screenshots, "Screenshots"),
                (Tab::Metadata, "Metadata"),
                (Tab::Emulators, "Emulators"),
                (Tab::Backup, "Backup"),
                (Tab::Saves, "Save backup"),
                (Tab::General, "General"),
                (Tab::GameDefaults, "Defaults"),
            ],
            self.tab,
            {
                let entity = cx.entity().downgrade();
                move |t, _, cx| {
                    entity
                        .update(cx, |this, cx| {
                            this.tab = t;
                            cx.notify();
                        })
                        .ok();
                }
            },
        );
        let content = match self.tab {
            Tab::Appearance => appearance::render(self, window, cx),
            Tab::Screenshots => screenshots::render(self, window, cx),
            Tab::Metadata => metadata::render(self, window, cx),
            Tab::Emulators => emulators::render(self, window, cx),
            Tab::Backup => backup::render(self, window, cx),
            Tab::Saves => saves::render(self, window, cx),
            Tab::General => heroic::render_general(self, window, cx),
            Tab::GameDefaults => heroic::render_defaults(self, window, cx),
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(tab_bar)
            .child(content)
            .children(self.error.clone().map(|e| div().text_color(theme::danger()).text_size(px(13.)).child(e)));
        let footer = button("settings-close", ButtonKind::Outline, "Close")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
            .into_any_element();
        let entity = cx.entity().downgrade();
        let vh = f32::from(window.viewport_size().height);
        dialog(
            DialogProps {
                id: "collection-settings".into(),
                title: "Collection settings".into(),
                width: 840f32.min(f32::from(window.viewport_size().width) * 0.9) + 64.,
                height: Some(debug_dialog_height().unwrap_or(620f32.min(vh * 0.7))),
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

/// Placeholder body for tabs that are not ported yet.
pub fn pending(text: &str) -> AnyElement {
    div()
        .py(px(24.))
        .text_size(px(14.))
        .text_color(theme::white_a(0.6))
        .child(format!("{text} — not available in this build yet."))
        .into_any_element()
}
