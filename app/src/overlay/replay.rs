//! The overlay's side of the replay buffer (`services::replay`): Guide + X
//! or the panel's "Save replay" saves the last seconds of the game and marks
//! them as a moment in its journal, to write about later like any other.
//!
//! Saved from the panel, the replay asks for the time the panel was open
//! too and that time is cut off its end afterwards: the panel is in the
//! picture (the recorder captures the whole monitor).

use std::time::Duration;

use chrono::Utc;
use gpui_kit::*;
use heronite_data::journal::{self, Entry, SOURCE_OVERLAY};
use heronite_i18n::tr;

use super::moment::may_write;
use super::toast::Notice;
use super::{OverlayService, HIDE_BEFORE_CAPTURE};
use crate::icons::Icon;

impl OverlayService {
    /// Seconds a save keeps when the replay is on (for the panel's row).
    pub fn replay_seconds(cx: &App) -> Option<u32> {
        let service = crate::services::replay::get(cx)?;
        let s = service.read(cx);
        s.prefs().enabled.then_some(s.prefs().options.seconds)
    }

    /// Saves the replay of the running game and marks it in its journal.
    pub fn save_replay(&mut self, cx: &mut Context<Self>) {
        let Some(game) = self.current() else {
            self.notify(Notice::text(Icon::Videocam, tr!("No game to save a replay of"), true), cx);
            return;
        };
        let Some(replay) = crate::services::replay::get(cx) else { return };
        // The panel's time is cut off the end, and the moment is when it opened.
        let open_for = self.panel_opened.map(|t| t.elapsed() + HIDE_BEFORE_CAPTURE);
        let at = Utc::now() - chrono::Duration::from_std(open_for.unwrap_or_default()).unwrap_or_default();
        self.close_panel(cx);
        let extra = open_for.unwrap_or(Duration::ZERO);
        let saving = replay.update(cx, |r, cx| r.save(&game.game.app_name, extra, cx));
        let key = game.info.as_ref().map(|i| journal::collection_key(&i.runner, &game.game.app_name));
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            let (path, seconds) = match saved {
                Ok(s) => (s.path, s.seconds),
                Err(why) => {
                    let notice = Notice::text(Icon::Videocam, tr!("Replay not saved"), true).sub(why);
                    this.update(cx, |this, cx| this.notify(notice, cx)).ok();
                    return;
                }
            };
            if open_for.is_some() && heronite_overlay::replay::trim::available() {
                heronite_overlay::replay::trim::cut_end_later(path.clone(), extra.as_secs_f64());
            }
            let marked = match key.clone() {
                Some(key) if may_write(&paths) => {
                    let entry = Entry::new(Some(at), "", None, SOURCE_OVERLAY).with_clip(path.to_string_lossy().into_owned());
                    cx.background_spawn(async move { journal::add(&paths, &key, &entry) }).await.is_ok()
                }
                _ => false,
            };
            this.update(cx, |this, cx| {
                if marked {
                    if let Some(key) = key {
                        this.journal_changed(key, cx);
                    }
                }
                let sub = if marked {
                    tr!("The last {n} s. Write about it in Notes.", n = seconds)
                } else {
                    tr!("The last {n} s, in its replays folder.", n = seconds)
                };
                this.notify(Notice::text(Icon::Videocam, tr!("Replay saved"), false).sub(sub), cx);
            })
            .ok();
        })
        .detach();
    }
}
