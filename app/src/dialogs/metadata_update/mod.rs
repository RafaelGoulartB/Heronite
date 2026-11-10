//! "Update metadata": updating several games in three steps of one dialog,
//! with a stepper at the top (a step done can be clicked again).
//!
//! 1. Games (`games.rs`): the selection, the games on screen, the whole
//!    Collection or the games that need attention.
//! 2. What to update (`what.rs`): how (fill / refresh / replace), which
//!    fields and from which sources.
//! 3. Update (`results.rs`): the job's progress and each game's result, with
//!    Stop, Undo and "Update again".
//!
//! The job itself belongs to [`hub::MetadataHub`]: hiding the dialog leaves
//! it running (the header pill shows it), and the dialog opened while a job
//! runs goes straight to step 3 of that job.

mod fake;
pub mod families;
pub mod family_notice;
mod games;
pub mod hub;
mod results;
pub mod source_downloads;
mod what;
pub(crate) mod words;

use std::collections::{HashMap, HashSet};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_metadata::{BulkItem, GameCounts, SourceDescriptor, UpdateMode};

use self::hub::{hub, HubEvent, MetadataHub};
use self::words::Bucket;
use super::{DialogCtx, DialogEvent};
use crate::services::widgets::Covers;
use crate::theme;
use crate::ui::dialog::{dialog, DialogProps};

/// The games the dialog can work on, gathered by the Collection when it opens.
#[derive(Default)]
pub struct UpdateGames {
    /// The selected games, when opened from a selection.
    pub selected: Option<Vec<BulkItem>>,
    /// What the list shows now.
    pub shown: Vec<BulkItem>,
    /// Every game of the Collection.
    pub all: Vec<BulkItem>,
    /// The list's search and filters as words, when any is on.
    pub filters: Option<String>,
    /// Cover chains by metadata key (`runner:appName`).
    pub covers: HashMap<String, Vec<ImageRef>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Games,
    What,
    Update,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    Selected,
    Shown,
    All,
    Attention,
}

pub struct MetadataUpdate {
    hub: Entity<MetadataHub>,
    step: Step,
    games: UpdateGames,
    scope: Scope,
    /// The games that need attention among all, once read.
    attention: Option<Vec<BulkItem>>,
    /// Counts of the whole Collection, once read.
    all_counts: Option<GameCounts>,
    /// Counts of the chosen games, once read. Read again, with the two
    /// above, when the data changed (a job ended, an undo).
    counts: Option<(Scope, GameCounts)>,
    /// The sources a job may ask, in the user's order, once read.
    sources: Option<Vec<SourceDescriptor>>,
    /// The cover families this installation keeps (a tick each), once read.
    kept: Vec<CoverFamily>,
    mode: UpdateMode,
    fields_off: HashSet<String>,
    sources_off: HashSet<String>,
    /// Replace everything is asked about before it starts (once).
    confirm_replace: bool,
    replace_ok: bool,
    /// The job this dialog started (step 3 follows it while it waits).
    started: Option<u64>,
    filter: Option<Bucket>,
    confirm_undo: bool,
    cache: results::Cache,
    list: UniformListScrollHandle,
    covers: Covers,
    _tasks: Vec<Task<()>>,
    counts_task: Option<Task<()>>,
    _subs: [Subscription; 2],
}

impl EventEmitter<DialogEvent> for MetadataUpdate {}

