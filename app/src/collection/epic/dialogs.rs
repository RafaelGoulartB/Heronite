//! Epic dialogs opened from the context menu and the confirmations behind
//! them.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config;
use serde_json::Value;

use super::Request;
use crate::collection::CollectionView;
use crate::dialogs::epic_confirm::{Confirm, EpicConfirmDialog, Toggle};
use crate::dialogs::epic_install::{EpicInstallDialog, Mode};
use crate::dialogs::epic_modify::EpicModifyDialog;
use heronite_i18n::tr;

impl CollectionView {
    /// Context-menu actions that need no dialog (the row id tells which).
    pub(super) fn epic_direct(&mut self, key: &str, cx: &mut Context<Self>) {
        // `update` and `sync`: decided by what the game offers.
        let Some(game) = self.epic_game(key) else { return };
        let svc = self.epic_service(cx);
        let update = svc.read(cx).has_update(&game.app_name);
        if update {
            self.epic_update(key, cx);
        } else {
            let (app, title) = (game.app_name.clone(), game.title.clone());
            svc.update(cx, |s, cx| s.sync_saves(&app, &title, cx));
        }
    }

    pub(super) fn epic_open_modify(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        let info = Self::epic_game_info(&game);
        let data_paths = self.data_paths.clone();
        let view = cx.new(|cx| EpicModifyDialog::new(data_paths, info, cx));
        self.open_dialog(view, cx);
    }

