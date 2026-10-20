//! The Collection's side of the other launchers' import
//! (`services::launcher_imports`): the service starts with the Collection
//! (and keeps the launchers turned on in sync), and the Collection reloads
//! when their games come in or go.

use gpui_kit::*;
use heronite_i18n::trn;

use super::CollectionView;
use crate::services::launcher_imports::LauncherImportsEvent;

impl CollectionView {
    pub(super) fn watch_launcher_imports(&mut self, cx: &mut Context<Self>) {
        let svc = crate::services::launcher_imports(cx);
        cx.subscribe(&svc, |this, _, ev: &LauncherImportsEvent, cx| {
            let LauncherImportsEvent::Imported(launcher, report) = ev;
            if report.added > 0 {
                let text =
                    trn!(report.added, "Added {n} game from {launcher}", "Added {n} games from {launcher}", launcher = launcher.name());
                this.show_toast(text, false, cx);
            }
            this.reload(cx);
        })
        .detach();
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
    }
}
