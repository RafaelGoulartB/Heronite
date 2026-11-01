//! `GameSettingsDialog` ("Game settings"): Images, Details and Launch tabs for
//! one game, plus Heroic's own settings tabs.

mod actions;
mod console;
mod console_actions;
mod details;
mod images;
mod launch;
mod sessions;

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata};
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_metadata::{ArtKind, MetadataService};

use super::art_images::{picture, DialogImages};
use super::metadata::MetadataDialog;
use super::metadata_wizard::hub;
use super::save_backups::{self, SaveBackupsView};
use super::screenshot_art::ScreenshotArtDialog;
use super::settings_panel::{GameFacts, Section, SettingsPanel, Target};
use super::web_image::WebImageDialog;
use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, close_button, dialog_title, kicker, tabs, ButtonKind};
use crate::ui::dialog::{drawer, DrawerProps};
use actions::ActionsState;
use details::{form_from, Details, Form};
use launch::LaunchState;
use sessions::SessionsState;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    /// Images and details.
    Game,
    /// Play (and, for local games, what it starts) and the user's actions.
    Actions,
    /// The panel's launch settings.
    Launch,
    Compatibility,
    Performance,
    Gpu,
    /// The Ludusavi backups.
    Saves,
    /// The store's cloud saves.
    CloudSaves,
    /// What the game can reach.
    Sandbox,
}

impl Tab {
    /// The panel section this tab shows.
    fn section(self) -> Option<Section> {
        match self {
            Tab::Launch => Some(Section::Launch),
            Tab::Compatibility => Some(Section::Compatibility),
            Tab::Performance => Some(Section::Performance),
            Tab::Gpu => Some(Section::Gpu),
            Tab::CloudSaves => Some(Section::Saves),
            Tab::Sandbox => Some(Section::Sandbox),
            _ => None,
        }
    }

    fn of_section(s: Section) -> Tab {
        match s {
            Section::Launch => Tab::Launch,
            Section::Compatibility => Tab::Compatibility,
            Section::Performance => Tab::Performance,
            Section::Gpu => Tab::Gpu,
            Section::Saves => Tab::CloudSaves,
            Section::Sandbox => Tab::Sandbox,
            Section::General => Tab::Game,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Game => tr!("Details"),
            Tab::Actions => tr!("Actions"),
            Tab::Launch => tr!("Launch"),
            Tab::Compatibility => tr!("Compatibility"),
            Tab::Performance => tr!("Performance"),
            Tab::Gpu => tr!("GPU"),
            Tab::Saves => tr!("Saves"),
            Tab::CloudSaves => tr!("Cloud saves"),
            Tab::Sandbox => tr!("Sandbox"),
        }
    }

