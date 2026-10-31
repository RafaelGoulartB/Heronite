//! Games the Amazon Games app installed on this PC (Windows): brought in as
//! installed Amazon games with `nile import`, so they are not downloaded
//! again. Runs after an Amazon sign-in (quietly: nothing is said when the
//! app has no games to bring) and from the Amazon page's button.

use gpui_kit::*;
use heronite_amazon::cmd::Nile;
use heronite_i18n::{tr, trn};

use super::{Downloads, DownloadsEvent, Store};

impl Downloads {
    /// Whether games of the Amazon Games app are being brought in.
    pub fn amazon_app_importing(&self) -> bool {
        self.amazon_app_importing
    }

    /// Brings in the games of the Amazon Games app that are in the library
    /// and not installed yet. `quiet`: only say something when a game was
    /// brought in or refused.
    pub fn amazon_import_app_games(&mut self, quiet: bool, cx: &mut Context<Self>) {
        if self.amazon_app_importing || !self.store(Store::Amazon).account.logged_in {
            return;
        }
        let Some(db) = heronite_amazon::amazon_app::database() else {
            if !quiet {
                self.toast(cx, tr!("The Amazon Games app was not found on this PC"), true);
            }
            return;
        };
        self.amazon_app_importing = true;
        cx.notify();
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let nile = Nile::locate_or_install(&paths)?;
                    heronite_amazon::amazon_app::import_all(&paths, &nile, &db, &|| false)
                })
                .await;
            this.update(cx, |this, cx| {
                this.amazon_app_importing = false;
                match res {
                    Ok(report) => {
                        let (n, failed) = (report.imported.len(), report.failed.len());
                        for (title, why) in &report.failed {
                            log::warn!("amazon: {title} could not be brought in from the Amazon Games app: {why}");
                        }
                        if n > 0 {
                            this.store_reload_local(Store::Amazon, cx);
                            this.emit(cx, DownloadsEvent::LibraryChanged);
                        }
                        if failed > 0 {
                            let (title, why) = &report.failed[0];
                            let first = why.lines().next().unwrap_or_default();
                            let text = trn!(
                                failed,
                                "{n} game of the Amazon Games app could not be added ({title}: {error})",
                                "{n} games of the Amazon Games app could not be added ({title}: {error})",
                                title = title,
                                error = first
                            );
                            this.toast(cx, text, true);
                        }
                        if n > 0 {
                            this.toast(
                                cx,
                                trn!(n, "Added {n} game from the Amazon Games app", "Added {n} games from the Amazon Games app"),
                                false,
                            );
                        } else if failed == 0 && !quiet {
                            this.toast(cx, tr!("The Amazon Games app has no other games to add"), false);
                        }
                    }
                    Err(err) => {
                        let msg = format!("{err:#}");
                        log::warn!("amazon: reading the Amazon Games app's games: {msg}");
                        if !quiet {
                            this.toast(cx, tr!("Could not add the games of the Amazon Games app: {error}", error = msg), true);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