impl MetadataUpdate {
    pub fn new(ctx: DialogCtx, games: UpdateGames, cx: &mut Context<Self>) -> Self {
        let hub = hub(cx);
        let sub = cx.observe(&hub, |_, _, cx| cx.notify());
        // A job ended or was undone: what the games lack changed.
        let data = cx.subscribe(&hub, |this: &mut Self, _, ev: &HubEvent, cx| {
            if matches!(ev, HubEvent::DataChanged) {
                this.all_counts = None;
                this.counts = None;
                this.load(cx);
            }
        });
        let running = hub.read(cx).running();
        let scope = if games.selected.is_some() { Scope::Selected } else { Scope::Shown };
        let covers = Covers::new(&ctx.data_paths, &ctx.app, cx, |this: &mut Self| &mut this.covers);
        let mut this = Self {
            hub,
            step: if running { Step::Update } else { Step::Games },
            games,
            scope,
            attention: None,
            all_counts: None,
            counts: None,
            sources: None,
            kept: vec![CoverFamily::Tall],
            mode: UpdateMode::Fill,
            fields_off: HashSet::new(),
            sources_off: HashSet::new(),
            confirm_replace: false,
            replace_ok: false,
            started: None,
            filter: None,
            confirm_undo: false,
            cache: Default::default(),
            list: UniformListScrollHandle::new(),
            covers,
            _tasks: Vec::new(),
            counts_task: None,
            _subs: [sub, data],
        };
        this.load(cx);
        this
    }

    /// Reads, off the UI thread, what step 1 shows of the whole Collection
    /// and the sources a job may ask; then the chosen games' counts.
    fn load(&mut self, cx: &mut Context<Self>) {
        let service = self.hub.read(cx).service.clone();
        let all = self.games.all.clone();
        let task = cx.spawn(async move |this, cx| {
            let read = cx
                .background_spawn(async move {
                    let sources = service.bulk_sources(&service.metadata_settings());
                    let attention = service.needs_attention(&all).unwrap_or_default();
                    let counts = service.bulk_counts(&all).ok();
                    (sources, attention, counts, service.cover_families().kept())
                })
                .await;
            this.update(cx, |this, cx| {
                let (sources, attention, counts, kept) = read;
                this.sources = Some(sources);
                this.kept = kept;
                this.attention = Some(attention);
                this.all_counts = counts;
                // Counted on the list read before, if any.
                this.counts = this.counts.take().filter(|(s, _)| *s != Scope::Attention);
                this.count_chosen(cx);
                cx.notify();
            })
            .ok();
        });
        self._tasks.push(task);
        self.count_chosen(cx);
    }

    /// The games of `scope` (none while the ones needing attention load).
    fn games_of(&self, scope: Scope) -> &[BulkItem] {
        match scope {
            Scope::Selected => self.games.selected.as_deref().unwrap_or_default(),
            Scope::Shown => &self.games.shown,
            Scope::All => &self.games.all,
            Scope::Attention => self.attention.as_deref().unwrap_or_default(),
        }
    }

    fn chosen(&self) -> &[BulkItem] {
        self.games_of(self.scope)
    }

