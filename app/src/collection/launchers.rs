//! The Collection's side of the other launchers' import
//! (`services::launcher_imports`): the service starts with the Collection
//! (and keeps the launchers turned on in sync), and the Collection reloads
//! when their games come in or go. A game found in the Collection twice gets
//! a toast whose Review opens Settings → Other launchers, where it is merged.

use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{CollectionView, ToastAction};
use crate::services::launcher_imports::LauncherImportsEvent;

impl CollectionView {
    pub(super) fn watch_launcher_imports(&mut self, cx: &mut Context<Self>) {
        let svc = crate::services::launcher_imports(cx);
        cx.subscribe(&svc, |this, _, ev: &LauncherImportsEvent, cx| match ev {
            LauncherImportsEvent::Imported(launcher, report) => {
                if report.added > 0 {
                    let text =
                        trn!(report.added, "Added {n} game from {launcher}", "Added {n} games from {launcher}", launcher = launcher.name());
                    this.show_toast(text, false, cx);
                }
                this.reload(cx);
            }
            LauncherImportsEvent::Duplicates(new) => {
                let text = match new.as_slice() {
                    [one] => tr!(
                        "{title} is in your Collection twice: added by you and from {launcher}.",
                        title = one.theirs.title,
                        launcher = one.launcher.name()
                    ),
                    _ => trn!(new.len(), "{n} game is in your Collection twice.", "{n} games are in your Collection twice."),
                };
                this.show_toast(text, false, cx);
                this.toast_action = Some(ToastAction::ReviewDuplicates);
            }
            LauncherImportsEvent::Resolved => this.reload(cx),
        })
        .detach();
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
    }
}