    /// `enable()` of the EOS overlay without the overlay: offers to install it.
    pub(super) fn epic_open_eos_install(&mut self, app_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut spec = Confirm::new(
            "epic-eos-install",
            tr!("Overlay not installed"),
            tr!("The EOS Overlay is not installed. Do you want to install it now?"),
        );
        spec.confirm = Some(tr!("Install").into());
        let app = app_name.to_string();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |_, cx| {
                    let svc = crate::services::downloads(cx);
                    let app = app.clone();
                    svc.update(cx, |s, cx| s.eos_install_and_enable(&app, cx));
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    pub(super) fn epic_open_install(&mut self, key: &str, import: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        let info = Self::epic_game_info(&game);
        let (data_paths, app) = (self.data_paths.clone(), self.app_paths.clone());
        let mode = if import { Mode::Import } else { Mode::Install };
        let view = cx.new(|cx| EpicInstallDialog::new(data_paths, app, info, mode, window, cx));
        self.open_dialog(view, cx);
    }

    /// Whether a download is running; then `message` is shown as an error toast.
    pub(super) fn epic_busy_guard(&mut self, cx: &mut Context<Self>, message: &str) -> bool {
        let svc = self.epic_service(cx);
        let downloading = svc.read(cx).snapshot().state == heronite_epic::manager::DmState::Running;
        if downloading {
            self.show_toast(message.to_string(), true, cx);
        }
        downloading
    }

    pub(super) fn epic_open_uninstall(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        if self.epic_busy_guard(cx, tr!("Uninstalling is not possible while an Epic game is being installed.")) {
            return;
        }
        if self.playing.contains_key(&game.app_name) {
            self.show_toast(tr!("{title} is running. Close the game to uninstall it.", title = game.title), true, cx);
            return;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let settings = game_config::load_game(&self.data_paths, &game.app_name, &home);
        let global = game_config::load_global(&self.data_paths, &home);
        let prefix = settings.get("winePrefix").and_then(Value::as_str).unwrap_or_default().to_string();
        let default_prefix = global.get("winePrefix").and_then(Value::as_str).unwrap_or_default().to_string();
        let windows = game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("windows"));
        let mut spec = Confirm::new("epic-uninstall", tr!("Uninstall"), tr!("Do you want to uninstall \"{title}\"?", title = game.title));
        if windows && cfg!(target_os = "linux") && !prefix.is_empty() {
            let protect = prefix == default_prefix;
            spec.toggles.push(Toggle {
                label: tr!("Remove prefix: {prefix}", prefix),
                value: false,
                disabled: protect,
                note: Some(if protect {
                    tr!("The Wine prefix for this game is the default prefix. If you really want to delete it, you have to do it manually.")
                        .into()
                } else {
                    tr!("Note: This can't be undone and will also remove not backed up save files.").into()
                }),
            });
        }
        spec.toggles.push(Toggle {
            label: tr!("Erase settings and remove log").into(),
            value: false,
            disabled: false,
            note: Some(tr!("Note: This can't be undone. Any modified settings will be forgotten and log will be deleted.").into()),
        });
        spec.confirm = Some(tr!("Uninstall").into());
        spec.danger = true;
        let has_prefix_toggle = spec.toggles.len() == 2;
        let app = game.app_name.clone();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |choice, cx| {
                    let (prefix, settings) =
                        if has_prefix_toggle { (choice.toggles[0], choice.toggles[1]) } else { (false, choice.toggles[0]) };
                    let svc = crate::services::downloads(cx);
                    let app = app.clone();
                    svc.update(cx, |s, cx| s.uninstall(&app, prefix, settings, cx));
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    pub(super) fn epic_open_move(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        if self.epic_busy_guard(cx, tr!("Moving a game is not possible while an Epic game is being installed.")) {
            return;
        }
        let Some(current) = game.game.install.install_path.clone() else { return };
        let mut spec = Confirm::new(
            "epic-move",
            tr!("Move Game Installation"),
            tr!("{title}\nThis can take a long time, are you sure?", title = game.title),
        );
        let parent = std::path::Path::new(&current).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        spec.path = Some((tr!("Choose where you want to move").into(), parent, tr!("Choose where you want to move").into()));
        spec.confirm = Some(tr!("Move").into());
        let app = game.app_name.clone();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |choice, cx| {
                    let svc = crate::services::downloads(cx);
                    let (app, from) = (app.clone(), current.clone());
                    svc.update(cx, |s, cx| s.move_install(&app, &from, &choice.path, cx));
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    pub(super) fn epic_open_verify(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        if self.epic_busy_guard(cx, tr!("Verifying a game is not possible while an Epic game is being installed.")) {
            return;
        }
        let mut spec = Confirm::new(
            "epic-verify",
            tr!("Verify and Repair"),
            tr!("{title}\nDo you want to try to repair this game? This can take a long time.", title = game.title),
        );
        spec.confirm = Some(tr!("Verify").into());
        let app = game.app_name.clone();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |_, cx| {
                    let svc = crate::services::downloads(cx);
                    let app = app.clone();
                    svc.update(cx, |s, cx| s.verify(&app, cx));
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    pub(super) fn epic_open_update_prompt(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.epic_game(key) else { return };
        let mut spec = Confirm::new(
            "epic-update",
            tr!("Game Needs Update"),
            tr!("{title}\nThis game has an update, apply update now?", title = game.title),
        );
        spec.confirm = Some(tr!("Update").into());
        let weak = cx.entity().downgrade();
        let k = key.to_string();
        let k2 = key.to_string();
        let weak2 = weak.clone();
        spec.secondary = Some((
            tr!("Play anyway").into(),
            Rc::new(move |cx| {
                weak2
                    .update(cx, |this, cx| {
                        if let Some(g) = this.epic_game(&k2) {
                            if let Err(err) = this.runtime.play_skip_version_check(&g) {
                                this.show_toast(err, true, cx);
                            }
                        }
                    })
                    .ok();
            }),
        ));
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |_, cx| {
                    let k = k.clone();
                    weak.update(cx, |this, cx| this.epic_update(&k, cx)).ok();
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    /// `HERONITE_DEBUG_DIALOG=epic-install:<appName>` / `epic-import:<appName>` /
    /// `epic-uninstall:<appName>` / `epic-move:..` / `epic-verify:..` /
    /// `epic-update:..` / `epic-modify:..` / `epic-eos-install:..` / `epic-message`.
    pub(in crate::collection) fn epic_debug_dialog(&mut self, which: &str, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((kind, app)) =
            which.split_once(':').map(|(k, a)| (k, a.to_string())).or_else(|| which.starts_with("epic-").then(|| (which, String::new())))
        else {
            return false;
        };
        let key = format!("legendary_{app}");
        self.epic.pending = match kind {
            "epic-install" => Some(Request::Install { key, import: false }),
            "epic-import" => Some(Request::Install { key, import: true }),
            "epic-uninstall" => Some(Request::Uninstall(key)),
            "epic-move" => Some(Request::Move(key)),
            "epic-verify" => Some(Request::Verify(key)),
            "epic-update" => Some(Request::UpdatePrompt(key)),
            "epic-modify" => Some(Request::Modify(key)),
            "epic-eos-install" => Some(Request::EosInstall(app.clone())),
            "epic-message" => Some(Request::Message {
                title: "Cloud saves: Control".into(),
                text: "[cli] INFO: Cloud save sync for Control\n[cli] INFO: Remote save is newer, downloading 3 files\n[cli] INFO: Download finished".into(),
            }),
            _ => return false,
        };
        cx.notify();
        true
    }
}
