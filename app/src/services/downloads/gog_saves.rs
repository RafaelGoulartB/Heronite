//! Manual cloud saves sync of a GOG game (the automatic one around a launch
//! is `gog_sync_saves_flag` in `stores`).

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_epic::ops::OpResult;
use heronite_gog::cmd::Gogdl;
use heronite_gog::saves::{self, SyncMode};
use heronite_i18n::tr;

use super::{Downloads, DownloadsEvent};

impl Downloads {
    /// Syncs the saves of a GOG game and shows gogdl's log in a message.
    pub fn gog_sync_saves(&mut self, app_name: &str, title: &str, mode: SyncMode, cx: &mut Context<Self>) {
        let (app, title) = (app_name.to_string(), title.to_string());
        let (tx, mut rx) = unbounded::<String>();
        self.run_task_then(
            app_name,
            tr!("Syncing saves"),
            false,
            cx,
            move |data_paths, _, _| {
                let result = Gogdl::locate_or_install(data_paths).and_then(|gogdl| saves::sync(data_paths, &gogdl, &app, mode));
                match result {
                    Ok(text) => {
                        let _ = tx.unbounded_send(text);
                        OpResult { ok: true, aborted: false, warning: false, message: tr!("Cloud saves synced").into() }
                    }
                    Err(err) => OpResult { ok: false, aborted: false, warning: false, message: format!("{err:#}") },
                }
            },
            |_, _, _| {},
        );
        cx.spawn(async move |this, cx| {
            if let Some(text) = rx.next().await {
                this.update(cx, |this, cx| {
                    this.emit(cx, DownloadsEvent::Message { title: tr!("Cloud saves: {title}", title = title), text });
                })
                .ok();
            }
        })
        .detach();
    }
}
