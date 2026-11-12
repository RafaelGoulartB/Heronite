//! What the dialog asks and saves, all off the UI thread: the search of the
//! picked slot, a picture picked, a file, a screenshot, Reset, and keeping a
//! family. Each save takes the game's pictures as saved (`art.json`), builds
//! the slots again and announces `DialogEvent::Changed`.

use heronite_data::cover_family::CoverFamily;
use heronite_data::models::CollectionGameArt;
use heronite_i18n::tr;
use heronite_metadata::web_images::{cover_query, wallpaper_query};
use heronite_metadata::{ArtImages, MetadataApplyArgs};

use gpui_kit::*;

use super::slots::Slot;
use super::{found, Busy, CoversDialog};
use crate::dialogs::screenshot_art::ScreenshotArtDialog;
use crate::dialogs::web_image::ArtEvent;
use crate::dialogs::DialogEvent;

impl CoversDialog {
    /// Reads which families are kept and who draws them (the console theme
    /// included).
    pub(super) fn read_families(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let families = cx.background_spawn(async move { service.cover_families() }).await;
            this.update(cx, |this, cx| {
                this.families = families;
                this.compute_slots();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn pick_slot(&mut self, slot: Slot, cx: &mut Context<Self>) {
        if self.slot == slot {
            return;
        }
        self.slot = slot;
        self.source = None;
        self.error.clear();
        self.search(cx);
        cx.notify();
    }

    /// Searches the picked slot's pictures with the words typed (the game's
    /// title to start), or shows the ones already found for them.
    pub(super) fn search(&mut self, cx: &mut Context<Self>) {
        // Every search, even one shown from before, makes the answers of
        // those still running stale: they must not replace what is shown.
        self.request += 1;
        self.selected = None;
        self.visible = found::PAGE;
        self.results = ArtImages::default();
        if !self.kept(self.slot) {
            self.loading = false;
            return;
        }
        let typed = self.query.read(cx).value().split_whitespace().collect::<Vec<_>>().join(" ");
        if typed.is_empty() {
            return;
        }
        if let Some(found) = self.searched.get(&(self.slot, typed.clone())) {
            self.results = found.clone();
            self.loading = false;
            cx.notify();
            return;
        }
        let (id, slot) = (self.request, self.slot);
        self.loading = true;
        self.error.clear();
        if let Some(fake) = &self.fake {
            let results = fake.results(slot);
            self.found_for(id, slot, typed, Ok(results), cx);
            return;
        }
        // The words alone, as typed, would find anything: the query asks
        // for a cover (or a wallpaper) of that title, as the web picker does.
        let query = match slot {
            _ if typed.contains('"') => typed.clone(),
            Slot::Background => wallpaper_query(&typed),
            Slot::Cover(_) => cover_query(&typed),
        };
        let (service, kind) = (self.service.clone(), slot.kind());
        let game = MetadataApplyArgs {
            app_name: self.game.app_name.clone(),
            runner: self.runner.clone(),
            title: self.game.title.clone(),
            ..Default::default()
        };
        let words = typed.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.search_game_art_images(&game, &words, &query, kind) }).await;
            this.update(cx, |this, cx| this.found_for(id, slot, typed, res, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn found_for(&mut self, id: u64, slot: Slot, typed: String, res: anyhow::Result<ArtImages>, cx: &mut Context<Self>) {
        if self.request != id {
            return;
        }
        self.loading = false;
        match res {
            Ok(list) => {
                self.searched.insert((slot, typed), list.clone());
                if self.slot == slot {
                    self.results = list;
                    self.selected = None;
                }
            }
            Err(err) => self.error = format!("{err:#}"),
        }
        cx.notify();
    }

    /// A save ended: the slots show what was saved, the Collection reloads.
    fn saved(&mut self, res: anyhow::Result<CollectionGameArt>, cx: &mut Context<Self>) {
        self.busy = None;
        match res {
            Ok(art) => {
                self.art = art;
                self.selected = None;
                self.compute_slots();
                if self.fake.is_none() {
                    cx.emit(DialogEvent::Changed);
                }
            }
            Err(err) => self.error = format!("{err:#}"),
        }
        cx.notify();
    }

    /// Runs `save` off the UI thread (made up: `fake` on the pictures in memory).
    fn save(
        &mut self,
        save: impl FnOnce(&heronite_metadata::MetadataService, &str, &str) -> anyhow::Result<CollectionGameArt> + Send + 'static,
        fake: impl FnOnce(&mut CollectionGameArt),
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::Saving);
        self.error.clear();
        if self.fake.is_some() {
            let mut art = self.art.clone();
            fake(&mut art);
            self.saved(Ok(art), cx);
            return;
        }
        let (service, runner, app) = (self.service.clone(), self.runner.clone(), self.game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { save(&service, &runner, &app) }).await;
            this.update(cx, |this, cx| this.saved(res, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    /// "Use for <family>": downloads the picture picked (`pick`, else the
    /// selection) as the slot's.
    pub(super) fn use_selected(&mut self, pick: Option<usize>, cx: &mut Context<Self>) {
        let Some(image) = pick.or(self.selected).and_then(|i| self.found().get(i).map(|f| f.image.clone())) else {
            return;
        };
        self.selected = pick.or(self.selected);
        let kind = self.kind();
        let url = image.image_url.clone();
        self.save(
            move |s, runner, app| s.set_art_from_url(runner, app, kind, &image.image_url, Some(&image.thumb_url)),
            move |art| *kind.of_art_mut(art) = Some(url),
            cx,
        );
    }

    pub(super) fn pick_file(&mut self, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update(cx, |this, cx| {
                let kind = this.kind();
                let file = path.to_string_lossy().into_owned();
                let shown = file.clone();
                this.save(
                    move |s, runner, app| s.set_art_from_file(runner, app, kind, &file),
                    move |art| *kind.of_art_mut(art) = Some(shown),
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// "Reset": back to what the sources gave for the slot.
    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        let kind = self.kind();
        self.save(move |s, runner, app| s.reset_art(runner, app, kind), move |art| *kind.of_art_mut(art) = None, cx);
    }

    pub(super) fn open_screenshot(&mut self, cx: &mut Context<Self>) {
        let (ctx, game, kind) = (self.ctx.clone(), self.game.clone(), self.kind());
        let view = cx.new(|cx| ScreenshotArtDialog::new(ctx, game, kind, cx));
        let sub = cx.subscribe(&view, |this: &mut Self, _, ev: &ArtEvent, cx| {
            match ev {
                ArtEvent::Changed(art) => this.saved(Ok(art.clone()), cx),
                ArtEvent::Close => this.screenshot = None,
            }
            cx.notify();
        });
        self.screenshot = Some((view, sub));
        cx.notify();
    }

    /// Keeps `family` on this PC (`keepCoverFamilies`): updates fill it from
    /// now on, and its pictures can be searched here.
    pub(super) fn keep_family(&mut self, family: CoverFamily, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::Keeping);
        if self.fake.is_some() {
            let mut families = self.families.clone();
            families.states.iter_mut().filter(|s| s.family == family).for_each(|s| s.kept_by_choice = true);
            self.families_kept(families, family, cx);
            return;
        }
        let (paths, service) = (self.ctx.data_paths.clone(), self.service.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    heronite_data::settings::set_cover_family_kept(&paths, family, true)?;
                    Ok::<_, anyhow::Error>(service.cover_families())
                })
                .await;
            this.update(cx, |this, cx| match res {
                Ok(families) => {
                    this.families_kept(families, family, cx);
                    // The Collection's copy of the settings follows.
                    cx.emit(DialogEvent::Changed);
                }
                Err(err) => {
                    this.busy = None;
                    this.error = format!("{err:#}");
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn families_kept(&mut self, families: heronite_data::cover_family::CoverFamilies, family: CoverFamily, cx: &mut Context<Self>) {
        self.busy = None;
        self.families = families;
        self.compute_slots();
        self.slot = Slot::Cover(family);
        self.search(cx);
        cx.notify();
    }
}
