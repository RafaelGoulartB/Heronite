//! Games GOG Galaxy installed on this PC (Windows): shown as installed GOG
//! games without downloading them again (`heronite_gog::galaxy`). Runs
//! quietly after every GOG library refresh (so after logging in) and from the
//! GOG page's button.

use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{Downloads, DownloadsEvent, Store};

impl Downloads {
    /// Whether GOG Galaxy's games are being looked for.
    pub fn gog_galaxy_importing(&self) -> bool {
        self.gog_galaxy_importing
    }

    /// Looks for the games of the library GOG Galaxy installed and shows
    /// them as installed (and takes off the ones Galaxy no longer has).
    pub fn gog_import_galaxy(&mut self, cx: &mut Context<Self>) {
        if self.gog_galaxy_importing || !self.store(Store::Gog).account.logged_in {
            return;
        }
        self.gog_galaxy_importing = true;
        cx.notify();
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_gog::galaxy::import(&paths) }).await;
            this.update(cx, |this, cx| {
                this.gog_galaxy_importing = false;
                match res {
                    Ok(report) => {
                        let n = report.added.len();
                        if n > 0 || !report.removed.is_empty() {
                            this.store_reload_local(Store::Gog, cx);
                            this.emit(cx, DownloadsEvent::LibraryChanged);
                        }
                        let text = if n > 0 {
                            trn!(n, "Added {n} game from GOG Galaxy", "Added {n} games from GOG Galaxy")
                        } else {
                            tr!("GOG Galaxy has no other games of your library").to_string()
                        };
                        this.toast(cx, text, false);
                    }
                    Err(err) => this.toast(cx, tr!("Could not add the games of GOG Galaxy: {error}", error = format!("{err:#}")), true),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
