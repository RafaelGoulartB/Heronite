//! `CollectionGameArtDialog` ("Game settings"): Images, Details and Launch
//! tabs for one game.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::{collect_tag_options, Game};
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata, LocalGameMeta, LocalMetaFile};
use heronite_data::{art, json_store, ArtContext, HeroicPaths};
use heronite_images::ImageRef;
use heronite_launch::native::{detect_executable_kind, effective_platform, ExecutableKind, Platform};
use heronite_launch::SystemProbe;
use heronite_metadata::{ArtKind, DetailsPatch, DetailsResult, MetadataService};

use super::art_images::{chain_for_url, picture, DialogImages};
use super::heroic_settings::{debug_dialog_height, GameFacts, HeroicSettingsPanel, Section, Target};
use super::metadata_wizard::hub;
use super::screenshot_art::ScreenshotArtDialog;
use super::web_image::{ArtEvent, WebImageDialog};
use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, tabs, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{input_box, labeled, new_input, new_textarea, select_field, textarea_box, TagEvent, TagField};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Images,
    Details,
    Launch,
    /// Heroic's game settings (`heroic_settings`).
    Wine,
    Other,
    Advanced,
    Saves,
    Gamescope,
}

impl Tab {
    /// The Heroic settings section this tab shows.
    fn section(self) -> Option<Section> {
        match self {
            Tab::Wine => Some(Section::Wine),
            Tab::Other => Some(Section::Other),
            Tab::Advanced => Some(Section::Advanced),
            Tab::Saves => Some(Section::Saves),
            Tab::Gamescope => Some(Section::Gamescope),
            _ => None,
        }
    }

    fn of_section(s: Section) -> Tab {
        match s {
            Section::Wine => Tab::Wine,
            Section::Other => Tab::Other,
            Section::Advanced => Tab::Advanced,
            Section::Saves => Tab::Saves,
            Section::Gamescope => Tab::Gamescope,
            Section::General => Tab::Images,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Images => "Images",
            Tab::Details => "Details",
            Tab::Launch => "Launch",
            Tab::Wine => "Wine",
            Tab::Other => "Other",
            Tab::Advanced => "Advanced",
            Tab::Saves => "Cloud saves",
            Tab::Gamescope => "Gamescope",
        }
    }

