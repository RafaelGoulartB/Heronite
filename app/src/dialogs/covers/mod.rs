//! "Covers · <game>": the game's picture of each cover family and its
//! background, and where to find another.
//!
//! - Left (`slots.rs`): a slot per family the installation keeps, in its
//!   true shape, with where the picture comes from and who draws it, and a
//!   dot (its own picture, cropped from another, none); a family that is not
//!   kept says so and can be turned on. Then the background.
//! - Right (`found.rs`): for the picked slot, the picture it shows now, then
//!   what SteamGridDB and the web found for it (in its shape), a search field
//!   and "Load more". "Use for <family>" saves the one picked.
//! - Footer: "Choose a file…", "From a screenshot…" and "Reset" (back to
//!   what the sources gave) for the slot, Close.
//!
//! Every change is saved at once (`actions.rs`, off the UI thread) and
//! announced with `DialogEvent::Changed`, so the Collection behind shows it.
//! `HERONITE_DEBUG_DIALOG=covers[:<state>]` makes the searches up (`fake.rs`).

mod actions;
mod fake;
mod found;
mod slots;
pub mod words;

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::cover_family::{kept_by_choice, CoverFamilies, CoverFamily};
use heronite_data::models::CollectionGameArt;
use heronite_i18n::tr;
use heronite_metadata::{ArtImages, ArtKind, MetadataService};

use super::art_images::DialogImages;
use super::metadata_update::hub::hub;
use super::screenshot_art::ScreenshotArtDialog;
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, icon_button_sm, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::new_input;

pub use slots::Slot;

/// What is being saved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Busy {
    /// A picture of the slot (picked, a file, a screenshot) or its reset.
    Saving,
    /// A family being kept.
    Keeping,
}

pub struct CoversDialog {
    ctx: DialogCtx,
    service: Arc<MetadataService>,
    images: Entity<DialogImages>,
    game: Game,
    runner: String,
    /// The game's pictures set by hand (`art.json`), as last saved.
    art: CollectionGameArt,
    /// Which families this installation keeps and who draws them: from the
    /// settings at once, with the console theme once read.
    families: CoverFamilies,
    slots: Vec<slots::SlotInfo>,
    slot: Slot,
    query: Entity<InputState>,
    results: ArtImages,
    /// Pictures listed before "Load more".
    visible: usize,
    /// An index of [`Self::found`].
    selected: Option<usize>,
    loading: bool,
    busy: Option<Busy>,
    error: String,
    request: u64,
    /// The searches made, by slot and words: going back to a slot shows its
    /// pictures again without asking.
    searched: HashMap<(Slot, String), ArtImages>,
    screenshot: Option<(Entity<ScreenshotArtDialog>, Subscription)>,
    fake: Option<fake::Fake>,
}

impl EventEmitter<DialogEvent> for CoversDialog {}

impl CoversDialog {
    /// Opens on `slot`, else on the family the desktop's covers draw.
    pub fn new(ctx: DialogCtx, game: Game, slot: Option<Slot>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(ctx, game, slot, None, window, cx);
        this.read_families(cx);
        this.search(cx);
        this
    }

    /// The dialog over made-up searches (`HERONITE_DEBUG_DIALOG=covers:<state>`,
    /// see `fake.rs`): nothing is asked or written. `thumbs`: pictures the
    /// Collection already has.
    pub fn made_up(ctx: DialogCtx, game: Game, state: &str, thumbs: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fake = fake::Fake::new(state, thumbs);
        let slot = fake.slot();
        let mut this = Self::build(ctx, game, slot, Some(fake), window, cx);
        this.read_families(cx);
        this.search(cx);
        this.debug_state();
        this
    }

