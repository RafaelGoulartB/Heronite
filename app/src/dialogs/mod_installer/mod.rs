//! The FOMOD installer wizard: a modal over the Collection that walks the
//! installer's steps. The `Wizard` (in `heronite_mods::fomod`) holds the
//! rules; this view copies the current step out of it after every change and
//! never locks it while drawing.

mod debug;
mod render;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_mods::fomod::StepView;

use super::art_images::DialogImages;
use super::{DialogCtx, DialogEvent};
use crate::services::mods::{self, FomodRequest};

pub use debug::debug_request;

pub struct ModInstallerDialog {
    req: FomodRequest,
    images: Entity<DialogImages>,
    name: String,
    version: String,
    author: String,
    step: StepView,
    /// (position of the step, steps going by the flags now).
    index: (usize, usize),
    can_next: bool,
    has_next: bool,
    has_back: bool,
    /// What the installer's own dependencies still want.
    missing: Vec<String>,
    /// The option shown on the right: (group, option) of the step.
    focus: Option<(usize, usize)>,
    /// Pixel size of the pictures seen so far, read off the UI thread.
    sizes: HashMap<PathBuf, (u32, u32)>,
    scroll: ScrollHandle,
}

impl EventEmitter<DialogEvent> for ModInstallerDialog {}

impl ModInstallerDialog {
    pub fn new(ctx: DialogCtx, req: FomodRequest, cx: &mut Context<Self>) -> Self {
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let mut this = Self {
            name: req.mod_name.clone(),
            req,
            images,
            version: String::new(),
            author: String::new(),
            step: StepView::default(),
            index: (0, 0),
            can_next: false,
            has_next: false,
            has_back: false,
            missing: Vec::new(),
            focus: None,
            sizes: HashMap::new(),
            scroll: ScrollHandle::new(),
        };
        this.refresh(cx);
        this
    }

    /// Copies the wizard's current step, locking it only for this.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Ok(w) = self.req.wizard.lock() else { return };
        let info = w.info();
        if !info.name.trim().is_empty() {
            self.name = info.name.trim().to_string();
        }
        self.version = info.version.trim().to_string();
        self.author = info.author.trim().to_string();
        self.step = w.step();
        self.index = w.step_index();
        self.can_next = w.can_next();
        self.has_next = w.has_next();
        self.has_back = w.has_back();
        self.missing = w.missing_module_dependencies();
        drop(w);
        // The picture on the right: the first option picked, else the first.
        self.focus = self
            .step
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, grp)| grp.plugins.iter().enumerate().map(move |(p, pl)| (g, p, pl.selected)))
            .find(|(_, _, selected)| *selected)
            .map(|(g, p, _)| (g, p))
            .or_else(|| (!self.step.groups.is_empty() && !self.step.groups[0].plugins.is_empty()).then_some((0, 0)));
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.read_sizes(cx);
        cx.notify();
    }

    /// Reads the size of every picture of the step (header only), off the UI thread.
    fn read_sizes(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<PathBuf> = self
            .step
            .groups
            .iter()
            .flat_map(|g| &g.plugins)
            .filter_map(|p| p.image.clone())
            .filter(|i| !self.sizes.contains_key(i))
            .collect();
        if wanted.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    wanted.into_iter().filter_map(|p| image::image_dimensions(&p).ok().map(|d| (p, d))).collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.sizes.extend(found);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, group: usize, plugin: usize, on: bool, cx: &mut Context<Self>) {
        if let Ok(mut w) = self.req.wizard.lock() {
            w.select(group, plugin, on);
        }
        let keep = self.scroll.offset();
        self.refresh(cx);
        self.scroll.set_offset(keep);
        self.focus = Some((group, plugin));
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        if self.has_next {
            if let Ok(mut w) = self.req.wizard.lock() {
                w.next();
            }
            self.refresh(cx);
        } else {
            let job = self.req.job_id;
            mods::service(cx).update(cx, |s, cx| s.finish_fomod(job, cx));
            cx.emit(DialogEvent::Close);
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if let Ok(mut w) = self.req.wizard.lock() {
            w.back();
        }
        self.refresh(cx);
    }

    /// Ends the installer: `plain` installs the archive as it is.
    fn cancel(&mut self, plain: bool, cx: &mut Context<Self>) {
        let job = self.req.job_id;
        mods::service(cx).update(cx, |s, cx| s.cancel_fomod(job, plain, cx));
        cx.emit(DialogEvent::Close);
    }

    /// Esc cancels the installer (the job must not wait for answers that never come).
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        self.cancel(false, cx);
        true
    }
}
