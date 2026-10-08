//! `CollectionSettingsDialog`: six tabs, each in its own file. A tab module
//! exposes `pub struct State` (built in `State::new`) and
//! `pub fn render(&mut SettingsDialog, &mut Window, &mut Context<SettingsDialog>) -> AnyElement`.
//! Reference screenshots: `docs/reference/dialogs/settings-*.png`.

pub mod appearance;
pub mod backup;
pub mod emulators;
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
        // Debug: `HERONITE_DEBUG_SETTINGS_TAB=appearance|screenshots|metadata|emulators|backup|saves`.
        let tab = match std::env::var("HERONITE_DEBUG_SETTINGS_TAB").as_deref() {
            Ok("screenshots") => Tab::Screenshots,
            Ok("metadata") => Tab::Metadata,
            Ok("emulators") => Tab::Emulators,
            Ok("backup") => Tab::Backup,
            Ok("saves") => Tab::Saves,
            _ => Tab::Appearance,
        };
        Self {
            ctx,
            tab,
            settings,
            error: None,
            appearance,
            screenshots,
            metadata,
            emulators,
            backup,
            saves,
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
        };
        let body = div()
            .flex()
            .flex_col()
            .child(tab_bar)
            .child(div().mt(px(12.)).child(content))
            .children(self.error.clone().map(|e| div().mt(px(12.)).mx(px(6.)).text_color(theme::danger_text()).text_size(px(13.)).child(e)));
        let footer = button("settings-close", ButtonKind::Primary, "Done")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
            .into_any_element();
        let entity = cx.entity().downgrade();
        let vh = f32::from(window.viewport_size().height);
        dialog(
            DialogProps {
                id: "collection-settings".into(),
                title: "Collection settings".into(),
                width: 900f32.min(f32::from(window.viewport_size().width) * 0.92),
                height: Some(debug_dialog_height().unwrap_or(680f32.min(vh * 0.74))),
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
