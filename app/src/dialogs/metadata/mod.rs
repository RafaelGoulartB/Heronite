//! "Metadata" of one game: how each source found it and, field by field,
//! what will be saved and from where.
//!
//! - Left, "Found as" (`found_as.rs`): a row per details source with the
//!   game it matched (pinned or left out because it is another game), or
//!   that it found nothing, could not be reached or was not asked (folded
//!   into one line per reason). "Change" (`change.rs`) searches one source
//!   and pins the game picked; "Automatic" unpins it.
//! - Right (`fields.rs`): a row per field with a value: the pictures first,
//!   each source's as a tile in the picture's shape with its size and what
//!   to know before picking it (`pictures.rs`); then what will be saved of
//!   the other fields, where it comes from (segments with the source names
//!   when sources differ, else a line) and a lock.
//! - Footer: "Everything from…", Close and Apply, which saves the picks,
//!   pins and locks of what is on screen without asking the sources again
//!   (`actions.rs`).
//!
//! The sources are asked when it opens and again after a pin changes or
//! "Search every source again". `HERONITE_DEBUG_DIALOG=metadata:<state>`
//! makes everything up (`fake.rs`).

mod actions;
mod change;
mod fake;
mod fields;
mod found_as;
mod pictures;

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::cover_family::CoverFamily;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata};
use heronite_i18n::tr;
use heronite_metadata::{MetadataPreview, MetadataService};
use indexmap::IndexMap;

use super::art_images::{chain_for_url, picture, DialogImages};
use super::covers::{CoversDialog, Slot};
use super::metadata_update::hub::{hub, MetadataHub};
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::select_field;
use crate::ui::icon;

pub fn field_label(field: &str) -> &'static str {
    match field {
        "description" => tr!("Description"),
        "releaseDate" => tr!("Release date"),
        "developers" => tr!("Developers"),
        "publishers" => tr!("Publishers"),
        "genres" => tr!("Genres"),
        "themes" => tr!("Themes"),
        "gameModes" => tr!("Game modes"),
        "platforms" => tr!("Platforms"),
        "series" => tr!("Series"),
        "features" => tr!("Features"),
        "criticScore" => tr!("Score"),
        "ageRatings" => tr!("Age ratings"),
        "cover" => tr!("Cover \u{b7} Tall"),
        "coverSquare" => tr!("Cover \u{b7} Square"),
        "coverWide" => tr!("Cover \u{b7} Wide"),
        "hero" => tr!("Background"),
        _ => "",
    }
}

/// `field` of `md` as one line of text (a picture's address for `cover`
/// and `hero`); empty when it has none.
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
        "coverSquare" => m.cover_square_url.clone().unwrap_or_default(),
        "coverWide" => m.cover_wide_url.clone().unwrap_or_default(),
        "hero" => m.hero_url.clone().unwrap_or_default(),
        "developers" => join(&m.developers),
        "publishers" => join(&m.publishers),
        "genres" => join(&m.genres),
        "themes" => join(&m.themes),
        "gameModes" => join(&m.game_modes),
        "platforms" => join(&m.platforms),
        "features" => join(&m.features),
        "ageRatings" => {
            m.age_ratings.iter().map(|r| format!("{} {}", r.board().map_or("", |b| b.name()), r.badge())).collect::<Vec<_>>().join(", ")
        }
        _ => String::new(),
    }
}

/// A field drawn as a picture.
fn is_picture(field: &str) -> bool {
    matches!(field, "cover" | "coverSquare" | "coverWide" | "hero")
}

/// The pick that keeps what the game has.
const KEEP: &str = "keep";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Preview,
    Apply,
}

pub struct MetadataDialog {
    ctx: DialogCtx,
    hub: Entity<MetadataHub>,
    service: Arc<MetadataService>,
    images: Entity<DialogImages>,
    game: Game,
    title: String,
    runner: String,
    /// Where the game comes from, under its title.
    origin: String,
    preview: Option<MetadataPreview>,
    busy: Option<Busy>,
    error: Option<String>,
    /// The user's picks: field -> source id or [`KEEP`]. A field not here
    /// takes what the merge proposes ([`Self::default_pick`]).
    chosen: IndexMap<String, String>,
    /// Pins changed here: source -> its game, or `None` for "Automatic".
    pins: IndexMap<String, Option<String>>,
    /// Locks changed here.
    locks: IndexMap<String, bool>,
    /// "Change": the search of one source.
    change: Option<change::Change>,
    everything_open: bool,
    /// The segment under the pointer (field, source): its value shows.
    hover: Option<(&'static str, String)>,
    /// What Apply saved, for a host that shows the game's details itself.
    applied: Option<CollectionGameMetadata>,
    fake: Option<fake::FakePreview>,
    preview_task: Option<Task<()>>,
    /// The cover families this installation keeps: a row each.
    kept: Vec<CoverFamily>,
    /// "More options…": the Covers dialog over this one.
    covers: Option<(Entity<CoversDialog>, Subscription)>,
    /// The pictures set by hand there, once it changed one.
    covers_art: Option<CollectionGameArt>,
    fields_scroll: ScrollHandle,
    /// The tiles of each picture row (`pictures.rs`), for its arrows.
    tile_scrolls: std::collections::HashMap<&'static str, ScrollHandle>,
    /// The sizes of the pictures read (`pictures.rs`), by address; `None`:
    /// not a picture whose size can be read.
    sizes: std::collections::HashMap<String, Option<(u32, u32)>>,
}

impl EventEmitter<DialogEvent> for MetadataDialog {}

impl MetadataDialog {
    pub fn new(ctx: DialogCtx, game: Game, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(ctx, game, None, cx);
        this.read_families(cx);
        this.run_preview(cx);
        this
    }