    /// The tab Heroic opens first for this game (its Wine or Other tab), or
    /// Images when the game has no Heroic settings.
    pub fn heroic_start(game: &Game) -> Tab {
        GameFacts::of(game).start_section().map(Tab::of_section).unwrap_or(Tab::Images)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Art(ArtKind),
    Details,
    Launch,
}

#[derive(Clone, PartialEq, Default)]
struct Form {
    title: String,
    description: String,
    release_date: String,
    series: String,
    developers: Vec<String>,
    publishers: Vec<String>,
    genres: Vec<String>,
    themes: Vec<String>,
    game_modes: Vec<String>,
    platforms: Vec<String>,
    features: Vec<String>,
    score: String,
    websites: String,
    notes: String,
}

fn form_from(title: &str, md: Option<&CollectionGameMetadata>, meta: Option<&LocalGameMeta>) -> Form {
    let md_str = |v: &Option<String>| v.clone().unwrap_or_default();
    Form {
        title: md.and_then(|m| m.title.as_deref()).map(str::trim).filter(|t| !t.is_empty()).unwrap_or(title).to_string(),
        description: md.map(|m| md_str(&m.description)).unwrap_or_default(),
        release_date: md.map(|m| md_str(&m.release_date)).unwrap_or_default(),
        series: md.map(|m| md_str(&m.series)).unwrap_or_default(),
        developers: md.map(|m| m.developers.clone()).unwrap_or_default(),
        publishers: md.map(|m| m.publishers.clone()).unwrap_or_default(),
        genres: md.map(|m| m.genres.clone()).unwrap_or_default(),
        themes: md.map(|m| m.themes.clone()).unwrap_or_default(),
        game_modes: md.map(|m| m.game_modes.clone()).unwrap_or_default(),
        platforms: md.map(|m| m.platforms.clone()).unwrap_or_default(),
        features: md.map(|m| m.features.clone()).unwrap_or_default(),
        score: md
            .and_then(|m| m.critic_score)
            .map(|n| if n.get().fract() == 0.0 { format!("{}", n.get() as i64) } else { n.get().to_string() })
            .unwrap_or_default(),
        websites: md.map(|m| m.websites.join(", ")).unwrap_or_default(),
        notes: md.and_then(|m| m.notes.clone()).filter(|n| !n.is_empty()).or_else(|| meta.and_then(|m| m.notes.clone())).unwrap_or_default(),
    }
}

fn split_list(value: &str) -> Vec<String> {
    value.split([',', ';', '\n']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

const TAG_FIELDS: [(&str, &str, bool); 8] = [
    ("series", "Series", false),
    ("developers", "Developers", true),
    ("publishers", "Publishers", true),
    ("genres", "Genres", true),
    ("themes", "Themes", true),
    ("gameModes", "Game modes", true),
    ("platforms", "Platforms", true),
    ("features", "Features", true),
];

struct Details {
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    release: Entity<InputState>,
    score: Entity<InputState>,
    websites: Entity<InputState>,
    notes: Entity<TextareaState>,
    /// In [`TAG_FIELDS`] order.
    tags: Vec<Entity<TagField>>,
}

struct LaunchState {
    executable: Entity<InputState>,
    run_as: String,
    run_as_open: bool,
    initial_exe: String,
    initial_run_as: String,
    exists: bool,
    detected: ExecutableKind,
    effective: Platform,
    saved: bool,
}

enum Nested {
    Web(Entity<WebImageDialog>, #[allow(dead_code)] Subscription),
    Screenshot(Entity<ScreenshotArtDialog>, #[allow(dead_code)] Subscription),
}

pub struct GameSettingsDialog {
    ctx: DialogCtx,
    service: Arc<MetadataService>,
    images: Entity<DialogImages>,
    game: Game,
    title: String,
    tab: Tab,
    art: CollectionGameArt,
    default_cover: Vec<ImageRef>,
    default_hero: Vec<ImageRef>,
    busy: Option<Busy>,
    error: String,
    nested: Option<Nested>,
    details: Details,
    initial: Form,
    launch: Option<LaunchState>,
    /// Heroic settings: the view and the tabs this game has.
    heroic: Option<(Entity<HeroicSettingsPanel>, Vec<Section>)>,
}

impl EventEmitter<DialogEvent> for GameSettingsDialog {}

fn detect(path: &str) -> (bool, ExecutableKind) {
    (!path.is_empty() && Path::new(path).exists(), detect_executable_kind(path, &SystemProbe))
}

impl GameSettingsDialog {
    pub fn new(ctx: DialogCtx, game: Game, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();

        // Default art = what the game shows without a custom image.
        let shots = ctx.lib.settings.screenshots_folder().map(|p| p.to_path_buf());
        let art_ctx = ArtContext::new(&ctx.heroic, shots.as_deref());
        let old_cover = game.art.as_deref().and_then(|a| a.cover_url.clone()).map(|u| chain_for_url(&ctx.heroic, shots.as_deref(), &u)).unwrap_or_default();
        let old_hero = game.art.as_deref().and_then(|a| a.hero_url.clone()).map(|u| chain_for_url(&ctx.heroic, shots.as_deref(), &u)).unwrap_or_default();
        let default_cover: Vec<ImageRef> =
            crate::images::to_chain(&art::card_cover(&art_ctx, &game)).into_iter().filter(|r| !old_cover.contains(r)).collect();
        let default_hero: Vec<ImageRef> =
            crate::images::to_chain(&art::stage_art(&art_ctx, &game)).into_iter().filter(|r| !old_hero.contains(r)).collect();

        let initial = form_from(&game.title, game.metadata.as_deref(), game.meta.as_deref());
        let options = collect_tag_options(&ctx.lib.games);
        let tags: Vec<Entity<TagField>> = TAG_FIELDS
            .iter()
            .enumerate()
            .map(|(i, (id, label, multiple))| {
                let values = match *id {
                    "series" => vec![initial.series.clone()],
                    "developers" => initial.developers.clone(),
                    "publishers" => initial.publishers.clone(),
                    "genres" => initial.genres.clone(),
                    "themes" => initial.themes.clone(),
                    "gameModes" => initial.game_modes.clone(),
                    "platforms" => initial.platforms.clone(),
                    _ => initial.features.clone(),
                };
                let opts = options.get(i).map(|(_, v)| v.clone()).unwrap_or_default();
                let field = cx.new(|cx| TagField::new(&format!("gs-{id}"), label, *multiple, opts, values, window, cx));
                cx.subscribe(&field, |_, _, _: &TagEvent, cx| cx.notify()).detach();
                field
            })
            .collect();
        let details = Details {
            name: new_input(window, cx, &initial.title, ""),
            description: new_textarea(window, cx, &initial.description, 6),
            release: new_input(window, cx, &initial.release_date, ""),
            score: new_input(window, cx, &initial.score, ""),
            websites: new_input(window, cx, &initial.websites, ""),
            notes: new_textarea(window, cx, &initial.notes, 4),
            tags,
        };
        for input in [&details.name, &details.release, &details.score, &details.websites] {
            cx.subscribe_in(input, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        for area in [&details.description, &details.notes] {
            cx.subscribe_in(area, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }

        let show_launch = game.runner == "sideload" && !matches!(game.launch_kind.as_deref(), Some("emulator") | Some("steam-uri"));
        let launch = show_launch.then(|| {
            let exe = game.game.install.executable.clone().unwrap_or_default();
            let run_as = game.meta.as_deref().and_then(|m| m.run_as.clone()).unwrap_or_else(|| "auto".into());
            let input = new_input(window, cx, &exe, "");
            cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.refresh_launch(&text);
                    if let Some(l) = this.launch.as_mut() {
                        l.saved = false;
                    }
                    cx.notify();
                }
            })
            .detach();
            let mut state = LaunchState {
                executable: input,
                run_as: run_as.clone(),
                run_as_open: std::env::var_os("HERONITE_DEBUG_SELECT_OPEN").is_some(),
                initial_exe: exe.clone(),
                initial_run_as: run_as,
                exists: false,
                detected: ExecutableKind::Unknown,
                effective: Platform::Windows,
                saved: false,
            };
            let (exists, detected) = detect(&exe);
            state.exists = exists;
            state.detected = detected;
            state
        });
        let facts = GameFacts::of(&game);
        let sections = facts.sections();
        let heroic = (!sections.is_empty()).then(|| {
            let start = tab.section().filter(|s| sections.contains(s)).unwrap_or(sections[0]);
            let panel = cx.new(|cx| HeroicSettingsPanel::new(ctx.heroic.clone(), Target::Game(facts), start, window, cx));
            cx.observe(&panel, |_, _, cx| cx.notify()).detach();
            (panel, sections)
        });
        let tab = match tab {
            Tab::Launch if launch.is_none() => Tab::Images,
            t if t.section().is_some_and(|s| !heroic.as_ref().is_some_and(|(_, secs)| secs.contains(&s))) => Tab::Images,
            t => t,
        };

        let mut this = Self {
            service,
            images,
            title: game.title.clone(),
            tab,
            art: game.art.as_deref().cloned().unwrap_or_default(),
            default_cover,
            default_hero,
            busy: None,
            error: String::new(),
            nested: None,
            details,
            initial,
            launch,
            heroic,
            game,
            ctx,
        };
        if let Some(text) = this.launch.as_ref().map(|l| l.initial_exe.clone()) {
            this.refresh_launch(&text);
        }
        this
    }

    fn runner(&self) -> String {
        if self.game.runner == "zoom" { "sideload".into() } else { self.game.runner.clone() }
    }

    // -- launch ------------------------------------------------------------

    fn refresh_launch(&mut self, exe: &str) {
        let (exists, detected) = detect(exe);
        let mut meta = self.game.meta.as_deref().cloned();
        let run_as = self.launch.as_ref().map(|l| l.run_as.clone()).unwrap_or_else(|| "auto".into());
        if let Some(m) = meta.as_mut() {
            m.run_as = Some(run_as.clone());
        } else {
            let m = LocalGameMeta { run_as: Some(run_as.clone()), ..Default::default() };
            meta = Some(m);
        }
        let effective = effective_platform(&self.game.game, meta.as_ref(), exe, &SystemProbe);
        if let Some(l) = self.launch.as_mut() {
            l.exists = exists;
            l.detected = detected;
            l.effective = effective;
        }
    }

    fn browse_executable(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update_in(cx, |this, window, cx| {
                if let Some(l) = this.launch.as_ref() {
                    let text = path.to_string_lossy().to_string();
                    l.executable.update(cx, |s, cx| s.set_value(text, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    fn save_launch(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.launch.as_ref() else { return };
        let exe = l.executable.read(cx).value().trim().to_string();
        let run_as = l.run_as.clone();
        self.busy = Some(Busy::Launch);
        self.error.clear();
        let (heroic, game) = (self.ctx.heroic.clone(), self.game.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { write_launch_options(&heroic, &game, &exe, &run_as).map(|_| (exe, run_as)) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok((exe, run_as)) => {
                        if let Some(l) = this.launch.as_mut() {
                            l.initial_exe = exe;
                            l.initial_run_as = run_as;
                            l.saved = true;
                        }
                        cx.emit(DialogEvent::Changed);
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

    // -- art ---------------------------------------------------------------

    fn art_result(&mut self, res: anyhow::Result<CollectionGameArt>, cx: &mut Context<Self>) {
        self.busy = None;
        match res {
            Ok(art) => {
                self.art = art;
                cx.emit(DialogEvent::Changed);
            }
            Err(err) => self.error = format!("{err:#}"),
        }
        cx.notify();
    }

    fn pick_image(&mut self, kind: ArtKind, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Select".into()) });
        let (service, runner, app) = (self.service.clone(), self.runner(), self.game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update(cx, |this, cx| {
                this.busy = Some(Busy::Art(kind));
                this.error.clear();
                cx.notify();
            })
            .ok();
            let res = cx.background_spawn(async move { service.set_art_from_file(&runner, &app, kind, &path.to_string_lossy()) }).await;
            this.update(cx, |this, cx| this.art_result(res, cx)).ok();
        })
        .detach();
    }

    fn reset_image(&mut self, kind: ArtKind, cx: &mut Context<Self>) {
        self.busy = Some(Busy::Art(kind));
        self.error.clear();
        let (service, runner, app) = (self.service.clone(), self.runner(), self.game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.reset_art(&runner, &app, kind) }).await;
            this.update(cx, |this, cx| this.art_result(res, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn open_web(&mut self, kind: ArtKind, window: &mut Window, cx: &mut Context<Self>) {
        let (ctx, game, title) = (self.ctx.clone(), self.game.clone(), self.title.clone());
        let view = cx.new(|cx| WebImageDialog::new(ctx, game, title, kind, window, cx));
        let sub = cx.subscribe(&view, Self::on_art_event);
        self.nested = Some(Nested::Web(view, sub));
        cx.notify();
    }

    fn open_screenshot(&mut self, kind: ArtKind, cx: &mut Context<Self>) {
        let (ctx, game) = (self.ctx.clone(), self.game.clone());
        let view = cx.new(|cx| ScreenshotArtDialog::new(ctx, game, kind, cx));
        let sub = cx.subscribe(&view, Self::on_art_event);
        self.nested = Some(Nested::Screenshot(view, sub));
        cx.notify();
    }

    fn on_art_event<T: 'static>(&mut self, _: Entity<T>, ev: &ArtEvent, cx: &mut Context<Self>) {
        match ev {
            ArtEvent::Changed(art) => {
                self.art = art.clone();
                cx.emit(DialogEvent::Changed);
            }
            ArtEvent::Close => self.nested = None,
        }
        cx.notify();
    }

    /// Public so debug hooks can open the web image picker directly.
    pub fn debug_open_web(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_web(ArtKind::Cover, window, cx);
    }

    // -- details -----------------------------------------------------------

    fn read_form(&self, cx: &App) -> Form {
        let d = &self.details;
        let tag = |i: usize| d.tags[i].read(cx).values().to_vec();
        Form {
            title: d.name.read(cx).value().to_string(),
            description: d.description.read(cx).value().to_string(),
            release_date: d.release.read(cx).value().to_string(),
            series: d.tags[0].read(cx).value(),
            developers: tag(1),
            publishers: tag(2),
            genres: tag(3),
            themes: tag(4),
            game_modes: tag(5),
            platforms: tag(6),
            features: tag(7),
            score: d.score.read(cx).value().to_string(),
            websites: d.websites.read(cx).value().to_string(),
            notes: d.notes.read(cx).value().to_string(),
        }
    }

    fn save_details(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let (form, old) = (self.read_form(cx), self.initial.clone());
        let mut patch = DetailsPatch { app_name: self.game.app_name.clone(), runner: self.runner(), ..Default::default() };
        if form.title != old.title {
            patch.title = Some(form.title.clone());
        }
        if form.notes != old.notes {
            patch.notes = Some(form.notes.clone());
        }
        if form.description != old.description {
            patch.description = Some(form.description.clone());
        }
        if form.release_date != old.release_date {
            patch.release_date = Some(form.release_date.clone());
        }
        if form.series != old.series {
            patch.series = Some(form.series.clone());
        }
        macro_rules! list {
            ($f:ident) => {
                if form.$f != old.$f {
                    patch.$f = Some(form.$f.clone());
                }
            };
        }
        list!(developers);
        list!(publishers);
        list!(genres);
        list!(themes);
        list!(game_modes);
        list!(platforms);
        list!(features);
        if form.websites != old.websites {
            patch.websites = Some(split_list(&form.websites));
        }
        if form.score != old.score {
            let t = form.score.trim();
            if t.is_empty() {
                patch.critic_score = Some(None);
            } else {
                match t.parse::<f64>() {
                    Ok(n) if n.is_finite() => patch.critic_score = Some(Some(n.round())),
                    _ => {
                        self.error = "Score must be a number, or empty.".into();
                        cx.notify();
                        return;
                    }
                }
            }
        }
        self.busy = Some(Busy::Details);
        self.error.clear();
        let service = self.service.clone();
        let title = self.title.clone();
        cx.spawn(async move |this, cx| {
            let res: anyhow::Result<DetailsResult> = cx.background_spawn(async move { service.update_game_details(&patch) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok(r) => {
                        let meta = r.meta.clone().or_else(|| this.game.meta.as_deref().cloned());
                        this.initial = form_from(&title, Some(&r.metadata), meta.as_ref());
                        cx.emit(DialogEvent::Changed);
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

    // -- render parts ------------------------------------------------------

    fn art_slot(&mut self, kind: ArtKind, cx: &mut Context<Self>) -> Div {
        let is_cover = kind == ArtKind::Cover;
        let custom_url = if is_cover { self.art.cover_url.clone() } else { self.art.hero_url.clone() };
        let shots = self.ctx.lib.settings.screenshots_folder().map(|p| p.to_path_buf());
        let mut chain = custom_url.as_deref().map(|u| chain_for_url(&self.ctx.heroic, shots.as_deref(), u)).unwrap_or_default();
        chain.extend(if is_cover { self.default_cover.clone() } else { self.default_hero.clone() });
        let (w, h) = if is_cover { (132., 198.) } else { (840., 271.) };
        let key = chain.first().map(|r| format!("{r:?}")).unwrap_or_default();
        let src = if chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get(&format!("gs:{key}"), chain, w, h)) };
        let preview = picture(src)
            .w(px(w))
            .h(px(h))
            .flex_none()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::white_a(0.08))
            .into_any_element();

        let busy = self.busy.is_some();
        let tag = if is_cover { "cover" } else { "hero" };
        let busy_this = self.busy == Some(Busy::Art(kind));
        let mut actions = div().flex().flex_wrap().gap(px(8.)).child(action_button(
            SharedString::from(format!("gs-{tag}-change")),
            ButtonKind::Outline,
            if busy_this { "Saving\u{2026}" } else { "Change" },
            !busy,
            cx.listener(move |this, _, _, cx| this.pick_image(kind, cx)),
        ));
        actions = actions.child(action_button(
            SharedString::from(format!("gs-{tag}-web")),
            ButtonKind::Outline,
            "Search web",
            !busy,
            cx.listener(move |this, _, window, cx| this.open_web(kind, window, cx)),
        ));
        if !is_cover {
            actions = actions.child(action_button(
                "gs-hero-shot",
                ButtonKind::Outline,
                "Screenshot",
                !busy,
                cx.listener(move |this, _, _, cx| this.open_screenshot(kind, cx)),
            ));
        }
        actions = actions.child(action_button(
            SharedString::from(format!("gs-{tag}-reset")),
            ButtonKind::Outline,
            "Reset",
            !busy && custom_url.is_some(),
            cx.listener(move |this, _, _, cx| this.reset_image(kind, cx)),
        ));
        let copy = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(div().font_family(theme::FONT_HEADING).text_size(px(22.)).text_color(rgb(0xcaf3fd)).child(if is_cover { "Cover" } else { "Background" }))
            .child(div().mb(px(8.)).text_size(px(13.33)).text_color(white()).child(if custom_url.is_some() { "Custom image" } else { "Default" }))
            .child(actions);
        if is_cover {
            div().flex().items_center().justify_between().gap(px(16.)).child(copy).child(preview)
        } else {
            div().flex().flex_col().gap(px(16.)).pt(px(16.)).border_t_1().border_color(theme::white_a(0.08)).child(copy).child(preview)
        }
    }

    fn images_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let cover = self.art_slot(ArtKind::Cover, cx);
        let hero = self.art_slot(ArtKind::Hero, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(div().text_size(px(16.)).text_color(white()).child(
                "These images are used in Collection. Steam art stays cached; a custom file replaces it until you reset.",
            ))
            .child(div().text_size(px(16.)).font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child(self.title.clone()))
            .child(cover)
            .child(hero)
    }

    fn details_tab(&mut self) -> Div {
        let d = &self.details;
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(16.)).text_color(white()).child(
                "These fields stay in Collection. Fetching metadata later will not overwrite values you edit here, unless you choose to overwrite.",
            ))
            .child(div().text_size(px(13.33)).text_color(white()).child("Pick existing tags or type a new one and press Enter to create it."))
            .child(labeled("Name", input_box(&d.name, false)))
            .child(labeled("Description", textarea_box(&d.description, 6)))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().flex_1().child(labeled("Release date", input_box(&d.release, false))))
                    .child(div().flex_1().child(labeled("Score", input_box(&d.score, false)))),
            );
        for tag in &d.tags {
            col = col.child(tag.clone());
        }
        col.child(labeled("Websites", input_box(&d.websites, false))).child(labeled("Notes", textarea_box(&d.notes, 4)))
    }

    fn launch_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(l) = self.launch.as_ref() else { return div() };
        let dirty = l.executable.read(cx).value().as_ref() != l.initial_exe || l.run_as != l.initial_run_as;
        let exe_empty = l.executable.read(cx).value().trim().is_empty();
        let busy = self.busy.is_some();
        let detected = match l.detected {
            ExecutableKind::Linux => "Linux program (ELF, script or AppImage)",
            ExecutableKind::Windows => "Windows program",
            ExecutableKind::Unknown => "Unknown file type",
        };
        let effective = if l.effective == Platform::Linux { "Runs natively on Linux" } else { "Runs through Wine/Proton" };
        let run_as = [("auto", "Automatic (from the file)"), ("linux", "Linux native"), ("windows", "Windows (Wine/Proton)")];
        let current_label = run_as.iter().find(|(v, _)| *v == l.run_as).map(|(_, t)| *t).unwrap_or_default();
        let options: Vec<(SharedString, bool)> = run_as.iter().map(|(v, t)| ((*t).into(), *v == l.run_as)).collect();
        let select = select_field(
            "gs-runas",
            Some("Run as"),
            current_label,
            false,
            l.run_as_open,
            options,
            cx,
            |this, _, cx| {
                if let Some(l) = this.launch.as_mut() {
                    l.run_as_open = !l.run_as_open;
                }
                cx.notify();
            },
            move |this, i, _, cx| {
                let exe = this.launch.as_ref().map(|l| l.executable.read(cx).value().to_string()).unwrap_or_default();
                if let Some(l) = this.launch.as_mut() {
                    l.run_as = run_as[i].0.to_string();
                    l.run_as_open = false;
                    l.saved = false;
                }
                this.refresh_launch(&exe);
                cx.notify();
            },
        );
        let saved = l.saved && !dirty;
        let mut status = div().text_size(px(16.)).line_height(px(24.)).text_color(white()).child(
            div()
                .flex()
                .flex_wrap()
                .child(div().font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child("Detected:"))
                .child(div().child(format!(" {detected}")))
                .when(!l.exists, |d| d.child(div().text_color(theme::danger()).child(" (file not found)"))),
        );
        status = status.child(
            div()
                .flex()
                .child(div().font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child("Current:"))
                .child(div().child(format!(" {effective}"))),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(div().text_size(px(16.)).line_height(px(24.)).text_color(white()).child(
                "Linux programs (ELF binaries, shell scripts, AppImages) run directly; Windows programs run through Wine/Proton. Automatic decides from the file.",
            ))
            .child(labeled(
                "Executable",
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_1().child(input_box(&l.executable, false)))
                    .child(action_button("gs-browse", ButtonKind::Outline, "Browse", !busy, cx.listener(|this, _, _, cx| this.browse_executable(cx)))),
            ))
            .child(status)
            .child(select)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(action_button(
                        "gs-launch-save",
                        ButtonKind::Primary,
                        if self.busy == Some(Busy::Launch) { "Saving\u{2026}" } else { "Save" },
                        !busy && dirty && !exe_empty,
                        cx.listener(|this, _, _, cx| this.save_launch(cx)),
                    ))
                    .when(saved, |d| d.child(div().text_size(px(16.)).text_color(theme::success()).child("Saved"))),
            )
    }
}

/// Port of `setLocalLaunchOptions`: overlay meta (`runAs`, executable) and the
/// sideload entry (platform, executable, folder).
fn write_launch_options(paths: &HeroicPaths, game: &heronite_data::CollectionGame, executable: &str, run_as: &str) -> anyhow::Result<()> {
    let exe = {
        let t = executable.trim();
        if t.is_empty() { game.game.install.executable.clone().unwrap_or_default() } else { t.to_string() }
    };
    let mut new_meta = game.meta.as_deref().cloned();
    if let Some(m) = new_meta.as_mut() {
        m.run_as = Some(run_as.to_string());
        if !exe.is_empty() {
            m.remapped_executable = Some(exe.clone());
            m.launch_kind = "executable".into();
        }
        let (ra, re, lk) = (m.run_as.clone(), m.remapped_executable.clone(), m.launch_kind.clone());
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            if let Some(e) = f.games.get_mut(&game.app_name) {
                e.run_as = ra;
                e.remapped_executable = re;
                e.launch_kind = lk;
            }
            Ok(())
        })?;
    }
    let mut info = (*game.game).clone();
    info.install.executable = Some(exe.clone());
    let mut meta_for_platform = new_meta.unwrap_or_default();
    meta_for_platform.run_as = Some(run_as.to_string());
    let platform = effective_platform(&info, Some(&meta_for_platform), &exe, &SystemProbe);
    let linux = platform == Platform::Linux;
    json_store::update_value(&paths.sideload_library(), |doc| {
        let Some(games) = doc.get_mut("games").and_then(|g| g.as_array_mut()) else { return Ok(()) };
        for g in games {
            if g.get("app_name").and_then(|v| v.as_str()) != Some(game.app_name.as_str()) {
                continue;
            }
            g["is_linux_native"] = serde_json::Value::Bool(linux);
            if !exe.is_empty() {
                if let Some(dir) = Path::new(&exe).parent() {
                    g["folder_name"] = serde_json::Value::String(dir.to_string_lossy().into_owned());
                }
                if !g["install"].is_object() {
                    g["install"] = serde_json::json!({});
                }
                g["install"]["executable"] = serde_json::Value::String(exe.clone());
            }
            if !g["install"].is_object() {
                g["install"] = serde_json::json!({});
            }
            g["install"]["platform"] = serde_json::Value::String(if linux { "linux" } else { "Windows" }.into());
        }
        Ok(())
    })?;
    Ok(())
}

impl Render for GameSettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let mut items = vec![(Tab::Images, Tab::Images.label()), (Tab::Details, Tab::Details.label())];
        if self.launch.is_some() {
            items.push((Tab::Launch, Tab::Launch.label()));
        }
        if let Some((_, sections)) = &self.heroic {
            items.extend(sections.iter().map(|s| (Tab::of_section(*s), s.label())));
        }
        let entity = cx.entity().downgrade();
        let panel = self.heroic.as_ref().map(|(p, _)| p.clone());
        let tab_bar = tabs("gs", &items, self.tab, move |t, window, cx| {
            if let (Some(section), Some(panel)) = (t.section(), panel.as_ref()) {
                panel.update(cx, |p, cx| p.set_section(section, window, cx));
            }
            entity
                .update(cx, |this, cx| {
                    this.tab = t;
                    cx.notify();
                })
                .ok();
        });
        let content = match self.tab {
            Tab::Images => self.images_tab(cx),
            Tab::Details => self.details_tab(),
            Tab::Launch => self.launch_tab(cx),
            _ => match &self.heroic {
                Some((panel, _)) => div().child(panel.clone()),
                None => div(),
            },
        };
        let mut body = div().flex().flex_col().gap(px(16.)).child(tab_bar).child(content);
        if !self.error.is_empty() {
            body = body.child(div().text_size(px(16.)).text_color(theme::danger()).child(self.error.clone()));
        }

        let dirty = self.read_form(cx) != self.initial;
        let mut footer = div().flex().gap(px(16.));
        if self.tab == Tab::Details {
            footer = footer.child(action_button(
                "gs-save",
                ButtonKind::Primary,
                if self.busy == Some(Busy::Details) { "Saving\u{2026}" } else { "Save" },
                self.busy.is_none() && dirty,
                cx.listener(|this, _, _, cx| this.save_details(cx)),
            ));
        }
        footer = footer.child(
            button("gs-close", if self.tab == Tab::Details { ButtonKind::Outline } else { ButtonKind::Primary }, "Close")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
        );
        let entity = cx.entity().downgrade();
        let (vw, vh) = (f32::from(window.viewport_size().width), f32::from(window.viewport_size().height));
        let main = dialog(
            DialogProps {
                id: "collection-game-settings".into(),
                title: format!("{} (Settings)", self.title).into(),
                width: 840f32.min(vw * 0.9) + 64.,
                height: Some(debug_dialog_height().unwrap_or(620f32.min(vh * 0.7))),
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        );
        let nested: Option<AnyView> = match &self.nested {
            Some(Nested::Web(v, _)) => Some(v.clone().into()),
            Some(Nested::Screenshot(v, _)) => Some(v.clone().into()),
            None => None,
        };
        div().absolute().inset_0().child(main).children(nested)
    }
}
