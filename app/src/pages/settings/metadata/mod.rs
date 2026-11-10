//! Settings → Metadata: where the details and pictures of the games come
//! from. A summary card (how many games have details, "Update metadata…",
//! `summary.rs`); the sources as rows that open in place, in three cards by
//! kind (`sources.rs`, an open row in `source_body.rs`); the cover shapes
//! kept and who draws them (`shapes.rs`); what happens when details arrive
//! (`arrive.rs`); the order of each field (`field_order.rs`);
//! console mode's rows (`console.rs`).
//!
//! The rows come from the metadata service ([`SourceRow`]). Building them
//! reads the global settings (SteamGridDB's key), so they are built when the
//! section shows and after a change (`stale`), never per frame.
//!
//! Debug: `HERONITE_DEBUG_METADATA_OPEN=<source id>[,<source id>…]` opens
//! those rows (`advanced`: the order of each field) and scrolls to the last;
//! `HERONITE_DEBUG_SOURCE_DOWNLOAD` shows a source's database in a made-up
//! state (`dialogs/metadata_update/source_downloads.rs`).
//!
//! An extension's row adds who made it, where it came from and its actions
//! (`extension.rs`); under the cards, the dashed "Get more sources" row
//! (`more.rs`) opens the gallery (`dialogs/extensions`).

mod arrive;
mod console;
mod download;
mod extension;
mod field_order;
pub(crate) mod labels;
mod more;
mod shapes;
mod source_body;
mod sources;
mod summary;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::MetadataSettings;
use heronite_data::settings::{self, EffectiveSettings, METADATA_FIELDS};
use heronite_data::source_prefs::ids;
use heronite_i18n::tr;
use heronite_metadata::sources::{builtin_descriptors, OptionKind, OptionValue, SourceRow, SourceStatus};
use heronite_metadata::MetadataService;

pub(super) use console::{console_apply, console_rows};

use super::SettingsPage;
use crate::dialogs::metadata_wizard::{hub, HubEvent, HubGlobal};
use crate::ui::fields::on_commit;

/// One field's sources, first to last, as the merge asks them.
struct FieldLine {
    field: &'static str,
    sources: Vec<String>,
    /// The user set this field's order (it no longer follows the rows').
    customised: bool,
}

/// The outcome of a source's last "Test" on this page.
struct Tested {
    ok: bool,
    message: SharedString,
}

#[derive(Default)]
pub struct State {
    rows: Vec<SourceRow>,
    fields: Vec<FieldLine>,
    /// The rows no longer match the settings: built again before the next draw.
    stale: bool,
    /// The text and secret options' inputs, by (source id, option key).
    inputs: HashMap<(String, String), Entity<InputState>>,
    /// The sources whose rows are open.
    open: HashSet<String>,
    /// "Order for one field (advanced)" is open.
    advanced_open: bool,
    /// The select whose menu is open (`age-board`, `opt:<source>:<key>`).
    open_select: Option<SharedString>,
    /// The source being tested.
    testing: Option<String>,
    tested: HashMap<String, Tested>,
    coverage: Option<summary::Coverage>,
    /// The Collection's games that need attention, counted with `coverage`.
    attention: Option<heronite_metadata::service::Attention>,
    coverage_task: Option<Task<()>>,
    /// The cover families and their counts ("Cover shapes"), counted with `coverage`.
    shapes: Option<shapes::Shapes>,
    /// `HERONITE_DEBUG_METADATA_OPEN` was applied.
    debug_done: bool,
    /// Follows the hub: a database download's progress, and its end.
    hub_watch: Option<Subscription>,
    /// The extension whose removal is being asked.
    removing: Option<String>,
    /// An extension is being removed.
    ext_busy: bool,
    /// Why the last removal failed.
    ext_error: Option<String>,
    /// An extension `HERONITE_DEBUG_METADATA_OPEN` named: opened once listed.
    debug_reveal: Option<String>,
}

impl State {
    /// The page's copy of the settings changed: the rows follow.
    pub(super) fn settings_changed(&mut self) {
        self.stale = true;
    }

    fn row(&self, id: &str) -> Option<&SourceRow> {
        self.rows.iter().find(|r| r.descriptor.id == id)
    }
}

/// The metadata service of the window (`None` before the Collection made it).
fn service(cx: &App) -> Option<Arc<MetadataService>> {
    cx.has_global::<HubGlobal>().then(|| hub(cx).read(cx).service.clone())
}

/// Redraws while a source's database downloads, and reads the rows again
/// once one was downloaded or removed.
fn watch_hub(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.metadata.hub_watch.is_some() || !cx.has_global::<HubGlobal>() {
        return;
    }
    let entity = hub(cx);
    let watch = cx.subscribe(&entity, |d: &mut SettingsPage, _, ev: &HubEvent, cx| {
        if let HubEvent::SourceDownloaded | HubEvent::ExtensionsChanged = ev {
            d.metadata.stale = true;
            cx.notify();
        }
    });
    cx.observe(&entity, |_, _, cx| cx.notify()).detach();
    d.metadata.hub_watch = Some(watch);
}

