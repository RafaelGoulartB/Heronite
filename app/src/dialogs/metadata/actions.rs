//! What the dialog asks and saves, and the rules of its picks and locks.
//! The sources are asked off the UI thread; Apply saves the preview on
//! screen with the picks, pins and locks (`apply_preview`), or only the
//! locks when no source answered (`set_field_locks`).

use heronite_data::settings::METADATA_FIELDS;
use heronite_metadata::{MetadataApplyArgs, MetadataPreview, SourceAnswer};
use indexmap::IndexMap;

use gpui_kit::*;

use super::{format_value, Busy, MetadataDialog, KEEP};
use crate::dialogs::metadata_update::words;
use crate::dialogs::DialogEvent;

impl MetadataDialog {
    fn args(&self) -> MetadataApplyArgs {
        MetadataApplyArgs {
            app_name: self.game.app_name.clone(),
            runner: self.runner.clone(),
            title: self.title.clone(),
            steam_app_id: self.game.steam_app_id.clone(),
            source: Some("auto".into()),
            overwrite_existing: Some(true),
            preserve_manual: Some(false),
            pins: (!self.pins.is_empty()).then(|| self.pins.clone()),
            ..Default::default()
        }
    }

    /// Asks every source (with the pins changed here) off the UI thread.
    pub(super) fn run_preview(&mut self, cx: &mut Context<Self>) {
        if self.busy == Some(Busy::Apply) {
            return;
        }
        self.busy = Some(Busy::Preview);
        self.error = None;
        if let Some(fake) = &self.fake {
            let preview = fake.preview(&self.game, &self.pins);
            self.previewed(Ok(preview));
            self.read_sizes(cx);
            return;
        }
        let service = self.service.clone();
        let args = self.args();
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.preview_game_metadata(&args) }).await;
            this.update(cx, |this, cx| {
                this.previewed(res);
                this.read_sizes(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn previewed(&mut self, res: anyhow::Result<MetadataPreview>) {
        self.busy = None;
        match res {
            Ok(p) => {
                // A pick of a source that no longer has a value goes back
                // to what the merge proposes.
                self.chosen.retain(|field, source| {
                    source == KEEP || p.candidates.iter().any(|c| &c.source == source && !format_value(Some(&c.metadata), field).is_empty())
                });
                self.preview = Some(p);
            }
            Err(err) => self.error = Some(format!("{err:#}")),
        }
    }

    /// The record locks `field` and it was not changed here: what the game
    /// has stays, whatever is picked.
    pub(super) fn frozen(&self, field: &str) -> bool {
        !self.locks.contains_key(field) && self.preview.as_ref().is_some_and(|p| p.locks.iter().any(|f| f == field))
    }

    pub(super) fn locked(&self, field: &str) -> bool {
        self.locks.get(field).copied().unwrap_or_else(|| self.preview.as_ref().is_some_and(|p| p.locks.iter().any(|f| f == field)))
    }

    pub(super) fn toggle_lock(&mut self, field: &str) {
        let now = !self.locked(field);
        let saved = self.preview.as_ref().is_some_and(|p| p.locks.iter().any(|f| f == field));
        if now == saved {
            self.locks.shift_remove(field);
        } else {
            self.locks.insert(field.to_owned(), now);
        }
    }

    /// What the merge proposes for `field`: its source, else keeping what
    /// the game has.
    pub(super) fn default_pick(&self, field: &str) -> String {
        let Some(p) = &self.preview else { return KEEP.into() };
        match p.proposed.field_sources.get(field) {
            Some(source) if p.candidates.iter().any(|c| &c.source == source && !format_value(Some(&c.metadata), field).is_empty()) => {
                source.clone()
            }
            _ => KEEP.into(),
        }
    }

    /// The pick of `field`: [`KEEP`] or a source id.
    pub(super) fn pick(&self, field: &str) -> String {
        if self.frozen(field) {
            return KEEP.into();
        }
        self.chosen.get(field).cloned().unwrap_or_else(|| self.default_pick(field))
    }

    pub(super) fn set_pick(&mut self, field: &str, source: &str) {
        if source == self.default_pick(field) {
            self.chosen.shift_remove(field);
        } else {
            self.chosen.insert(field.to_owned(), source.to_owned());
        }
    }

    /// The value `source` ([`KEEP`]: the game) has for `field`.
    pub(super) fn value_of(&self, field: &str, source: &str) -> String {
        if source == KEEP {
            return format_value(self.current(), field);
        }
        let candidate = self.preview.as_ref().and_then(|p| p.candidates.iter().find(|c| c.source == source));
        format_value(candidate.map(|c| &c.metadata), field)
    }

    /// The fields with a value now or from some source, in the usual order,
    /// with a cover row per kept family after the tall one (always shown:
    /// "More options…" sits there).
    pub(super) fn visible_fields(&self) -> Vec<&'static str> {
        let Some(p) = &self.preview else { return Vec::new() };
        let mut out = Vec::new();
        for f in METADATA_FIELDS {
            if f == "cover" {
                out.extend(self.kept.iter().map(|family| family.field()));
                continue;
            }
            let has = !format_value(p.current.as_ref(), f).is_empty()
                || p.candidates.iter().any(|c| !format_value(Some(&c.metadata), f).is_empty());
            if has {
                out.push(f);
            }
        }
        out
    }

    /// The sources "Everything from…" offers: those that found the game.
    pub(super) fn everything_sources(&self) -> Vec<(String, String)> {
        let Some(p) = &self.preview else { return Vec::new() };
        p.candidates.iter().map(|c| (c.source.clone(), words::source_name(&self.service, &c.source))).collect()
    }

    /// Picks `source` for every field it has (locked fields stay).
    pub(super) fn take_everything_from(&mut self, source: &str) {
        for field in self.visible_fields() {
            if !self.frozen(field) && !self.value_of(field, source).is_empty() {
                self.set_pick(field, source);
            }
        }
    }

    /// Pins `id` as the game of `source` (`None`: find it again by itself)
    /// and asks the sources again.
    pub(super) fn pin(&mut self, source: &str, id: Option<String>, cx: &mut Context<Self>) {
        self.pins.insert(source.to_owned(), id);
        self.change = None;
        self.run_preview(cx);
    }

    /// Whether `source`'s match is pinned (here or in the record).
    pub(super) fn pinned(&self, source: &str) -> bool {
        match self.pins.get(source) {
            Some(pin) => pin.is_some(),
            None => self
                .preview
                .as_ref()
                .and_then(|p| p.sources.iter().find(|s| s.source == source))
                .is_some_and(|s| matches!(s.answer, SourceAnswer::Found { pinned: true, .. })),
        }
    }

    /// The preview holds something a source found.
    fn has_fetch(&self) -> bool {
        self.preview.as_ref().is_some_and(|p| !p.offline && !p.candidates.is_empty())
    }

    pub(super) fn can_apply(&self) -> bool {
        self.busy.is_none() && (self.has_fetch() || !self.locks.is_empty())
    }

    /// Saves what is on screen and closes.
    pub(super) fn apply(&mut self, cx: &mut Context<Self>) {
        if !self.can_apply() {
            return;
        }
        if self.fake.is_some() {
            cx.emit(DialogEvent::Close);
            return;
        }
        let Some(preview) = self.preview.clone() else { return };
        self.busy = Some(Busy::Apply);
        self.error = None;
        let service = self.service.clone();
        let mut args = self.args();
        let fields = self.visible_fields();
        args.field_picks = Some(fields.iter().map(|f| (f.to_string(), self.pick(f))).collect());
        args.locks = (!self.locks.is_empty()).then(|| self.locks.clone());
        let fetched = self.has_fetch();
        let locks: IndexMap<String, bool> = self.locks.clone();
        // Saves the preview the user is looking at; asking the sources again
        // could answer differently, or not at all.
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if fetched {
                        service.apply_preview(&preview, &args).map(Some)
                    } else {
                        service.set_field_locks(&args.runner, &args.app_name, &locks)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok(md) => {
                        this.applied = md;
                        cx.emit(DialogEvent::ChangedAndClose);
                    }
                    Err(err) => this.error = Some(format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// `HERONITE_DEBUG_DIALOG=metadata:<state>` (on the made-up preview):
    /// `change` opens "Change" on the first source that can be searched.
    pub(super) fn debug_state(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(fake) = &self.fake {
            for (field, source) in fake.picks() {
                self.set_pick(field, source);
            }
        }
        if state == "change" {
            let first = self.preview.as_ref().and_then(|p| p.sources.first()).map(|s| s.source.clone());
            if let Some(source) = first {
                self.open_change(&source, window, cx);
            }
        }
    }
}
