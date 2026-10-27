//! The Collection's side of the store clients (`services::clients`): the
//! service starts with the app, and the Collection reloads when the games of
//! a client come in or go. The welcome and the ☰ menu read its state, so
//! they repaint when it changes.

use gpui_kit::*;
use heronite_i18n::trn;

use super::CollectionView;
use crate::services::clients::ClientsEvent;

impl CollectionView {
    pub(super) fn watch_clients(&mut self, cx: &mut Context<Self>) {
        let svc = crate::services::clients(cx);
        cx.subscribe(&svc, |this, _, ev: &ClientsEvent, cx| match ev {
            ClientsEvent::Synced(client, report) => {
                if report.added > 0 {
                    // Same words as the other launchers' toast.
                    let text =
                        trn!(report.added, "Added {n} game from {launcher}", "Added {n} games from {launcher}", launcher = client.name());
                    this.show_toast(text, false, cx);
                }
                if report.added + report.updated > 0 {
                    this.reload(cx);
                }
            }
            ClientsEvent::Removed(removed) => {
                if *removed > 0 {
                    this.reload(cx);
                }
            }
        })
        .detach();
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
    }
}