    /// The tab the settings open on: Compatibility for games run through
    /// Wine or Proton, Performance for Linux games, Game otherwise.
    pub fn settings_start(game: &Game) -> Tab {
        GameFacts::of(game).start_section().map(Tab::of_section).unwrap_or(Tab::Game)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Art(ArtKind),
    Details,
    Launch,
}

enum Nested {
    Web(Entity<WebImageDialog>, #[allow(dead_code)] Subscription),
    Screenshot(Entity<ScreenshotArtDialog>, #[allow(dead_code)] Subscription),
    Metadata(Entity<MetadataDialog>, #[allow(dead_code)] Subscription),
}

pub struct GameSettingsDialog {
    ctx: DialogCtx,
    service: Arc<MetadataService>,
    images: Entity<DialogImages>,
    game: Game,
    /// The game's fetched metadata: `game`'s, until "Fetch metadata" applies new.
    metadata: Option<Arc<CollectionGameMetadata>>,
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
    /// Saved time played, in minutes.
    initial_playtime: u64,
    /// The Details tab as the drawer opened ("Undo" goes back to it).
    opened: (Form, u64),
    /// The wait before the Details tab saves by itself.
    save_timer: Option<Task<()>>,
    /// The Details tab saved something since the drawer opened.
    saved: bool,
    /// Close once the save being written is done.
    close_after_save: bool,
    /// The play sessions (Game tab).
    sessions: SessionsState,
    launch: Option<LaunchState>,
    /// The game's own actions (Launch tab).
    actions: ActionsState,
    /// Heroic settings: the view and the tabs this game has.
    panel: Option<(Entity<SettingsPanel>, Vec<Section>)>,
    /// The Saves tab, for games that run here or through Steam.
    backups: Option<Entity<SaveBackupsView>>,
    /// Console mode: focused row and the open keyboard / picker.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for GameSettingsDialog {}

impl GameSettingsDialog {
    /// Drops the question about "Reset this tab"; true when there was one.
    fn cancel_reset_question(&mut self, cx: &mut Context<Self>) -> bool {
        self.panel.as_ref().is_some_and(|(p, _)| p.update(cx, |p, cx| p.cancel_reset(cx)))
    }

    /// Esc, offered by the host before it closes the drawer: answers the
    /// question about resetting the tab, else closes once the Details tab
    /// saved what it has pending.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.cancel_reset_question(cx) {
            return true;
        }
        self.close(cx);
        true
    }

    pub fn new(ctx: DialogCtx, game: Game, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();

        let (default_cover, default_hero) = Self::default_art(&ctx, &game);
        let initial = form_from(&game.title, game.metadata.as_deref(), game.meta.as_deref());
        let details = Self::build_details(&ctx, &initial, game.playtime_minutes, window, cx);
        let launch = Self::build_launch(&game, window, cx);
        let actions = Self::build_actions(&game, &ctx.data_paths);
        let facts = GameFacts::of(&game);
        let mut sections = facts.sections();
        // The Sandbox tab only for a game put in the sandbox (when it was
        // added or installed), where there is one (not in the Flatpak).
        // Decided once: turning it off here keeps the tab.
        if !heronite_platform::supported(heronite_platform::Feature::Sandbox) || !sandboxed(&ctx, &game.app_name) {
            sections.retain(|s| *s != Section::Sandbox);
        }
        let panel = (!sections.is_empty()).then(|| {
            let start = tab.section().filter(|s| sections.contains(s)).unwrap_or(sections[0]);
            let panel = cx.new(|cx| SettingsPanel::new(ctx.data_paths.clone(), Target::Game(facts), start, window, cx));
            cx.observe(&panel, |_, _, cx| cx.notify()).detach();
            (panel, sections)
        });
        let backups = save_backups::has_tab(&game).then(|| {
            let view = cx.new(|_| SaveBackupsView::new(ctx.data_paths.clone(), &game));
            cx.observe(&view, |_, _, cx| cx.notify()).detach();
            view
        });
        let has = |s: Section| panel.as_ref().is_some_and(|(_, secs)| secs.contains(&s));
        let tab = match tab {
            Tab::Saves if backups.is_none() => Tab::Game,
            t if t.section().is_some_and(|s| !has(s)) => Tab::Game,
            t => t,
        };

        let mut this = Self {
            service,
            images,
            title: game.title.clone(),
            metadata: game.metadata.clone(),
            tab,
            art: game.art.as_deref().cloned().unwrap_or_default(),
            default_cover,
            default_hero,
            busy: None,
            error: String::new(),
            nested: None,
            details,
            opened: (initial.clone(), game.playtime_minutes),
            save_timer: None,
            saved: false,
            close_after_save: false,
            initial,
            initial_playtime: game.playtime_minutes,
            sessions: SessionsState::new(game.sessions.as_deref().map_or(&[], Vec::as_slice)),
            launch,
            actions,
            panel,
            backups,
            game,
            ctx,
            console_list: Default::default(),
            layer: Default::default(),
        };
        if let Some(text) = this.launch.as_ref().map(|l| l.initial_exe.clone()) {
            this.refresh_launch(&text);
        }
        if this.tab == Tab::Saves {
            this.show_backups(cx);
        }
        this.debug_set_art(cx);
        // `HERONITE_DEBUG_ACTION_EDIT=new|<id>`: the Actions tab's action editor starts open.
        if let Some(which) = crate::env::value("HERONITE_DEBUG_ACTION_EDIT") {
            this.edit_action((which != "new").then_some(which), window, cx);
        }
        this.debug_sessions(window, cx);
        // `HERONITE_DEBUG_DETAILS_EDIT=<name>[@undo|@close]`: that name typed in the Details tab,
        // which then saves by itself; `undo` presses Undo once saved, `close` closes at once
        // (use a copy of the data folder).
        if let Some(spec) = crate::env::value("HERONITE_DEBUG_DETAILS_EDIT") {
            let (name, then) = spec.split_once('@').unwrap_or((spec.as_str(), ""));
            this.details.name.update(cx, |s, cx| s.set_value(name.to_string(), window, cx));
            this.schedule_save(cx);
            match then {
                "close" => this.close(cx),
                "undo" => {
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(std::time::Duration::from_millis(2500)).await;
                        this.update_in(cx, |this, window, cx| this.undo_details(window, cx)).ok();
                    })
                    .detach();
                }
                _ => {}
            }
        }
        this
    }

