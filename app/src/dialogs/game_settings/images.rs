//! The Images tab: custom cover (of the family the desktop draws, with the
//! state of the others and "All covers…") and background, web and
//! screenshot pickers.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::cover_family::CoverFamily;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata};
use heronite_data::{art, ArtContext};
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_metadata::ArtKind;

use super::{Busy, GameSettingsDialog, Nested};
use crate::dialogs::art_images::{chain_for_url, picture};
use crate::dialogs::covers::{words, CoversDialog};
use crate::dialogs::screenshot_art::ScreenshotArtDialog;
use crate::dialogs::web_image::{ArtEvent, WebImageDialog};
use crate::dialogs::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, caption, card, ButtonKind};

/// Width of the background preview inside the drawer card.
const HERO_W: f32 = 500.;

impl GameSettingsDialog {
    /// The cover and background chains the game shows without a custom image.
    pub(super) fn default_art(ctx: &DialogCtx, game: &Game) -> (Vec<ImageRef>, Vec<ImageRef>) {
        let shots = ctx.lib.settings.screenshots_folder().map(|p| p.to_path_buf());
        let art_ctx = ArtContext::new(&ctx.data_paths, shots.as_deref());
        let old_cover = game
            .art
            .as_deref()
            .and_then(|a| a.cover_url.clone())
            .map(|u| chain_for_url(&ctx.data_paths, shots.as_deref(), &u))
            .unwrap_or_default();
        let old_hero = game
            .art
            .as_deref()
            .and_then(|a| a.hero_url.clone())
            .map(|u| chain_for_url(&ctx.data_paths, shots.as_deref(), &u))
            .unwrap_or_default();
        let default_cover: Vec<ImageRef> =
            crate::images::to_chain(&art::card_cover(&art_ctx, game)).into_iter().filter(|r| !old_cover.contains(r)).collect();
        let default_hero: Vec<ImageRef> =
            crate::images::to_chain(&art::stage_art(&art_ctx, game)).into_iter().filter(|r| !old_hero.contains(r)).collect();
        (default_cover, default_hero)
    }

    /// New fetched metadata: its cover and background replace the old ones
    /// in the default chains, where `art::card_cover` / `stage_art` put them
    /// (right after the custom image, which the chains leave out).
    pub(super) fn metadata_art_changed(&mut self, old: Option<&CollectionGameMetadata>) {
        let shots = self.ctx.lib.settings.screenshots_folder().map(|p| p.to_path_buf());
        let chain = |url: Option<&str>| url.map(|u| chain_for_url(&self.ctx.data_paths, shots.as_deref(), u)).unwrap_or_default();
        let new = self.metadata.as_deref();
        let swap = |default: &[ImageRef], old: Vec<ImageRef>, new: Vec<ImageRef>| -> Vec<ImageRef> {
            let rest = default.iter().filter(|r| !old.contains(r) && !new.contains(r)).cloned();
            new.iter().cloned().chain(rest).collect()
        };
        self.default_cover =
            swap(&self.default_cover, chain(old.and_then(|m| m.cover_url.as_deref())), chain(new.and_then(|m| m.cover_url.as_deref())));
        self.default_hero =
            swap(&self.default_hero, chain(old.and_then(|m| m.hero_url.as_deref())), chain(new.and_then(|m| m.hero_url.as_deref())));
        self.fetched_family = Self::fetched_of(&self.ctx, new, self.desktop_family());
    }

    /// The family the desktop's covers draw: what the Cover block shows.
    pub(super) fn desktop_family(&self) -> CoverFamily {
        self.ctx.lib.settings.cover_aspect.family()
    }

    /// The picture of `family` the sources gave (none for the tall one,
    /// whose chain is `default_cover`).
    pub(super) fn fetched_of(ctx: &DialogCtx, md: Option<&CollectionGameMetadata>, family: CoverFamily) -> Vec<ImageRef> {
        let shots = ctx.lib.settings.screenshots_folder();
        match family {
            CoverFamily::Tall => Vec::new(),
            f => md.and_then(|m| ArtKind::cover(f).of_record(m)).map(|u| chain_for_url(&ctx.data_paths, shots, u)).unwrap_or_default(),
        }
    }

