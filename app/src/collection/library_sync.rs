//! The Collection's side of the library sync (`services::sync`): it reads
//! the library again when a sync brought changes from another PC.

use gpui_kit::*;

use super::CollectionView;
use crate::services::sync::SyncEvent;

impl CollectionView {
    pub(super) fn watch_library_sync(&mut self, cx: &mut Context<Self>) {
        let Some(svc) = crate::services::sync::get(cx) else { return };
        cx.subscribe(&svc, |this, _, ev: &SyncEvent, cx| match ev {
            SyncEvent::Changed => this.reload(cx),
        })
        .detach();
    }
}
