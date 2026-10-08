//! `CollectionMetadataDialog`: fetch metadata candidates (IGDB, Steam, store,
//! Lutris, Playnite), pick a source per field and apply.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::CollectionGameMetadata;
use heronite_data::settings::METADATA_FIELDS;
use heronite_data::collection::Game;
use heronite_metadata::{MetadataApplyArgs, MetadataPreview, MetadataSearchHit, MetadataService};

use super::art_images::{chain_for_url, picture, DialogImages};
use super::metadata_wizard::hub;
use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{input_box, new_input, select_field};

pub const FETCH_SOURCES: [(&str, &str); 6] = [
    ("auto", "Auto (IGDB first)"),
    ("igdb", "IGDB"),
    ("steam", "Steam"),
    ("store", "Store"),
    ("playnite", "Playnite"),
    ("lutris", "Lutris"),
];

pub fn field_label(field: &str) -> &'static str {
    match field {
        "description" => "Description",
        "releaseDate" => "Release date",
        "developers" => "Developers",
        "publishers" => "Publishers",
        "genres" => "Genres",
        "themes" => "Themes",
        "gameModes" => "Game modes",
        "platforms" => "Platforms",
        "series" => "Series",
        "features" => "Features",
        "criticScore" => "Score",
        "cover" => "Cover",
        "hero" => "Background",
        _ => "",
    }
}

fn source_label(source: &str) -> String {
    FETCH_SOURCES
        .iter()
        .find(|(id, _)| *id == source)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| if source == "manual" { "Manual".into() } else { source.to_string() })
}