/// The section is on screen: its rows, and the count of the summary.
pub(super) fn shown(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    d.metadata.stale = true;
    watch_hub(d, cx);
    if cx.has_global::<HubGlobal>() {
        hub(cx).update(cx, |h, cx| h.read_extensions(false, cx));
    }
    ensure_rows(d, window, cx);
    summary::count(d, cx);
    if !d.metadata.debug_done {
        d.metadata.debug_done = true;
        debug_open(d, cx);
    }
}

/// The Collection's library changed while the page is on screen.
pub(super) fn library_changed(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.section == super::Section::Metadata {
        summary::count(d, cx);
    }
}

/// Builds the rows again when they are stale, and the inputs of options not
/// seen yet. An input that does not have the keyboard takes the saved value.
pub(super) fn ensure_rows(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    // The page may show before the Collection made the hub.
    watch_hub(d, cx);
    if !d.metadata.stale {
        reveal_installed(d, cx);
        return;
    }
    let Some(service) = service(cx) else { return };
    d.metadata.stale = false;
    let m: &MetadataSettings = &d.settings.metadata;
    let mut rows = service.source_rows(m);
    debug_download(&mut rows, cx);
    // Only the sources shown as rows: a hidden one (Playnite) or one this
    // build does not know keeps its place when the order is saved.
    let shown = |id: &String| rows.iter().any(|r| r.descriptor.id == *id);
    d.metadata.fields = METADATA_FIELDS
        .iter()
        .map(|field| {
            let sources = service.field_order(m, field).into_iter().filter(shown).collect();
            FieldLine { field, sources, customised: service.is_field_customised(m, field) }
        })
        .collect();
    for r in &rows {
        for o in r.options.iter().filter(|o| matches!(o.option.kind, OptionKind::Text | OptionKind::Secret)) {
            let key = (r.descriptor.id.clone(), o.option.key.clone());
            let value = o.value.as_text().to_string();
            match d.metadata.inputs.get(&key) {
                Some(input) => {
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    if !focused && input.read(cx).value() != value.as_str() {
                        input.update(cx, |s, cx| s.set_value(value, window, cx));
                    }
                }
                None => {
                    let secret = o.option.kind == OptionKind::Secret;
                    let input = cx.new(|cx| {
                        let mut s = InputState::new(window, cx).masked(secret);
                        s.set_value(value, window, cx);
                        s
                    });
                    let (id, opt) = key.clone();
                    on_commit(&input, window, cx, move |d: &mut SettingsPage, value, _, cx| {
                        let saved =
                            d.metadata.row(&id).and_then(|r| r.options.iter().find(|o| o.option.key == opt)).map(|o| o.value.as_text());
                        if saved != Some(value.as_str()) {
                            set_option(d, &id, &opt, OptionValue::Text(value), cx);
                        }
                    });
                    d.metadata.inputs.insert(key, input);
                }
            }
        }
    }
    d.metadata.rows = rows;
    reveal_installed(d, cx);
}

/// Opens the row of the extension just installed and scrolls to it, once
/// the rows list it.
fn reveal_installed(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let from_hub = cx.has_global::<HubGlobal>().then(|| hub(cx).read(cx).extensions.reveal.clone()).flatten();
    let Some(id) = from_hub.or_else(|| d.metadata.debug_reveal.clone()) else { return };
    let Some(name) = d.metadata.row(&id).map(|r| labels::name(&r.descriptor.name).to_string()) else { return };
    if cx.has_global::<HubGlobal>() {
        hub(cx).update(cx, |h, _| h.extensions.reveal = None);
    }
    if d.metadata.debug_reveal.as_deref() == Some(id.as_str()) {
        d.metadata.debug_reveal = None;
    }
    d.metadata.open.insert(id);
    flash_to(d, name, cx);
}

/// Scrolls the page to the row whose name is `text` and flashes it.
fn flash_to(d: &mut SettingsPage, text: String, cx: &mut Context<SettingsPage>) {
    crate::ui::find::set_target(text);
    d.find_frames = super::FIND_FRAMES;
    d.find_task = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(crate::ui::find::FLASH).await;
        this.update(cx, |_, cx| {
            crate::ui::find::clear();
            cx.notify();
        })
        .ok();
    }));
}

/// Records a write made through the service and builds the rows again.
fn apply(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, write: impl FnOnce(&MetadataService) -> anyhow::Result<EffectiveSettings>) {
    let Some(service) = service(cx) else { return };
    let res = write(&service);
    d.saved(res, cx);
    d.metadata.stale = true;
}

/// Writes the metadata settings with `edit` applied (the switches that
/// belong to no source).
fn persist(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, edit: impl FnOnce(&mut MetadataSettings)) {
    let mut m = d.settings.metadata.clone();
    edit(&mut m);
    let res = settings::save_metadata(&d.ctx.data_paths, &m);
    d.saved(res, cx);
    d.metadata.stale = true;
}

fn set_enabled(d: &mut SettingsPage, id: &str, on: bool, cx: &mut Context<SettingsPage>) {
    apply(d, cx, |s| s.set_source_enabled(id, on));
    // The tags the Collection lacks come right away.
    if on && id == ids::STEAM_TAGS && cx.has_global::<HubGlobal>() {
        hub(cx).update(cx, |h, cx| h.refresh_steam_tags(cx));
    }
}

