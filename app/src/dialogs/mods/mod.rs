//! The Mods manager (`docs/MODS.md`): a drawer for one game with its mods in
//! load order (Installed), a search of Nexus Mods and Thunderstore (Browse),
//! the game's links (Game) and, for Bethesda games, the plugin list.
//!
//! The mods service owns the data; this view reads a snapshot of it in
//! `render` and calls its methods. Searches of the sites run here, off the
//! UI thread.

mod actions;
mod browse;
mod debug;
mod fixture;
mod format;
mod game_tab;
mod render_browse;
mod render_game;
mod render_jobs;
mod render_list;
mod render_menu;
mod render_notes;
mod render_plugins;
mod render_row;

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_i18n::tr;
use heronite_mods::nexus::StoredAccount;

use super::art_images::DialogImages;
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::services::mods::{self, GameMods, HandlerState, Job, ModsService};
use crate::theme;
use crate::ui::controls::{close_button, dialog_title, kicker, tabs, toggle};
use crate::ui::dialog::{drawer, DrawerProps};
use crate::ui::icon;

/// Drawer width (px): wide enough for a mod's row with its pills.
const DRAWER_W: f32 = 720.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Installed,
    Browse,
    Game,
    /// Bethesda games: the plugins of the enabled mods.
    Plugins,
}

impl Tab {
    pub fn parse(s: &str) -> Option<Tab> {
        match s {
            "installed" => Some(Tab::Installed),
            "browse" => Some(Tab::Browse),
            "game" => Some(Tab::Game),
            "plugins" => Some(Tab::Plugins),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Installed => tr!("Installed"),
            Tab::Browse => tr!("Browse"),
            Tab::Game => tr!("Game"),
            Tab::Plugins => tr!("Plugins"),
        }
    }
}

/// What the manager asks the Collection to do besides closing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModsDialogEvent {
    /// Settings → Mods (the FUSE problem, the Nexus account, the links).
    OpenSettings,
    /// The FOMOD installer the service holds is waiting: show its wizard again.
    OpenInstaller,
}

/// What the service holds for the game, copied for one frame.
pub(super) struct View {
    pub g: GameMods,
    pub jobs: Vec<Job>,
    pub account: Option<StoredAccount>,
    pub support_problem: Option<String>,
    pub handlers: HandlerState,
    /// Mods whose files do not look like they belong in the game's folders.
    pub misplaced: HashSet<String>,
}

impl View {
    pub fn premium(&self) -> bool {
        self.account.as_ref().is_some_and(|a| a.account.is_premium)
    }
}

/// A question asked inside the drawer, answered by the next click.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Confirm {
    Remove(String),
    ClearOverwrite,
    ForgetGame,
}

pub(super) struct Rename {
    pub id: String,
    pub input: Entity<InputState>,
}

/// The "⋯" menu of a mod, opened at the click.
pub(super) struct RowMenu {
    pub id: String,
    pub at: Point<Pixels>,
}

pub struct ModsDialog {
    game: Arc<CollectionGame>,
    key: String,
    service: Entity<ModsService>,
    images: Entity<DialogImages>,
    tab: Tab,
    /// The game is running: changes wait for the next launch.
    running: bool,
    menu: Option<RowMenu>,
    rename: Option<Rename>,
    confirm: Option<Confirm>,
    /// Mods whose conflict list is open.
    expanded: HashSet<String>,
    /// "Turn into a mod": the name being typed.
    naming_overwrite: Option<Entity<InputState>>,
    browse: browse::Browse,
    links: game_tab::LinksForm,
    fixture: Option<fixture::Fixture>,
}

impl EventEmitter<DialogEvent> for ModsDialog {}
impl EventEmitter<ModsDialogEvent> for ModsDialog {}

impl ModsDialog {
    pub fn new(ctx: DialogCtx, game: Arc<CollectionGame>, tab: Tab, running: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = mods::service(cx);
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        cx.observe_in(&service, window, |this, _, window, cx| {
            this.sync_links_form(window, cx);
            if this.tab == Tab::Browse {
                this.browse_shown(cx);
            }
            cx.notify();
        })
        .detach();
        let fixture = fixture::load(&ctx, &game);
        if fixture.is_none() {
            service.update(cx, |s, cx| {
                s.load(&game, cx);
                s.refresh_handlers(cx);
            });
        }
        let browse = browse::Browse::new(window, cx);
        let links = game_tab::LinksForm::new(window, cx);
        let mut this = Self {
            key: game.key.clone(),
            game,
            service,
            images,
            tab,
            running,
            menu: None,
            rename: None,
            confirm: None,
            expanded: HashSet::new(),
            naming_overwrite: None,
            browse,
            links,
            fixture,
        };
        this.sync_links_form(window, cx);
        this.debug_state(window, cx);
        if this.tab == Tab::Browse {
            this.browse_shown(cx);
        }
        this
    }

    pub fn game_key(&self) -> &str {
        &self.key
    }

    pub fn app_name(&self) -> &str {
        &self.game.app_name
    }

