//! An install whose folder went away outside the app: point it at the folder
//! the game was moved to, or take it off the installed list. Nothing is moved
//! or deleted here; each store checks that the new folder holds the game.

use gpui_kit::*;
use heronite_amazon::cmd::Nile;
use heronite_epic::cmd::Legendary;
use heronite_epic::ops as epic_ops;
use heronite_i18n::tr;

use super::stores::{done, failed};
use super::Downloads;

impl Downloads {
    /// The game's folder was moved or renamed by hand: the store now looks
    /// for it in `folder`.
    pub fn relocate(&mut self, runner: &str, app_name: &str, folder: &str, cx: &mut Context<Self>) {
        let (runner, app, folder) = (runner.to_string(), app_name.to_string(), folder.to_string());
        self.run_task_then(
            app_name,
            tr!("Changing the install location"),
            false,
            cx,
            move |paths, _, _| match runner.as_str() {
                "gog" => heronite_gog::ops::relocate(paths, &app, &folder).map_or_else(failed, |_| done(tr!("Install location changed"))),
                "nile" => {
                    heronite_amazon::ops::relocate(paths, &app, &folder).map_or_else(failed, |_| done(tr!("Install location changed")))
                }
                _ => match Legendary::locate(paths) {
                    Ok(lg) => epic_ops::relocate(paths, &lg, &app, &folder),
                    Err(err) => failed(err),
                },
            },
            |_, _, _| {},
        );
    }

    /// Takes the game off the installed list without touching its files (its
    /// folder is gone). The game stays in the library, ready to install again.
    pub fn forget_install(&mut self, runner: &str, app_name: &str, cx: &mut Context<Self>) {
        let (runner, app) = (runner.to_string(), app_name.to_string());
        self.run_task_then(
            app_name,
            tr!("Removing from the installed games"),
            false,
            cx,
            move |paths, _, _| match runner.as_str() {
                "gog" => heronite_gog::ops::forget(paths, &app).map_or_else(failed, |_| done(tr!("Removed from the installed games"))),
                "nile" => Nile::locate(paths)
                    .and_then(|n| heronite_amazon::ops::forget(paths, &n, &app))
                    .map_or_else(failed, |_| done(tr!("Removed from the installed games"))),
                _ => match Legendary::locate(paths) {
                    Ok(lg) => epic_ops::force_uninstall(paths, &lg, &app),
                    Err(err) => failed(err),
                },
            },
            |_, _, _| {},
        );
    }
}