    /// The tabs: Game and Actions always, then what the game has.
    fn tab_list(&self) -> Vec<Tab> {
        let has = |s: Section| self.panel.as_ref().is_some_and(|(_, secs)| secs.contains(&s));
        let mut t = vec![Tab::Game, Tab::Actions];
        t.extend(
            [Tab::Launch, Tab::Compatibility, Tab::Performance, Tab::Gpu, Tab::Sandbox]
                .into_iter()
                .filter(|t| t.section().is_some_and(has)),
        );
        if self.backups.is_some() {
            t.push(Tab::Saves);
        }
        if has(Section::Saves) {
            t.push(Tab::CloudSaves);
        }
        t
    }

    /// Reads the backups the first time the Saves tab shows.
    fn show_backups(&self, cx: &mut Context<Self>) {
        if let Some(view) = &self.backups {
            view.update(cx, |v, cx| v.shown(cx));
        }
    }

    fn runner(&self) -> String {
        if self.game.runner == "zoom" {
            "sideload".into()
        } else {
            self.game.runner.clone()
        }
    }

    /// Cover, kicker, title and the "store, installed, size" line.
    fn drawer_header(&mut self, on_close: impl Fn(&mut Window, &mut App) + 'static, cx: &mut Context<Self>) -> Div {
        let (chain, _) = self.art_chain(ArtKind::Cover);
        let key = chain.first().map(|r| format!("{r:?}")).unwrap_or_default();
        let src =
            if chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get(&format!("gs-head:{key}"), chain, 104., 140.)) };
        let cover = picture(src).w(px(52.)).h(px(70.)).flex_none().rounded(px(10.)).border_1().border_color(theme::ink_a(0.10));
        let mut meta = vec![self.game.source_label.to_string()];
        meta.push(if self.game.installed { tr!("Installed").into() } else { tr!("Not installed").into() });
        if let Some(size) = self.game.game.install.install_size.as_deref().filter(|s| !s.is_empty() && self.game.installed) {
            meta.push(size.to_string());
        }
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(20.))
            .pt(px(20.))
            .child(cover)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(kicker(tr!("Game settings")))
                    .child(dialog_title(self.title.clone()).truncate())
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).truncate().child(meta.join(" \u{b7} "))),
            )
            .child(div().self_start().child(close_button("gs-close-x").on_click(move |_, w, cx| on_close(w, cx))))
    }
}

impl GameSettingsDialog {
    /// The Details tab's saving, next to Done: "Saving…", then "Saved" with
    /// "Undo" while it differs from what the drawer opened with.
    fn save_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let note = |text: &'static str| div().text_size(px(12.)).text_color(theme::text_muted()).child(text);
        if self.busy == Some(Busy::Details) {
            return Some(note(tr!("Saving…")).into_any_element());
        }
        if !self.saved {
            return None;
        }
        let undo = self.changed_since_open(cx).then(|| {
            div()
                .id("gs-undo")
                .text_size(px(12.))
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|s| s.text_color(theme::accent_overlay()))
                .child(tr!("Undo"))
                .on_click(cx.listener(|this, _, window, cx| this.undo_details(window, cx)))
        });
        Some(div().flex().items_center().gap(px(8.)).mr(px(6.)).child(note(tr!("Saved"))).children(undo).into_any_element())
    }
}

/// A card row holding a form control (padding 12).
fn block(el: impl IntoElement) -> AnyElement {
    div().py(px(12.)).child(el).into_any_element()
}