    /// Reads which families are kept and who draws them (the console theme
    /// included), for the line under the cover.
    pub(super) fn read_families(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let families = cx.background_spawn(async move { service.cover_families() }).await;
            this.update(cx, |this, cx| {
                this.families = Some(families);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Square: cropped · Wide: not kept": the other families of this game.
    fn other_families(&self) -> Option<String> {
        let families = self.families.as_ref()?;
        let desktop = self.desktop_family();
        let md = self.metadata.as_deref();
        let parts: Vec<String> = CoverFamily::ALL
            .into_iter()
            .filter(|f| *f != desktop)
            .map(|f| {
                let name = words::family_name(f);
                let kind = ArtKind::cover(f);
                let own = f == CoverFamily::Tall
                    || kind.of_art(&self.art).is_some_and(|u| !u.is_empty())
                    || md.and_then(|m| kind.of_record(m)).is_some_and(|u| !u.is_empty());
                if !families.is_kept(f) {
                    tr!("{family}: not kept", family = name)
                } else if own {
                    tr!("{family}: own picture", family = name)
                } else {
                    tr!("{family}: cropped", family = name)
                }
            })
            .collect();
        Some(parts.join(" \u{b7} "))
    }

    /// "All covers…": the Covers dialog over the drawer.
    fn open_covers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ctx, game) = (self.ctx.clone(), self.game.clone());
        let view = cx.new(|cx| CoversDialog::new(ctx, game, None, window, cx));
        let sub = cx.subscribe(&view, |this: &mut Self, view, ev: &DialogEvent, cx| {
            match ev {
                DialogEvent::Changed | DialogEvent::ChangedAndClose => {
                    this.art = view.read(cx).art().clone();
                    cx.emit(DialogEvent::Changed);
                }
                DialogEvent::Close => {}
            }
            if matches!(ev, DialogEvent::Close | DialogEvent::ChangedAndClose) {
                this.nested = None;
            }
            cx.notify();
        });
        self.nested = Some(Nested::Covers(view, sub));
        cx.notify();
    }

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
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update(cx, |this, cx| this.set_image_file(kind, path.to_string_lossy().into_owned(), cx)).ok();
        })
        .detach();
    }

    fn set_image_file(&mut self, kind: ArtKind, path: String, cx: &mut Context<Self>) {
        self.busy = Some(Busy::Art(kind));
        self.error.clear();
        let (service, runner, app) = (self.service.clone(), self.runner(), self.game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.set_art_from_file(&runner, &app, kind, &path) }).await;
            this.update(cx, |this, cx| this.art_result(res, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    /// `HERONITE_DEBUG_SET_ART=<cover|hero|coverSquare|coverWide>:<file>@<ms>`:
    /// after `ms`, sets that image as if picked with "Change" (fixtures only:
    /// it writes the art).
    pub(super) fn debug_set_art(&mut self, cx: &mut Context<Self>) {
        let Some(spec) = crate::env::value("HERONITE_DEBUG_SET_ART") else { return };
        let (what, delay) = spec.rsplit_once('@').map(|(w, d)| (w.to_string(), d.parse().unwrap_or(0))).unwrap_or((spec.clone(), 0));
        let Some((kind, file)) = what.split_once(':') else { return };
        let kind = ArtKind::from_field(kind).unwrap_or(ArtKind::Cover);
        let file = file.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(delay)).await;
            this.update(cx, |this, cx| this.set_image_file(kind, file, cx)).ok();
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

    /// "Use defaults": drops every custom image of the game.
    pub(super) fn reset_all_images(&mut self, cx: &mut Context<Self>) {
        // Every cover family's too.
        let kinds: Vec<ArtKind> = ArtKind::ALL.into_iter().filter(|k| k.of_art(&self.art).is_some()).collect();
        let Some(&first) = kinds.first() else { return };
        self.busy = Some(Busy::Art(first));
        self.error.clear();
        let (service, runner, app) = (self.service.clone(), self.runner(), self.game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let mut last = None;
                    for kind in kinds {
                        last = Some(service.reset_art(&runner, &app, kind)?);
                    }
                    last.ok_or_else(|| anyhow::anyhow!("nothing to reset"))
                })
                .await;
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
        self.open_web(ArtKind::cover(self.desktop_family()), window, cx);
    }

    /// Custom image first, then what the game shows without one (for a
    /// square or wide cover: the sources' one, else another one cropped).
    pub(super) fn art_chain(&self, kind: ArtKind) -> (Vec<ImageRef>, Option<String>) {
        let custom_url = kind.of_art(&self.art).filter(|u| !u.is_empty()).map(str::to_owned);
        let shots = self.ctx.lib.settings.screenshots_folder().map(|p| p.to_path_buf());
        let mut chain = custom_url.as_deref().map(|u| chain_for_url(&self.ctx.data_paths, shots.as_deref(), u)).unwrap_or_default();
        match kind {
            ArtKind::Cover => chain.extend(self.default_cover.iter().cloned()),
            ArtKind::Hero => chain.extend(self.default_hero.iter().cloned()),
            other => {
                let family = other.family().expect("a cover");
                let (tall, stage) = (self.art_chain(ArtKind::Cover).0, self.art_chain(ArtKind::Hero).0);
                let (rest, _) = art::pick_family(family, &self.fetched_family, &stage, &tall);
                chain.extend(rest.into_iter().filter(|r| !chain.contains(r)).collect::<Vec<_>>());
            }
        }
        (chain, custom_url)
    }

    fn art_slot(&mut self, kind: ArtKind, cx: &mut Context<Self>) -> Div {
        let is_cover = kind != ArtKind::Hero;
        let rounded = if is_cover { 10. } else { 12. };
        let (chain, custom_url) = self.art_chain(kind);
        // Requested at the size the Collection uses; shown scaled into the card.
        let (w, h, show_w, show_h) = match kind {
            ArtKind::Cover => (132., 198., 92., 138.),
            ArtKind::CoverSquare => (168., 168., 118., 118.),
            ArtKind::CoverWide => (256., 144., 176., 99.),
            ArtKind::Hero => (840., 271., HERO_W, HERO_W * 271. / 840.),
        };
        let key = chain.first().map(|r| format!("{r:?}")).unwrap_or_default();
        let src = if chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get(&format!("gs:{key}"), chain, w, h)) };
        let preview = picture(src)
            .w(px(show_w))
            .h(px(show_h))
            .flex_none()
            .rounded(px(rounded))
            .border_1()
            .border_color(theme::ink_a(0.08))
            .into_any_element();

        let busy = self.busy.is_some();
        let tag = if is_cover { "cover" } else { "hero" };
        let busy_this = self.busy == Some(Busy::Art(kind));
        let mut actions = div().flex().flex_wrap().gap(px(8.)).child(action_button(
            SharedString::from(format!("gs-{tag}-change")),
            ButtonKind::Glass,
            if busy_this { tr!("Saving…") } else { tr!("Change") },
            !busy,
            cx.listener(move |this, _, _, cx| this.pick_image(kind, cx)),
        ));
        actions = actions.child(action_button(
            SharedString::from(format!("gs-{tag}-web")),
            ButtonKind::Glass,
            tr!("Search web"),
            !busy,
            cx.listener(move |this, _, window, cx| this.open_web(kind, window, cx)),
        ));
        if !is_cover {
            actions = actions.child(action_button(
                "gs-hero-shot",
                ButtonKind::Glass,
                tr!("Screenshot"),
                !busy,
                cx.listener(move |this, _, _, cx| this.open_screenshot(kind, cx)),
            ));
        }
        actions = actions.child(action_button(
            SharedString::from(format!("gs-{tag}-reset")),
            ButtonKind::Ghost,
            tr!("Reset"),
            !busy && custom_url.is_some(),
            cx.listener(move |this, _, _, cx| this.reset_image(kind, cx)),
        ));
        // The cover: the other families of the game, and all of them in the Covers dialog.
        let others = is_cover.then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(8.))
                .mb(px(10.))
                .mt(px(-6.))
                .children(self.other_families().map(|line| div().text_size(px(12.)).text_color(theme::text_muted()).child(line)))
                .child(
                    div()
                        .id("gs-all-covers")
                        .text_size(px(12.))
                        .text_color(theme::accent())
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::text_bright()))
                        .child(tr!("All covers\u{2026}"))
                        .on_click(cx.listener(|this, _, window, cx| this.open_covers(window, cx))),
                )
        });
        let title = match (custom_url.is_some(), kind.family().filter(|f| *f != CoverFamily::Tall)) {
            (true, None) => tr!("Custom image").to_string(),
            (false, None) => tr!("Default image").to_string(),
            (true, Some(f)) => tr!("Custom image \u{b7} {family}", family = words::family_name(f)),
            (false, Some(f)) => tr!("Default image \u{b7} {family}", family = words::family_name(f)),
        };
        let copy = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(3.))
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(title))
            .child(div().mb(px(10.)).text_size(px(12.)).text_color(theme::text_dim()).child(if is_cover {
                tr!("Shown on the game card")
            } else {
                tr!("Shown behind the game in Collection")
            }))
            .children(others)
            .child(actions);
        if is_cover {
            div().flex().items_center().gap(px(16.)).py(px(14.)).child(preview).child(copy)
        } else {
            div().flex().flex_col().gap(px(14.)).py(px(14.)).child(preview).child(copy)
        }
    }

    pub(super) fn images_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let cover = self.art_slot(ArtKind::cover(self.desktop_family()), cx);
        let hero = self.art_slot(ArtKind::Hero, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(caption(tr!("Cover")))
            .child(card([cover.into_any_element()]))
            .child(caption(tr!("Background")))
            .child(card([hero.into_any_element()]))
    }
}
