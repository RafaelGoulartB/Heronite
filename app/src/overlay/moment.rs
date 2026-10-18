//! The overlay's side of the game's journal: Guide + Y (or the Notes tab's
//! "Mark this moment") takes a screenshot, the panel out of the way first,
//! and adds an entry with no words yet, to write later in the Notes tab or
//! the Collection's Journal.

use chrono::Utc;
use gpui_kit::*;
use heronite_data::journal::{self, Entry, SOURCE_OVERLAY};
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_media::capture::CaptureOutcome;

use super::toast::Notice;
use super::{OverlayService, HIDE_BEFORE_CAPTURE};
use crate::icons::Icon;

/// Whether the overlay may write the journal: the debug hooks drive it with
/// scripted presses, so they leave the real data folder alone (a copy given
/// with `HERONITE_DATA_DIR` is written).
pub(super) fn may_write(paths: &DataPaths) -> bool {
    crate::env::value("HERONITE_DEBUG_OVERLAY").is_none() || !paths.is_default()
}

impl OverlayService {
    /// Marks this moment of the running game in its journal.
    pub fn mark_moment(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.current().and_then(|g| g.info.map(|i| journal::collection_key(&i.runner, &g.game.app_name))) else {
            self.notify(Notice::text(Icon::Flag, tr!("No game to mark a moment in"), true), cx);
            return;
        };
        let at = Utc::now();
        let was_open = self.panel.is_some();
        self.close_panel(cx);
        let capture = crate::services::session::capture_of(cx);
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            if was_open {
                cx.background_executor().timer(HIDE_BEFORE_CAPTURE).await;
            }
            let write = may_write(&paths);
            let saved = cx
                .background_spawn(async move {
                    let shot = match capture.map(|c| c.capture_quiet()) {
                        Some(CaptureOutcome::Saved(path)) => Some(path.to_string_lossy().into_owned()),
                        _ => None,
                    };
                    let with_shot = shot.is_some();
                    let entry = Entry::new(Some(at), "", shot, SOURCE_OVERLAY);
                    let res = if write {
                        journal::add(&paths, &key, &entry)
                    } else {
                        log::info!("overlay debug: journal entry not written");
                        Ok(())
                    };
                    res.map(|_| (key, with_shot))
                })
                .await;
            this.update(cx, |this, cx| {
                let notice = match saved {
                    Ok((key, with_shot)) => {
                        this.journal_changed(key, cx);
                        let sub =
                            if with_shot { tr!("With a screenshot. Write about it in Notes.") } else { tr!("Write about it in Notes.") };
                        Notice::text(Icon::Flag, tr!("Moment marked"), false).sub(sub)
                    }
                    Err(e) => Notice::text(Icon::Flag, tr!("Could not mark the moment"), true).sub(format!("{e:#}")),
                };
                this.notify(notice, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The journal of the game under `key` changed here: the Collection reads it again.
    pub fn journal_changed(&self, key: String, cx: &mut Context<Self>) {
        if let Some(session) = crate::services::session::get(cx) {
            session.update(cx, |_, cx| cx.emit(crate::services::session::SessionEvent::JournalChanged(key)));
        }
    }
}
