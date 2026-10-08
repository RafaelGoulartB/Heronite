//! Settings → Metadata: IGDB credentials, background fill, image copy,
//! per-field source priority, bulk download.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::{self, default_field_priority, EffectiveSettings, METADATA_FIELDS};
use heronite_data::models::MetadataSettings;

use super::SettingsPage;
use crate::dialogs::metadata::field_label as metadata_field_label;
use crate::dialogs::metadata_wizard::hub;
use crate::pages::PageCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, toggle_row, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, new_input, on_commit, select_field};
use crate::ui::icon;

pub struct State {
    client_id: Entity<InputState>,
    secret: Entity<InputState>,
    testing: bool,
    ok_message: Option<SharedString>,
    advanced_open: bool,
    open_select: Option<&'static str>,
}

// "playnite" stays in the saved priorities (Heronite reads them) but is not
// offered: nothing feeds that source here.
const SOURCES: [(&str, &str); 5] = [
    ("auto", "Auto (IGDB first)"),
    ("igdb", "IGDB"),
    ("steam", "Steam"),
    ("store", "Store"),
    ("lutris", "Lutris"),
];

impl State {
    pub fn new(_ctx: &PageCtx, s: &EffectiveSettings, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let client_id = new_input(window, cx, &s.metadata.igdb_client_id, "");
        on_commit(&client_id, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if value != d.settings.metadata.igdb_client_id {
                persist(d, cx, |m| m.igdb_client_id = value);
            }
        });
        let secret = cx.new(|cx| {
            let mut st = InputState::new(window, cx).masked(true);
            st.set_value(s.metadata.igdb_client_secret.clone(), window, cx);
            st
        });
        on_commit(&secret, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if value != d.settings.metadata.igdb_client_secret {
                persist(d, cx, |m| m.igdb_client_secret = value);
            }
        });
        Self { client_id, secret, testing: false, ok_message: None, advanced_open: false, open_select: None }
    }
}

/// `persistMetadata`: replaces the metadata section with the edited copy.
fn persist(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, edit: impl FnOnce(&mut MetadataSettings)) {
    let mut m = d.settings.metadata.clone();
    edit(&mut m);
    d.metadata.ok_message = None;
    let res = settings::save_metadata(&d.ctx.heroic, &m);
    d.saved(res, cx);
}

fn preferred_source(d: &SettingsPage, field: &str) -> String {
    let default = default_field_priority(field);
    match d.settings.metadata.field_priority.get(field) {
        Some(cur) if *cur != default => cur.first().cloned().unwrap_or_else(|| "auto".into()),
        _ => "auto".into(),
    }
}

fn test_igdb(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.metadata.testing {
        return;
    }
    let id = d.metadata.client_id.read(cx).value().trim().to_string();
    let secret = d.metadata.secret.read(cx).value().to_string();
    if id != d.settings.metadata.igdb_client_id || secret != d.settings.metadata.igdb_client_secret {
        persist(d, cx, |m| {
            m.igdb_client_id = id;
            m.igdb_client_secret = secret;
        });
    }
    d.metadata.testing = true;
    d.metadata.ok_message = None;
    d.error = None;
    let service = hub(cx).read(cx).service.clone();
    cx.spawn(async move |this, cx| {
        let res = cx.background_spawn(async move { service.test_igdb_credentials() }).await;
        this.update(cx, |d, cx| {
            d.metadata.testing = false;
            if res.ok {
                d.metadata.ok_message = Some("IGDB credentials work.".into());
            } else {
                d.error = Some(res.error.unwrap_or_else(|| "IGDB test failed.".into()).into());
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let m = &d.settings.metadata;
    let (auto_fill, download) = (m.auto_fill_missing, m.download_images);
    let testing = d.metadata.testing;

    let creds = card_box().py(px(12.)).gap(px(12.)).child(labeled("Twitch / IGDB Client ID", input_box(&d.metadata.client_id, false))).child(
        labeled("Twitch / IGDB Client Secret", input_box(&d.metadata.secret, false)),
    );
    let actions = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mt(px(12.))
        .child(action_button("md-test", ButtonKind::Glass, "Test IGDB", !testing, cx.listener(|d, _, _, cx| test_igdb(d, cx))))
        .child(action_button(
            "md-wizard",
            ButtonKind::Primary,
            "Download metadata\u{2026}",
            true,
            cx.listener(|_, _, _, cx| hub(cx).update(cx, |h, cx| h.request_wizard(cx))),
        ))
        .children(d.metadata.ok_message.clone().map(|t| div().ml(px(6.)).text_size(px(13.)).text_color(theme::success()).child(t)));
    let images = card([
        toggle_row("md-autofill", "Fill missing metadata in the background", None, auto_fill)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.auto_fill_missing = !auto_fill)))
            .into_any_element(),
        toggle_row("md-download", "Copy covers to disk", Some("Off keeps the CDN URLs instead of storing the images."), download)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.download_images = !download)))
            .into_any_element(),
    ]);

    let mut col = div()
        .flex()
        .flex_col()
        .child(caption("Twitch / IGDB API"))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text("Create a Twitch app to get IGDB keys: https://dev.twitch.tv/console")))
        .child(creds)
        .child(actions)
        .child(caption("Images & fields"))
        .child(images);

    // Field priority (advanced), collapsible.
    let open = d.metadata.advanced_open;
    col = col.child(
        div()
            .id("md-advanced")
            .flex()
            .items_center()
            .gap(px(6.))
            .mt(px(14.))
            .mx(px(6.))
            .mb(px(6.))
            .cursor_pointer()
            .on_click(cx.listener(|d, _, _, cx| {
                d.metadata.advanced_open = !d.metadata.advanced_open;
                d.metadata.open_select = None;
                cx.notify();
            }))
            .child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(16.), theme::text_muted()))
            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child("Field priority (advanced)")),
    );
    if open {
        col = col.child(div().px(px(6.)).pb(px(8.)).child(meta_text("Preferred source per field. Other sources still fill empty values.")));
        let mut grid = card_box().py(px(12.)).flex_row().flex_wrap().gap_x(px(12.)).gap_y(px(10.));
        for field in METADATA_FIELDS {
            let current = preferred_source(d, field);
            let label = SOURCES.iter().find(|(v, _)| *v == current).map(|(_, l)| *l).unwrap_or("Auto (IGDB first)");
            let is_open = d.metadata.open_select == Some(field);
            let options: Vec<(SharedString, bool)> =
                SOURCES.iter().map(|(v, l)| (SharedString::from(*l), *v == current)).collect();
            grid = grid.child(div().w(px(200.)).child(select_field(
                &format!("md-prio-{field}"),
                Some(metadata_field_label(field)),
                label,
                false,
                is_open,
                options,
                cx,
                move |d: &mut SettingsPage, _, cx| {
                    d.metadata.open_select = if d.metadata.open_select == Some(field) { None } else { Some(field) };
                    cx.notify();
                },
                move |d: &mut SettingsPage, i, _, cx| {
                    let value = SOURCES[i].0;
                    d.metadata.open_select = None;
                    let default = default_field_priority(field);
                    let next: Vec<String> = if value == "auto" {
                        default
                    } else {
                        std::iter::once(value.to_string()).chain(default.into_iter().filter(|s| s != value)).collect()
                    };
                    persist(d, cx, |m| {
                        m.field_priority.insert(field.to_string(), next);
                    });
                },
            )));
        }
        col = col.child(grid);
    }
    col.into_any_element()
}