    /// The dialog over a made-up preview (`HERONITE_DEBUG_DIALOG=metadata:<state>`,
    /// see `fake.rs`): nothing is asked or written. `thumbs`: cover
    /// addresses the made-up sources propose.
    pub fn made_up(ctx: DialogCtx, game: Game, state: &str, thumbs: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(ctx, game, Some(fake::FakePreview::new(state, thumbs)), cx);
        this.read_families(cx);
        this.run_preview(cx);
        this.debug_state(state, window, cx);
        this
    }

    fn build(ctx: DialogCtx, game: Game, fake: Option<fake::FakePreview>, cx: &mut Context<Self>) -> Self {
        let hub = hub(cx);
        let service = hub.read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let runner = if game.runner == "zoom" { "sideload".to_string() } else { game.runner.clone() };
        Self {
            ctx,
            hub,
            service,
            images,
            title: game.title.clone(),
            runner,
            origin: origin(&game),
            game,
            preview: None,
            busy: None,
            error: None,
            chosen: IndexMap::new(),
            pins: IndexMap::new(),
            locks: IndexMap::new(),
            change: None,
            everything_open: false,
            hover: None,
            applied: None,
            fake,
            preview_task: None,
            kept: vec![CoverFamily::Tall],
            covers: None,
            covers_art: None,
            fields_scroll: ScrollHandle::new(),
            tile_scrolls: Default::default(),
            sizes: Default::default(),
        }
    }

    /// Reads which cover families are kept (a row each).
    fn read_families(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let kept = cx.background_spawn(async move { service.cover_families().kept() }).await;
            this.update(cx, |this, cx| {
                this.kept = kept;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The pictures set by hand in "More options…", once it changed one
    /// (for a host that shows them).
    pub fn covers_art(&self) -> Option<&CollectionGameArt> {
        self.covers_art.as_ref()
    }

    /// "More options…" of a picture's row: the Covers dialog over this one,
    /// on that picture's slot.
    fn open_covers(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        let (ctx, game) = (self.ctx.clone(), self.game.clone());
        let view = cx.new(|cx| CoversDialog::new(ctx, game, Some(slot), window, cx));
        let sub = cx.subscribe(&view, |this: &mut Self, view, ev: &DialogEvent, cx| {
            if matches!(ev, DialogEvent::Changed | DialogEvent::ChangedAndClose) {
                this.covers_art = Some(view.read(cx).art().clone());
                cx.emit(DialogEvent::Changed);
            }
            if matches!(ev, DialogEvent::Close | DialogEvent::ChangedAndClose) {
                this.covers = None;
            }
            cx.notify();
        });
        self.covers = Some((view, sub));
        cx.notify();
    }

    /// The metadata Apply saved (set before `ChangedAndClose`).
    pub fn applied(&self) -> Option<&CollectionGameMetadata> {
        self.applied.as_ref()
    }

    /// Esc closes "Change" or the open select first.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((view, _)) = &self.covers {
            if !view.update(cx, |v, cx| v.on_escape(cx)) {
                self.covers = None;
            }
        } else if self.everything_open {
            self.everything_open = false;
        } else if self.change.is_some() {
            self.change = None;
        } else {
            return false;
        }
        cx.notify();
        true
    }

    /// What the game has now: the preview's record, else the library's.
    fn current(&self) -> Option<&CollectionGameMetadata> {
        match &self.preview {
            Some(p) => p.current.as_ref(),
            None => self.game.metadata.as_deref(),
        }
    }

    /// A small picture of `url` (`w`×`h`), loaded on demand.
    fn thumb(&mut self, url: &str, w: f32, h: f32, cx: &mut Context<Self>) -> Div {
        let chain = chain_for_url(&self.ctx.data_paths, self.ctx.lib.settings.screenshots_folder(), url);
        if chain.is_empty() {
            return div().flex().items_center().justify_center().w(px(w)).h(px(h)).rounded(px(4.)).bg(theme::ink_a(0.05)).child(icon(
                Icon::Image,
                px(14.),
                theme::text_dim(),
            ));
        }
        let src = self.images.update(cx, |im, _| im.get(&format!("md:{url}"), chain, w, h));
        picture(src).w(px(w)).h(px(h)).rounded(px(4.))
    }

    /// The game's cover, title and where it comes from.
    fn render_head(&mut self, cx: &mut Context<Self>) -> Div {
        let cover =
            self.current().and_then(|m| m.cover_url.clone()).or_else(|| self.game.metadata.as_ref().and_then(|m| m.cover_url.clone()));
        let cover = match cover {
            Some(url) => self.thumb(&url, 40., 54., cx),
            None => div().w(px(40.)).h(px(54.)).rounded(px(4.)).bg(theme::ink_a(0.05)),
        };
        let updated = self.current().and_then(|m| m.fetched_at.as_deref()).and_then(ago);
        let line = [Some(self.origin.clone()).filter(|o| !o.is_empty()), updated.map(|when| tr!("last updated {when}", when))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        div().flex().items_center().gap(px(14.)).mb(px(16.)).child(cover).child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(16.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_bright())
                        .child(self.title.clone()),
                )
                .when(!line.is_empty(), |d| d.child(div().truncate().text_size(px(12.5)).text_color(theme::text_muted()).child(line))),
        )
    }