    pub fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        if self.running != running {
            self.running = running;
            cx.notify();
        }
    }

    /// Esc closes what is open inside the drawer first. True when it did.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let used = self.menu.take().is_some()
            || self.rename.take().is_some()
            || self.confirm.take().is_some()
            || self.naming_overwrite.take().is_some();
        if used {
            cx.notify();
        }
        used
    }

    fn snapshot(&self, cx: &App) -> Option<View> {
        let svc = self.service.read(cx);
        let support_problem = svc.support().and_then(|s| s.problem());
        let account = svc.account().cloned();
        let handlers = svc.handlers();
        match svc.game(&self.key) {
            Some(g) => Some(View {
                g: g.clone(),
                jobs: svc.jobs(Some(&self.key)).cloned().collect(),
                account,
                support_problem,
                handlers,
                misplaced: svc.misplaced(&self.key),
            }),
            None => self.fixture.as_ref().map(|f| View {
                g: f.game.clone(),
                jobs: f.jobs.clone(),
                account: account.or_else(|| f.account.clone()),
                support_problem: support_problem.or_else(|| f.support_problem.clone()),
                handlers,
                misplaced: f.misplaced.clone(),
            }),
        }
    }

    fn tab_list(&self, v: Option<&View>) -> Vec<Tab> {
        let mut t = vec![Tab::Installed, Tab::Browse, Tab::Game];
        if v.is_some_and(|v| v.g.info.profile.plugins.is_some()) {
            t.push(Tab::Plugins);
        }
        t
    }

    pub(super) fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.menu = None;
        self.confirm = None;
        if tab == Tab::Browse {
            self.browse_shown(cx);
        }
        cx.notify();
    }

    /// Kicker, game title, status line, the master switch and close.
    fn render_header(&mut self, v: Option<&View>, cx: &mut Context<Self>) -> Div {
        let (status, active) = match v {
            None => (tr!("Loading…").to_string(), false),
            Some(v) => (format::status_line(&v.g), v.g.list.active),
        };
        let key = self.key.clone();
        let switch = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Play with mods")))
            .child(toggle("mods-active", active, v.is_none()).on_click(cx.listener(move |this, _, _, cx| {
                this.service.update(cx, |s, cx| s.set_active(&key, !active, cx));
            })));
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(20.))
            .pt(px(20.))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(46.))
                    .rounded(px(12.))
                    .bg(theme::accent_tint())
                    .child(icon(Icon::Extension, px(24.), theme::accent())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(kicker(tr!("Mods")))
                    .child(dialog_title(self.game.title.clone()).truncate())
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).truncate().child(status)),
            )
            .child(switch)
            .child(div().self_start().child(close_button("mods-close-x").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))))
    }
}

impl Render for ModsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let view = self.snapshot(cx);
        let tab_items: Vec<(Tab, &'static str)> = self.tab_list(view.as_ref()).into_iter().map(|t| (t, t.label())).collect();
        if !tab_items.iter().any(|(t, _)| *t == self.tab) {
            self.tab = Tab::Installed;
        }
        let entity = cx.entity().downgrade();
        let tab_bar = tabs("mods", &tab_items, self.tab, move |t, _, cx| {
            entity.update(cx, |this, cx| this.show_tab(t, cx)).ok();
        });
        let header = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pb(px(10.))
            .child(self.render_header(view.as_ref(), cx))
            .child(div().px(px(20.)).child(tab_bar));

        let vh = f32::from(window.viewport_size().height);
        let body = match (&view, self.tab) {
            (_, Tab::Browse) => self.render_browse(view.as_ref(), window, cx),
            (None, _) => self.render_loading(),
            (Some(v), Tab::Installed) => self.render_installed(v, window, cx),
            (Some(v), Tab::Game) => self.render_game(v, window, cx),
            (Some(v), Tab::Plugins) => self.render_plugins(v, cx),
        };
        // Files dropped anywhere on the drawer's body are added as mods.
        let body = div()
            .id("mods-drop")
            .min_h(px((vh - 220.).max(200.)))
            .rounded(px(theme::R_CARD))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(theme::accent_wash()).border_1().border_color(theme::accent_wash_border()))
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, _, cx| this.install_paths(paths.paths().to_vec(), cx)))
            .child(body);

        let footer = (self.tab == Tab::Installed).then(|| self.render_footer(view.as_ref(), cx).into_any_element());
        let vw = f32::from(window.viewport_size().width);
        let main = drawer(
            DrawerProps { id: "collection-mods".into(), width: DRAWER_W.min(vw * 0.96) },
            header.into_any_element(),
            body.into_any_element(),
            footer,
            {
                let entity = cx.entity().downgrade();
                move |_, cx| {
                    entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
                }
            },
        );
        let menu = view.as_ref().and_then(|v| self.render_row_menu(v, cx));
        div().absolute().inset_0().child(main).children(menu)
    }
}

impl ModsDialog {
    fn render_loading(&self) -> Div {
        div().py(px(40.)).flex().justify_center().text_size(px(14.)).text_color(theme::text_dim()).child(tr!("Loading…"))
    }
}

/// "Needed by" and the other lines name mods by id: this finds the name.
pub(super) fn name_of<'a>(g: &'a GameMods, id: &'a str) -> &'a str {
    g.list.get(id).map_or(id, |m| m.name.as_str())
}
