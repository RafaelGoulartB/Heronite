//! "ReShade" in a game's menu: shown only when ReShade fits the game with
//! no known problem (`heronite_launch::reshade::fits`). That check reads the
//! game's programs, so it runs in the background the first time the menu
//! asks and is kept for a while; the menu redraws when it ends.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_data::{collection::Game, DataPaths};
use heronite_launch::reshade::{self, Facts};

use super::CollectionView;
use crate::dialogs::settings_panel::logic::RunKind;
use crate::dialogs::settings_panel::{GameFacts, ReshadePlace as Place};

/// How long an answer stays good (a game installed or changed meanwhile is
/// seen again after it).
const KEEP: Duration = Duration::from_secs(60);

thread_local! {
    /// Per game key: when it was asked and the answer (`None`: running).
    static FITS: RefCell<HashMap<String, (Instant, Option<bool>)>> = RefCell::default();
}

/// Whether ReShade fits `game` (blocking: reads its files).
fn check(paths: &DataPaths, game: &Game) -> bool {
    let facts = GameFacts::of(game);
    let Some(wine) = facts.run_kind.reshade() else { return false };
    let install = &game.game.install;
    let steam = game.steam_launch_id.as_deref().filter(|_| matches!(facts.run_kind, RunKind::Steam | RunKind::SteamDirect));
    // A Steam game's Linux build is a Linux game: no ReShade.
    let Some(place) = Place::find(paths, steam, install.install_path.clone(), install.executable.clone(), wine) else { return false };
    let home = std::env::var("HOME").unwrap_or_default();
    let settings = heronite_data::game_config::load_game(paths, &game.app_name, &home);
    let rs = reshade::ReShade::from_map(&settings);
    let target_exe = settings.get("targetExe").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let f = Facts {
        game_id: &game.app_name,
        title: &game.title,
        steam_app_id: facts.protondb_id.as_deref(),
        install_path: place.install_path.as_deref(),
        executable: place.executable.as_deref(),
        target_exe: &target_exe,
        settings: &rs,
        env_options: &[],
        wine: place.wine,
    };
    reshade::fits(paths, &f)
}

impl CollectionView {
    /// The cached answer for the game with `key`; starts the check when there
    /// is none (or it is old) and answers `false` until it ends.
    pub(super) fn reshade_fits(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let cached = FITS.with(|f| f.borrow().get(key).cloned());
        if let Some((at, answer)) = cached {
            if at.elapsed() < KEEP {
                return answer.unwrap_or(false);
            }
        }
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return false };
        let paths = self.data_paths.clone();
        let key = key.to_string();
        // The old answer stands while the new one is worked out.
        FITS.with(|f| f.borrow_mut().insert(key.clone(), (Instant::now(), cached.and_then(|c| c.1))));
        cx.spawn(async move |this, cx| {
            let ok = cx.background_spawn(async move { check(&paths, &game) }).await;
            FITS.with(|f| f.borrow_mut().insert(key, (Instant::now(), Some(ok))));
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        false
    }

    /// Opens the game's settings on the ReShade section.
    pub(super) fn open_reshade(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(g)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
        // The answer may change once the user acts there.
        FITS.with(|f| f.borrow_mut().remove(key));
        crate::dialogs::settings_panel::focus_reshade();
        let tab = crate::dialogs::game_settings::Tab::Performance;
        let view = cx.new(|cx| crate::dialogs::game_settings::GameSettingsDialog::new(ctx, g, tab, window, cx));
        self.open_dialog(view, cx);
    }
}