    fn render_footer(&mut self, cx: &mut Context<Self>) -> Div {
        let sources = self.everything_sources();
        let options: Vec<(SharedString, bool)> = sources.iter().map(|(_, name)| (name.clone().into(), false)).collect();
        let everything = select_field(
            "md-everything",
            None,
            tr!("Everything from\u{2026}"),
            sources.is_empty() || self.busy.is_some(),
            self.everything_open,
            options,
            cx,
            |this, _, cx| {
                this.everything_open = !this.everything_open;
                cx.notify();
            },
            move |this, i, _, cx| {
                if let Some((id, _)) = this.everything_sources().get(i).cloned() {
                    this.take_everything_from(&id);
                }
                this.everything_open = false;
                cx.notify();
            },
        );
        let applying = self.busy == Some(Busy::Apply);
        div()
            .flex()
            .flex_1()
            .items_center()
            .gap(px(10.))
            .child(
                div().flex().flex_1().min_w_0().items_center().gap(px(6.)).child(icon(Icon::Lock, px(12.), theme::text_dim())).child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(theme::text_dim())
                        .child(tr!("A locked field stays as it is in every later update.")),
                ),
            )
            .child(div().w(px(220.)).flex_none().child(everything))
            .child(button("md-close", ButtonKind::Glass, tr!("Close")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "md-apply",
                ButtonKind::Primary,
                if applying { tr!("Saving\u{2026}") } else { tr!("Apply") },
                self.can_apply(),
                cx.listener(|this, _, _, cx| this.apply(cx)),
            ))
    }
}

/// Height of the two columns: they scroll inside, so many sources or fields
/// never make the dialog grow.
const COLUMNS_H: f32 = 517.;

impl Render for MetadataDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let area = crate::ui::content_size(window);
        let (vw, vh) = (f32::from(area.width), f32::from(area.height));
        let columns_h = COLUMNS_H.min(vh * 0.68).max(260.);
        let left = match self.change.is_some() {
            true => self.render_change(cx),
            false => self.render_found_as(cx),
        };
        let body =
            div().flex().flex_col().child(self.render_head(cx)).child(
                div().flex().gap(px(20.)).h(px(columns_h)).child(left.w(px(310.)).flex_none()).child(self.render_fields(window, cx)),
            );
        let footer = self.render_footer(cx);
        let entity = cx.entity().downgrade();
        let main = dialog(
            DialogProps { id: "collection-metadata".into(), title: tr!("Metadata").into(), width: 1060f32.min(vw * 0.94), height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        );
        let covers: Option<AnyView> = self.covers.as_ref().map(|(v, _)| v.clone().into());
        div().absolute().inset_0().child(main).children(covers)
    }
}

/// Where a game comes from, as the Collection words it: an emulated game's
/// platform and emulator, else its library ("Steam", "GOG").
pub(crate) fn origin(g: &heronite_data::CollectionGame) -> String {
    let store = heronite_data::copies::store_label(g).to_string();
    let emulated = g.meta.as_deref().is_some_and(|m| m.launch_kind == "emulator");
    let platform = g.facets[heronite_data::collection::FacetKind::Platform.index()].first().map(|f| f.value.clone());
    match platform.filter(|_| emulated) {
        Some(p) if p != store => format!("{p} \u{b7} {store}"),
        _ => store,
    }
}

/// "3 weeks ago" for an ISO 8601 time; `None` when it cannot be read.
fn ago(iso: &str) -> Option<String> {
    let at = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    Some(crate::services::update::ago_text(at.timestamp().max(0) as u64))
}