fn format_value(md: Option<&CollectionGameMetadata>, field: &str) -> String {
    let Some(m) = md else { return String::new() };
    let join = |v: &[String]| v.join(", ");
    match field {
        "description" => m.description.clone().unwrap_or_default(),
        "releaseDate" => m.release_date.clone().unwrap_or_default(),
        "series" => m.series.clone().unwrap_or_default(),
        "criticScore" => m
            .critic_score
            .map(|n| if n.get().fract() == 0.0 { format!("{}", n.get() as i64) } else { n.get().to_string() })
            .unwrap_or_default(),
        "cover" => m.cover_url.clone().unwrap_or_default(),
        "hero" => m.hero_url.clone().unwrap_or_default(),
        "developers" => join(&m.developers),
        "publishers" => join(&m.publishers),
        "genres" => join(&m.genres),
        "themes" => join(&m.themes),
        "gameModes" => join(&m.game_modes),
        "platforms" => join(&m.platforms),
        "features" => join(&m.features),
        _ => String::new(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Preview,
    Search,
    Apply,
}

pub struct MetadataDialog {
    ctx: DialogCtx,
    service: Arc<MetadataService>,
    images: Entity<DialogImages>,
    game: Game,
    title: String,
    runner: String,
    source: String,
    source_open: bool,
    preview: Option<MetadataPreview>,
    picks: HashMap<String, String>,
    search: Entity<InputState>,
    hits: Vec<MetadataSearchHit>,
    igdb_id: Option<i64>,
    busy: Option<Busy>,
    error: String,
}

impl EventEmitter<DialogEvent> for MetadataDialog {}

impl MetadataDialog {
    pub fn new(ctx: DialogCtx, game: Game, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let search = new_input(window, cx, &game.title, "Search IGDB title");
        cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.run_search(cx);
            }
        })
        .detach();
        let igdb_id = game.metadata.as_ref().and_then(|m| m.igdb_id).map(|n| n.get() as i64);
        let auto_fetch = std::env::var("HERONITE_DEBUG_FETCH").is_ok();
        let runner = if game.runner == "zoom" { "sideload".to_string() } else { game.runner.clone() };
        let mut this = Self {
            ctx,
            service,
            images,
            title: game.title.clone(),
            runner,
            game,
            source: "auto".into(),
            source_open: false,
            preview: None,
            picks: HashMap::new(),
            search,
            hits: Vec::new(),
            igdb_id,
            busy: None,
            error: String::new(),
        };
        if auto_fetch {
            this.run_preview(None, cx);
        }
        this
    }

    fn args(&self) -> MetadataApplyArgs {
        MetadataApplyArgs {
            app_name: self.game.app_name.clone(),
            runner: self.runner.clone(),
            title: self.title.clone(),
            steam_app_id: self.game.steam_app_id.clone(),
            source: Some(self.source.clone()),
            igdb_id: self.igdb_id,
            overwrite_existing: Some(true),
            preserve_manual: Some(false),
            ..Default::default()
        }
    }

    fn run_preview(&mut self, igdb_id: Option<i64>, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        if igdb_id.is_some() {
            self.igdb_id = igdb_id;
        }
        self.busy = Some(Busy::Preview);
        self.error.clear();
        let service = self.service.clone();
        let args = self.args();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.preview_game_metadata(&args) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok(p) => {
                        let mut picks = HashMap::new();
                        for field in METADATA_FIELDS {
                            if let Some(chosen) = p.proposed.field_sources.get(field) {
                                picks.insert(field.to_string(), if chosen == "manual" { "keep".into() } else { chosen.clone() });
                            }
                        }
                        if let Some(id) = p.proposed.igdb_id {
                            this.igdb_id = Some(id.get() as i64);
                        }
                        this.picks = picks;
                        this.preview = Some(p);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::Search);
        self.error.clear();
        let typed = self.search.read(cx).value().trim().to_string();
        let query = if typed.is_empty() { self.title.clone() } else { typed };
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let hits = cx.background_spawn(async move { service.search_game_metadata(&query) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                if hits.is_empty() {
                    this.error =
                        "No IGDB matches. Check the title or add Twitch API credentials in Collection settings.".into();
                }
                this.hits = hits;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.preview.is_none() {
            return;
        }
        self.busy = Some(Busy::Apply);
        self.error.clear();
        let service = self.service.clone();
        let mut args = self.args();
        args.field_picks = Some(self.picks.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.apply_game_metadata(&args) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok(_) => cx.emit(DialogEvent::ChangedAndClose),
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn choice_tile(&mut self, field: &'static str, id: String, label: String, value: String, selected: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let is_image = field == "cover" || field == "hero";
        let detail: AnyElement = if is_image {
            let chain = chain_for_url(&self.ctx.heroic, self.ctx.lib.settings.screenshots_folder(), &value);
            if chain.is_empty() {
                div()
                    .text_size(px(12.))
                    .text_color(white())
                    .child(if value.is_empty() { "Empty" } else { "Local file" })
                    .into_any_element()
            } else {
                let (w, h) = if field == "cover" { (72., 108.) } else { (144., 54.) };
                let src = self.images.update(cx, |im, _| im.get(&format!("md:{value}"), chain, w, h));
                picture(src).w(px(w)).h(px(h)).rounded(px(4.)).into_any_element()
            }
        } else {
            div()
                .max_h(px(4.2 * 12.))
                .overflow_hidden()
                .text_size(px(12.))
                .line_height(px(16.2))
                .text_color(white())
                .child(if value.is_empty() { "Empty".to_string() } else { value })
                .into_any_element()
        };
        let pick_id = id.clone();
        div()
            .id(SharedString::from(format!("pick-{field}-{id}")))
            .flex()
            .flex_1()
            .min_w(px(160.))
            .gap(px(7.))
            .items_start()
            .p(px(8.8))
            .rounded(px(12.))
            .border_1()
            .border_color(if selected { theme::accent().into() } else { theme::white_a(0.1) })
            .bg(theme::black_a(0.2))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.picks.insert(field.to_string(), pick_id.clone());
                cx.notify();
            }))
            .child(
                div()
                    .mt(px(2.))
                    .size(px(14.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(if selected { theme::accent().into() } else { theme::white_a(0.6) })
                    .when(selected, |d| d.child(div().size(px(6.)).rounded_full().bg(theme::accent()))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(label))
                    .child(detail),
            )
    }
}

impl Render for MetadataDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let busy = self.busy.is_some();

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(help_text(
                "IGDB is tried first, then Steam, the store page, Lutris, and Playnite files. Use this for games that are not on Steam.",
            ))
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(self.title.clone()));
        if self.preview.as_ref().is_some_and(|p| !p.igdb_configured) {
            body = body.child(div().px(px(14.)).py(px(10.)).rounded(px(theme::R_INPUT)).bg(theme::accent_tint()).text_size(px(13.)).text_color(theme::text_primary()).child(
                "IGDB is not configured. Add a Twitch Client ID and Secret in Collection settings for the richest data.",
            ));
        }

        // Source + Fetch
        let sources: Vec<(SharedString, bool)> =
            FETCH_SOURCES.iter().map(|(id, label)| ((*label).into(), *id == self.source)).collect();
        let select = select_field(
            "md-source",
            Some("Source"),
            source_label(&self.source),
            false,
            self.source_open,
            sources,
            cx,
            |this, _, cx| {
                if this.busy.is_none() {
                    this.source_open = !this.source_open;
                    cx.notify();
                }
            },
            |this, i, _, cx| {
                this.source = FETCH_SOURCES[i].0.to_string();
                this.source_open = false;
                cx.notify();
            },
        );
        body = body.child(
            div()
                .flex()
                .items_end()
                .gap(px(8.))
                .child(div().w(px(200.)).child(select))
                .child(action_button(
                    "md-fetch",
                    ButtonKind::Primary,
                    if self.busy == Some(Busy::Preview) { "Fetching\u{2026}" } else { "Fetch" },
                    !busy,
                    cx.listener(|this, _, _, cx| this.run_preview(None, cx)),
                )),
        );

        // IGDB search
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(input_box(&self.search, false)))
                .child(action_button(
                    "md-search",
                    ButtonKind::Glass,
                    if self.busy == Some(Busy::Search) { "Searching\u{2026}" } else { "Find on IGDB" },
                    !busy,
                    cx.listener(|this, _, _, cx| this.run_search(cx)),
                )),
        );
        if !self.hits.is_empty() {
            let mut hits = div().flex().flex_wrap().gap(px(4.));
            for (i, hit) in self.hits.clone().into_iter().enumerate() {
                let selected = self.igdb_id.map(|n| n.to_string()) == Some(hit.id.clone());
                let id = hit.id.parse::<i64>().ok();
                let label = match hit.year {
                    Some(y) => format!("{} ({y})", hit.name),
                    None => hit.name.clone(),
                };
                hits = hits.child(
                    div()
                        .id(SharedString::from(format!("hit-{i}")))
                        .px(px(11.))
                        .py(px(4.))
                        .rounded_full()
                        .border_1()
                        .border_color(if selected { theme::accent().into() } else { theme::white_a(0.12) })
                        .bg(if selected { theme::rgb_a(165, 220, 235, 0.22).into() } else { theme::white_a(0.04) })
                        .text_size(px(13.))
                        .text_color(theme::text_primary())
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(id) = id {
                                this.run_preview(Some(id), cx);
                            }
                        }))
                        .child(label),
                );
            }
            body = body.child(hits);
        }

        // Per-field comparison
        if let Some(preview) = self.preview.clone() {
            let current = preview.current.clone().or_else(|| self.game.metadata.as_deref().cloned());
            let mut compare = div().flex().flex_col().gap(px(16.));
            for field in METADATA_FIELDS {
                let has = |m: Option<&CollectionGameMetadata>| !format_value(m, field).is_empty();
                let visible = has(current.as_ref())
                    || has(Some(&preview.proposed))
                    || preview.candidates.iter().any(|c| has(Some(&c.metadata)));
                if !visible {
                    continue;
                }
                let mut options = vec![("keep".to_string(), "Keep current".to_string(), format_value(current.as_ref(), field))];
                for c in &preview.candidates {
                    let v = format_value(Some(&c.metadata), field);
                    if !v.is_empty() {
                        options.push((c.source.clone(), source_label(&c.source), v));
                    }
                }
                if options.len() <= 1 && options[0].2.is_empty() {
                    continue;
                }
                let picked = self
                    .picks
                    .get(field)
                    .cloned()
                    .or_else(|| preview.proposed.field_sources.get(field).cloned())
                    .unwrap_or_else(|| "keep".into());
                let picked = if picked == "manual" { "keep".to_string() } else { picked };
                let mut row = div().flex().flex_wrap().gap(px(4.));
                for (id, label, value) in options {
                    let selected = picked == id;
                    row = row.child(self.choice_tile(field, id, label, value, selected, cx));
                }
                compare = compare.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(7.))
                        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(field_label(field)))
                        .child(row),
                );
            }
            body = body.child(compare);
        }
        if !self.error.is_empty() {
            body = body.child(div().px(px(14.)).py(px(10.)).rounded(px(theme::R_INPUT)).bg(theme::danger_fill()).text_size(px(13.)).text_color(theme::danger_text()).child(self.error.clone()));
        }

        let footer = div()
            .flex()
            .gap(px(16.))
            .child(action_button(
                "md-apply",
                ButtonKind::Primary,
                if self.busy == Some(Busy::Apply) { "Saving\u{2026}" } else { "Apply" },
                !busy && self.preview.is_some(),
                cx.listener(|this, _, _, cx| this.apply(cx)),
            ))
            .child(button("md-close", ButtonKind::Glass, "Close").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))));
        let entity = cx.entity().downgrade();
        let (vw, vh) = (f32::from(window.viewport_size().width), f32::from(window.viewport_size().height));
        dialog(
            DialogProps {
                id: "collection-metadata".into(),
                title: "Collection metadata".into(),
                width: 840f32.min(vw * 0.9) + 64.,
                height: Some(620f32.min(vh * 0.7)),
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
