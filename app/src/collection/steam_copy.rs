//! The Steam copy of a game the Collection has from another source (its
//! Steam twin, `heronite_data::steam_owned`): play it, install it or
//! uninstall it through Steam. A game whose only installed copy is Steam's
//! already plays and uninstalls through Steam with the usual actions.

use gpui_kit::prelude::*;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::game_actions::{GameAction, GameActionItem};
use super::CollectionView;
use crate::icons::Icon;

impl CollectionView {
    /// "Play with Steam", "Install with Steam" (the own copy is installed;
    /// otherwise it is one of the install ways) and "Uninstall from Steam".
    pub(super) fn steam_copy_actions(&self, game: &CollectionGame) -> Vec<GameActionItem> {
        if self.playing.contains_key(&game.app_name) {
            return Vec::new();
        }
        let item = GameActionItem::new;
        let mut v = Vec::new();
        if game.steam_also_playable().is_some() {
            v.push(item(GameAction::SteamPlay, "steam-play", tr!("Play with Steam"), Icon::PlayArrow, "launch run steam jogar iniciar"));
        }
        if game.steam_installable().is_some() && game.installed {
            v.push(item(
                GameAction::SteamInstall,
                "steam-install",
                tr!("Install with Steam"),
                Icon::Download,
                "download steam library instalar baixar biblioteca",
            ));
        }
        if game.steam_also_playable().is_some() {
            v.push(item(
                GameAction::SteamUninstall,
                "steam-uninstall",
                tr!("Uninstall from Steam"),
                Icon::DeleteForever,
                "remove delete steam desinstalar remover excluir",
            ));
        }
        v
    }

    pub(super) fn play_with_steam(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        if let Err(err) = self.runtime.play_with_steam(&game) {
            self.report_launch_error(err, cx);
        }
    }

    pub(super) fn install_with_steam(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let Some(app_id) = game.steam_installable().or(game.steam_app_id.as_deref()).map(str::to_string) else { return };
        if let Err(err) = self.runtime.install_with_steam(&app_id) {
            self.report_launch_error(err, cx);
        }
    }
}