    fn build(
        ctx: DialogCtx,
        game: Game,
        slot: Option<Slot>,
        fake: Option<fake::Fake>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let query = new_input(window, cx, &game.title, "");
        cx.subscribe_in(&query, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        })
        .detach();
        let settings = &ctx.lib.settings;
        let families = CoverFamilies::compute(settings.cover_aspect, &kept_by_choice(&settings.metadata), None);
        let slot = slot.unwrap_or(Slot::Cover(settings.cover_aspect.family()));
        let runner = if game.runner == "zoom" { "sideload".to_string() } else { game.runner.clone() };
        let mut this = Self {
            service,
            images,
            runner,
            art: game.art.as_deref().cloned().unwrap_or_default(),
            game,
            ctx,
            families,
            slots: Vec::new(),
            slot,
            query,
            results: ArtImages::default(),
            visible: found::PAGE,
            selected: None,
            loading: false,
            busy: None,
            error: String::new(),
            request: 0,
            searched: HashMap::new(),
            screenshot: None,
            fake,
        };
        this.compute_slots();
        this
    }

    /// The pictures set by hand, as last saved here (for a host that shows
    /// them too).
    pub fn art(&self) -> &CollectionGameArt {
        &self.art
    }

    /// Esc closes the screenshot picker first.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.screenshot.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// Whether the picked slot's family is kept (the background always is).
    fn kept(&self, slot: Slot) -> bool {
        match slot {
            Slot::Cover(f) => self.families.is_kept(f),
            Slot::Background => true,
        }
    }

    fn kind(&self) -> ArtKind {
        self.slot.kind()
    }

    /// The label of the primary button.
    fn use_label(&self) -> &'static str {
        match self.slot {
            Slot::Cover(CoverFamily::Tall) => tr!("Use for Tall"),
            Slot::Cover(CoverFamily::Square) => tr!("Use for Square"),
            Slot::Cover(CoverFamily::Wide) => tr!("Use for Wide"),
            Slot::Background => tr!("Use for Background"),
        }
    }

    fn render_footer(&mut self, cx: &mut Context<Self>) -> Div {
        let kept = self.kept(self.slot);
        let idle = self.busy.is_none();
        let custom = self.kind().of_art(&self.art).is_some_and(|u| !u.is_empty());
        let saving = self.busy == Some(Busy::Saving);
        div()
            .flex()
            .flex_1()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(theme::text_dim())
                    .child(tr!("Each shape keeps its own picture. A view asks for the shape it draws.")),
            )
            .child(icon_button_sm(
                "cv-file",
                ButtonKind::Ghost,
                Icon::FolderOpen,
                tr!("Choose a file\u{2026}"),
                idle && kept,
                cx.listener(|this, _, _, cx| this.pick_file(cx)),
            ))
            .child(icon_button_sm(
                "cv-shot",
                ButtonKind::Ghost,
                Icon::PhotoCamera,
                tr!("From a screenshot\u{2026}"),
                idle && kept,
                cx.listener(|this, _, _, cx| this.open_screenshot(cx)),
            ))
            .child(action_button("cv-reset", ButtonKind::Ghost, tr!("Reset"), idle && custom, cx.listener(|this, _, _, cx| this.reset(cx))))
            .child(button("cv-close", ButtonKind::Glass, tr!("Close")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "cv-use",
                ButtonKind::Primary,
                if saving && self.selected.is_some() { tr!("Downloading\u{2026}") } else { self.use_label() },
                idle && kept && self.selected.is_some(),
                cx.listener(|this, _, _, cx| this.use_selected(None, cx)),
            ))
    }
}

/// Width of the slots' column.
const SLOTS_W: f32 = 300.;
/// Height of the two columns: the found pictures scroll inside.
const COLUMNS_H: f32 = 500.;

impl Render for CoversDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let area = crate::ui::content_size(window);
        let (vw, vh) = (f32::from(area.width), f32::from(area.height));
        let columns_h = COLUMNS_H.min(vh * 0.62).max(300.);
        let body = div()
            .flex()
            .gap(px(22.))
            .h(px(columns_h))
            .child(self.render_slots(cx).w(px(SLOTS_W)).flex_none())
            .child(self.render_found(cx).flex_1().min_w_0());
        let footer = self.render_footer(cx);
        let entity = cx.entity().downgrade();
        let title = tr!("Covers \u{b7} {game}", game = self.game.title.clone());
        let main = dialog(
            DialogProps { id: "collection-covers".into(), title: title.into(), width: 1040f32.min(vw * 0.96), height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        );
        let nested: Option<AnyView> = self.screenshot.as_ref().map(|(v, _)| v.clone().into());
        div().absolute().inset_0().child(main).children(nested)
    }
}
