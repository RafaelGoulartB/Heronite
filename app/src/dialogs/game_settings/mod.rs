//! `GameSettingsDialog` ("Game settings"): Images, Details and Launch tabs for
//! one game, plus Heroic's own settings tabs.

mod console;
mod details;
mod images;
mod launch;

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::models::CollectionGameArt;
use heronite_images::ImageRef;
use heronite_metadata::{ArtKind, MetadataService};

use super::art_images::{picture, DialogImages};
use super::heroic_settings::{GameFacts, HeroicSettingsPanel, Section, Target};
use super::metadata_wizard::hub;
use super::screenshot_art::ScreenshotArtDialog;
use super::web_image::WebImageDialog;
use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, close_button, dialog_title, kicker, tabs, ButtonKind};
use crate::ui::dialog::{drawer, DrawerProps};
use details::{form_from, Details, Form};
use launch::LaunchState;

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
    /// Console mode: focused row and the open keyboard / picker.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for GameSettingsDialog {}

impl GameSettingsDialog {
    pub fn new(ctx: DialogCtx, game: Game, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();

        let (default_cover, default_hero) = Self::default_art(&ctx, &game);
        let initial = form_from(&game.title, game.metadata.as_deref(), game.meta.as_deref());
        let details = Self::build_details(&ctx, &initial, window, cx);
        let launch = Self::build_launch(&game, window, cx);
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
            console_list: Default::default(),
            layer: Default::default(),
        };
        if let Some(text) = this.launch.as_ref().map(|l| l.initial_exe.clone()) {
            this.refresh_launch(&text);
        }
        this
    }

    fn runner(&self) -> String {
        if self.game.runner == "zoom" { "sideload".into() } else { self.game.runner.clone() }
    }

    /// Cover, kicker, title and the "store, installed, size" line.
    fn drawer_header(&mut self, on_close: impl Fn(&mut Window, &mut App) + 'static, cx: &mut Context<Self>) -> Div {
        let (chain, _) = self.art_chain(ArtKind::Cover);
        let key = chain.first().map(|r| format!("{r:?}")).unwrap_or_default();
        let src = if chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get(&format!("gs-head:{key}"), chain, 104., 140.)) };
        let cover = picture(src).w(px(52.)).h(px(70.)).flex_none().rounded(px(10.)).border_1().border_color(theme::white_a(0.10));
        let mut meta = vec![self.game.source_label.to_string()];
        meta.push(if self.game.installed { "Installed".into() } else { "Not installed".into() });
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
                    .child(kicker("Game settings"))
                    .child(dialog_title(self.title.clone()).truncate())
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).truncate().child(meta.join(" \u{b7} "))),
            )
            .child(div().self_start().child(close_button("gs-close-x").on_click(move |_, w, cx| on_close(w, cx))))
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

        let dirty = self.read_form(cx) != self.initial;
        // "Use defaults" only where a reset exists: the custom images.
        let can_reset = self.tab == Tab::Images
            && self.busy.is_none()
            && (self.art.cover_url.is_some() || self.art.hero_url.is_some());
        let left = div().child(if self.tab == Tab::Images {
            action_button(
                "gs-defaults",
                ButtonKind::Ghost,
                "Use defaults",
                can_reset,
                cx.listener(|this, _, _, cx| this.reset_all_images(cx)),
            )
            .into_any_element()
        } else {
            div().into_any_element()
        });
        let mut right = div().flex().items_center().gap(px(8.));
        if self.tab == Tab::Details {
            right = right.child(action_button(
                "gs-save",
                ButtonKind::Primary,
                if self.busy == Some(Busy::Details) { "Saving\u{2026}" } else { "Save" },
                self.busy.is_none() && dirty,
                cx.listener(|this, _, _, cx| this.save_details(cx)),
            ));
        }
        right = right.child(
            button("gs-close", if self.tab == Tab::Details { ButtonKind::Glass } else { ButtonKind::Primary }, "Done")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
        );
        let footer = div().flex().w_full().items_center().justify_between().child(left).child(right);

        let entity = cx.entity().downgrade();
        let header_close = {
            let entity = entity.clone();
            move |_: &mut Window, cx: &mut App| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
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
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        );
        let nested: Option<AnyView> = match &self.nested {
            Some(Nested::Web(v, _)) => Some(v.clone().into()),
            Some(Nested::Screenshot(v, _)) => Some(v.clone().into()),
            None => None,
        };
        div().absolute().inset_0().child(main).children(nested).into_any_element()
    }
}

/// Drawer width (px).
const DRAWER_W: f32 = 580.;