impl Render for GameSettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.images.update(cx, |im, _| im.frame(window));
        let items: Vec<(Tab, &'static str)> = self.tab_list().into_iter().map(|t| (t, t.label())).collect();
        let entity = cx.entity().downgrade();
        let panel = self.panel.clone();
        let tab_bar = tabs("gs", &items, self.tab, move |t, window, cx| {
            if let Some((panel, _)) = panel.as_ref() {
                panel.update(cx, |p, cx| {
                    p.cancel_reset(cx);
                });
            }
            if let (Some(section), Some((panel, secs))) = (t.section(), panel.as_ref()) {
                if secs.contains(&section) {
                    panel.update(cx, |p, cx| p.set_section(section, window, cx));
                }
            }
            entity
                .update(cx, |this, cx| {
                    this.tab = t;
                    if t == Tab::Saves {
                        this.show_backups(cx);
                    }
                    cx.notify();
                })
                .ok();
        });
        let panel_view = self.panel.as_ref().map(|(p, _)| p.clone());
        let content = match self.tab {
            Tab::Game => div().flex().flex_col().gap(px(6.)).child(self.images_tab(cx)).child(self.details_tab(cx)),
            Tab::Actions => self.actions_tab(cx),
            Tab::Saves => div().children(self.backups.clone()),
            _ => div().children(panel_view),
        };
        let mut body = div().flex().flex_col().gap(px(10.)).child(content);
        if !self.error.is_empty() {
            body = body.child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(theme::R_INPUT))
                    .bg(theme::danger_fill())
                    .text_size(px(13.))
                    .text_color(theme::danger_text())
                    .child(self.error.clone()),
            );
        }

        // "Use defaults" only where a reset exists: the custom images.
        let can_reset = self.tab == Tab::Game && self.busy.is_none() && (self.art.cover_url.is_some() || self.art.hero_url.is_some());
        let left = div().child(if self.tab == Tab::Game {
            action_button(
                "gs-defaults",
                ButtonKind::Ghost,
                tr!("Use defaults"),
                can_reset,
                cx.listener(|this, _, _, cx| this.reset_all_images(cx)),
            )
            .into_any_element()
        } else {
            div().into_any_element()
        });
        let mut right = div().flex().items_center().gap(px(8.));
        if self.tab == Tab::Game {
            right = right.children(self.save_status(cx));
        }
        // The panel's tab: its changed settings back to the game defaults.
        let changed_panel =
            self.panel.as_ref().map(|(p, _)| p.clone()).filter(|p| self.tab.section().is_some() && p.read(cx).tab_changed());
        // While "Reset this tab" is asked, its question takes the buttons' place.
        let asking = changed_panel.as_ref().is_some_and(|p| p.read(cx).asking_reset());
        if let (Some(panel), true) = (changed_panel.clone(), asking) {
            right = SettingsPanel::reset_question(&panel, "gs-reset");
        } else {
            if let Some(panel) = changed_panel {
                right = right.child(action_button(
                    "gs-reset-tab",
                    ButtonKind::Ghost,
                    tr!("Reset this tab"),
                    true,
                    cx.listener(move |_, _, _, cx| panel.update(cx, |p, cx| p.ask_reset(cx))),
                ));
            }
            right =
                right.child(button("gs-close", ButtonKind::Primary, tr!("Done")).on_click(cx.listener(|this, _, _, cx| this.close(cx))));
        }
        let footer = div().flex().w_full().items_center().justify_between().child(left).child(right);

        let entity = cx.entity().downgrade();
        let header_close = {
            let entity = entity.clone();
            move |_: &mut Window, cx: &mut App| {
                entity.update(cx, |this, cx| this.close(cx)).ok();
            }
        };
        let header = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pb(px(10.))
            .child(self.drawer_header(header_close, cx))
            .child(div().px(px(20.)).child(tab_bar));
        let vw = f32::from(window.viewport_size().width);
        let main = drawer(
            DrawerProps { id: "collection-game-settings".into(), width: DRAWER_W.min(vw * 0.96) },
            header.into_any_element(),
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |this, cx| this.close(cx)).ok();
            },
        );
        let nested: Option<AnyView> = match &self.nested {
            Some(Nested::Web(v, _)) => Some(v.clone().into()),
            Some(Nested::Screenshot(v, _)) => Some(v.clone().into()),
            Some(Nested::Metadata(v, _)) => Some(v.clone().into()),
            None => None,
        };
        div().absolute().inset_0().child(main).children(nested).into_any_element()
    }
}

/// Drawer width (px).
const DRAWER_W: f32 = 580.;

/// The game's settings have the sandbox on (a tiny read of its config).
fn sandboxed(ctx: &DialogCtx, app_name: &str) -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let settings = heronite_data::game_config::load_game(&ctx.data_paths, app_name, &home);
    heronite_launch::sandbox::Sandbox::from_map(&settings).enabled
}