    /// The chosen games' counts (missing fields, each source's reach), read
    /// off the UI thread when the choice changes.
    fn count_chosen(&mut self, cx: &mut Context<Self>) {
        if self.counts.as_ref().is_some_and(|(s, _)| *s == self.scope) {
            return;
        }
        if self.scope == Scope::All {
            if let Some(all) = self.all_counts.clone() {
                self.counts = Some((Scope::All, all));
            }
            return;
        }
        if self.scope == Scope::Attention && self.attention.is_none() {
            return;
        }
        let scope = self.scope;
        let service = self.hub.read(cx).service.clone();
        let games = self.chosen().to_vec();
        self.counts_task = Some(cx.spawn(async move |this, cx| {
            let counts = cx.background_spawn(async move { service.bulk_counts(&games) }).await;
            this.update(cx, |this, cx| {
                match counts {
                    Ok(c) if this.scope == scope => this.counts = Some((scope, c)),
                    Ok(_) => {}
                    Err(err) => log::warn!("metadata counts: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.count_chosen(cx);
        cx.notify();
    }

    fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = step;
        self.confirm_replace = false;
        self.confirm_undo = false;
        cx.notify();
    }

    /// Whether step 3 has a job to show.
    fn has_job(&self, cx: &App) -> bool {
        self.started.is_some() || self.hub.read(cx).progress.job != 0
    }

    /// Hides the dialog; a running job goes on (the header pill shows it).
    fn hide(&mut self, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Close);
    }

    /// Esc: closes a question first. True when it did.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.confirm_replace || self.confirm_undo {
            self.confirm_replace = false;
            self.confirm_undo = false;
            cx.notify();
            return true;
        }
        false
    }

    /// Shows the last job's results.
    pub fn show_results(&mut self, cx: &mut Context<Self>) {
        self.go(Step::Update, cx);
    }

    /// Opens on step 1 with "Games that need attention" chosen.
    pub fn show_attention(&mut self, cx: &mut Context<Self>) {
        self.go(Step::Games, cx);
        self.set_scope(Scope::Attention, cx);
    }

    /// `HERONITE_DEBUG_DIALOG=wizard:<step>[:<state>]`: `2`, `2:replace`
    /// (Replace everything with its question), `3`, `3:undo` (the undo's
    /// question).
    pub fn debug_state(&mut self, state: &str, cx: &mut Context<Self>) {
        match state {
            "2" => self.step = Step::What,
            "2:replace" => {
                self.step = Step::What;
                self.mode = UpdateMode::Replace;
                self.confirm_replace = true;
            }
            "3" => self.step = Step::Update,
            "3:undo" => {
                self.step = Step::Update;
                self.confirm_undo = true;
            }
            _ => self.step = Step::Games,
        }
        cx.notify();
    }

    /// The stepper: three numbered pills; a step can be clicked when it can
    /// be shown.
    fn render_stepper(&self, cx: &mut Context<Self>) -> Div {
        let steps = [(Step::Games, tr!("Games")), (Step::What, tr!("What to update")), (Step::Update, tr!("Update"))];
        let has_job = self.has_job(cx);
        let mut row = div().flex().items_center().gap(px(6.)).mb(px(16.));
        for (i, (step, label)) in steps.into_iter().enumerate() {
            let active = self.step == step;
            let reachable = !active && (step != Step::Update || has_job) && (step == Step::Games || !self.chosen().is_empty());
            let number = div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded_full()
                .text_size(px(11.))
                .font_weight(theme::W_BOLD)
                .when(active, |d| d.bg(theme::accent()).text_color(theme::on_accent()))
                .when(!active, |d| d.bg(theme::ink_a(0.08)).text_color(theme::text_muted()))
                .child((i + 1).to_string());
            row = row.child(
                div()
                    .id(("mu-step", i))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(32.))
                    .pl(px(6.))
                    .pr(px(12.))
                    .rounded_full()
                    .text_size(px(13.5))
                    .when(active, |d| d.bg(theme::accent_tint()).text_color(theme::text_bright()))
                    .when(!active, |d| d.text_color(theme::text_muted()))
                    .when(reachable, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme::ink_a(0.06)).text_color(theme::text_primary()))
                            .on_click(cx.listener(move |this, _, _, cx| this.go(step, cx)))
                    })
                    .when(!reachable && !active, |d| d.opacity(0.6))
                    .child(number)
                    .child(label),
            );
        }
        row
    }
}

impl Render for MetadataUpdate {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.covers.drop_evicted(window);
        let (body, footer) = match self.step {
            Step::Games => (self.render_games(window, cx), self.games_footer(cx)),
            Step::What => (self.render_what(cx), self.what_footer(cx)),
            Step::Update => (self.render_results(window, cx), self.results_footer(cx)),
        };
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "metadata-update".into(), title: tr!("Update metadata").into(), width: 760., height: None },
            div().flex().flex_col().child(self.render_stepper(cx)).child(body).into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |this, cx| this.hide(cx)).ok();
            },
        )
    }
}

/// A round tick: filled with a check when on, an empty ring when off.
fn tick(on: bool) -> Div {
    let d = div().size(px(18.)).flex_none().rounded_full().flex().items_center().justify_center().border_1();
    if on {
        d.bg(theme::accent()).border_color(theme::accent()).child(crate::ui::icon(crate::icons::Icon::Check, px(13.), theme::page_bg()))
    } else {
        d.border_color(theme::ink_a(0.32))
    }
}

/// The footer's left side: a muted line that takes the room left.
fn footer_note(text: impl Into<SharedString>) -> Div {
    div().flex_1().min_w_0().text_size(px(12.5)).text_color(theme::text_dim()).child(text.into())
}