fn set_option(d: &mut SettingsPage, id: &str, key: &str, value: OptionValue, cx: &mut Context<SettingsPage>) {
    d.metadata.tested.remove(id);
    apply(d, cx, |s| s.set_source_option(id, key, value));
}

fn move_source(d: &mut SettingsPage, id: &str, to: usize, cx: &mut Context<SettingsPage>) {
    apply(d, cx, |s| s.move_source(id, to));
}

fn toggle_open(d: &mut SettingsPage, id: &str, cx: &mut Context<SettingsPage>) {
    if !d.metadata.open.remove(id) {
        d.metadata.open.insert(id.to_string());
    }
    d.metadata.open_select = None;
    cx.notify();
}

/// Runs the test of `id` off the UI thread, after saving what was typed in
/// its fields and not committed yet.
fn test(d: &mut SettingsPage, id: &str, window: &mut Window, cx: &mut Context<SettingsPage>) {
    if d.metadata.testing.is_some() {
        return;
    }
    let Some(service) = service(cx) else { return };
    let typed: Vec<(String, String)> = d
        .metadata
        .row(id)
        .into_iter()
        .flat_map(|r| r.options.iter())
        .filter_map(|o| {
            let input = d.metadata.inputs.get(&(id.to_string(), o.option.key.clone()))?;
            let value = input.read(cx).value().trim().to_string();
            (value != o.value.as_text()).then(|| (o.option.key.clone(), value))
        })
        .collect();
    for (key, value) in typed {
        set_option(d, id, &key, OptionValue::Text(value), cx);
    }
    d.metadata.testing = Some(id.to_string());
    d.metadata.tested.remove(id);
    let name = builtin_descriptors().iter().find(|s| s.id == id).map_or(id.to_string(), |s| labels::name(&s.name).to_string());
    let id = id.to_string();
    cx.spawn_in(window, async move |this, cx| {
        let source = id.clone();
        let res = cx.background_spawn(async move { service.test_source(&source) }).await;
        this.update_in(cx, |d, window, cx| {
            d.metadata.testing = None;
            if let Some(res) = res {
                let message = match (res.ok, res.error) {
                    (true, _) if id == ids::IGDB => tr!("IGDB credentials work.").to_string(),
                    (true, _) => tr!("{name} answered.", name),
                    (false, Some(e)) => e,
                    (false, None) if id == ids::IGDB => tr!("IGDB test failed.").to_string(),
                    (false, None) => tr!("{name} did not answer.", name),
                };
                d.metadata.tested.insert(id.clone(), Tested { ok: res.ok, message: message.into() });
            }
            d.metadata.stale = true;
            ensure_rows(d, window, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

/// The search picked `label` in this section: opens what holds it (a
/// source's row, one of its options, the order of each field).
pub(super) fn reveal(d: &mut SettingsPage, label: &str) {
    if label == "Order for one field (advanced)" {
        d.metadata.advanced_open = true;
        return;
    }
    let holder = builtin_descriptors().iter().find(|s| s.name == label || s.options.iter().any(|o| o.label == label));
    if let Some(s) = holder {
        d.metadata.open.insert(s.id.clone());
    }
}

/// `HERONITE_DEBUG_SOURCE_DOWNLOAD`: the row of the source it names shows
/// the made-up state, whatever is on disk.
fn debug_download(rows: &mut [SourceRow], cx: &App) {
    let hub = hub(cx);
    for r in rows.iter_mut() {
        let Some(state) = hub.read(cx).fake_download_state(&r.descriptor.id) else { continue };
        if let Some(download) = r.download.as_mut() {
            download.state = state.clone();
            if r.enabled {
                r.status = if state.ready { SourceStatus::On } else { SourceStatus::NeedsDownload };
            }
        }
    }
}

/// `HERONITE_DEBUG_METADATA_OPEN`: opens the rows named and scrolls to the last.
fn debug_open(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let Some(list) = crate::env::value("HERONITE_DEBUG_METADATA_OPEN") else { return };
    let mut last = None;
    for id in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if id == "advanced" {
            d.metadata.advanced_open = true;
            last = Some(heronite_i18n::tr("Order for one field (advanced)").to_string());
        } else if let Some(s) = builtin_descriptors().iter().find(|s| s.id == id) {
            d.metadata.open.insert(s.id.clone());
            last = Some(labels::name(&s.name).to_string());
        } else {
            // An extension: listed once the extensions are loaded.
            d.metadata.debug_reveal = Some(id.to_string());
        }
    }
    if let Some(text) = last {
        flash_to(d, text, cx);
    }
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    ensure_rows(d, window, cx);
    div()
        .flex()
        .flex_col()
        .child(summary::card(d))
        .children(sources::cards(d, cx))
        .child(more::row(cx))
        .children(shapes::card(d, cx))
        .child(arrive::card(d, cx))
        .child(field_order::block(d, cx))
        .into_any_element()
}
