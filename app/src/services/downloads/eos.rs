//! EOS Overlay: installation status and the per-game state shown in the menus.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_epic::cmd::Legendary;
use heronite_epic::eos_overlay::{self, PrefixFolder};
use heronite_epic::ops::OpResult;
use heronite_i18n::tr;

use super::{Downloads, DownloadsEvent, EosGame, EosStatus};

/// The cached overlay state of one game and what it was read from.
pub(super) struct EosEntry {
    state: EosGame,
    /// `None` when the state was set here (toggle, debug) and not read yet.
    stamp: Option<EosStamp>,
    /// `user.reg` of the prefix the state was read for.
    registry: Option<PathBuf>,
    loading: bool,
}

impl EosEntry {
    pub(super) fn set(state: EosGame) -> Self {
        Self { state, stamp: None, registry: None, loading: false }
    }
}

/// Modification times of the files the per-game state depends on: the game
/// settings and the global defaults (which prefix) and the prefix registry
/// (written by legendary, Heroic, the first-launch setup or Wine itself).
/// When one changes, the next look at the menu reads the state again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EosStamp([Option<SystemTime>; 3]);

impl EosStamp {
    fn read(data_paths: &DataPaths, app_name: &str, registry: Option<&Path>) -> Self {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        Self([mtime(&data_paths.game_config(app_name)), mtime(&data_paths.store_config()), registry.and_then(mtime)])
    }
}

/// The overlay is on or off for every game at once (Windows), not per Wine
/// prefix (Linux).
pub fn global() -> bool {
    !cfg!(target_os = "linux")
}

impl Downloads {
    pub fn eos(&self) -> &EosStatus {
        &self.eos
    }

    /// Reads the overlay state; `check` also looks up the latest version
    /// (which may ask legendary, so it only runs when the page is open).
    pub fn eos_reload(&mut self, check: bool, cx: &mut Context<Self>) {
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let (status, latest, enabled) = cx
                .background_spawn(async move {
                    let status = eos_overlay::status(&data_paths);
                    let lg = Legendary::locate(&data_paths).ok();
                    let latest = match &lg {
                        Some(lg) if check && status.installed => eos_overlay::latest_version(&data_paths, lg),
                        _ => String::new(),
                    };
                    // Windows: one switch for every game (the Epic Games
                    // Launcher's own overlay counts too).
                    let enabled = lg.filter(|_| check && global()).map(|lg| eos_overlay::is_enabled(&lg, None));
                    (status, latest, enabled)
                })
                .await;
            this.update(cx, |this, cx| {
                if enabled.is_some() {
                    this.eos.enabled = enabled;
                }
                this.eos.installed = status.installed;
                this.eos.version = status.version.unwrap_or_default();
                if !status.installed {
                    this.eos.latest.clear();
                } else if check {
                    this.eos.latest = latest;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Check for updates": `legendary status`, then the latest version.
    pub fn eos_check_updates(&mut self, cx: &mut Context<Self>) {
        if self.eos.checking {
            return;
        }
        self.eos.checking = true;
        cx.notify();
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let latest = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&data_paths).ok()?;
                    eos_overlay::update_info(&data_paths, &lg);
                    Some(eos_overlay::latest_version(&data_paths, &lg))
                })
                .await;
            this.update(cx, |this, cx| {
                this.eos.checking = false;
                if let Some(latest) = latest {
                    this.eos.latest = latest;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Installs the overlay, or updates it when it is already installed.
    pub fn eos_install(&mut self, cx: &mut Context<Self>) {
        self.eos_install_then(None, cx);
    }

    fn eos_install_then(&mut self, enable_for: Option<String>, cx: &mut Context<Self>) {
        let label = if self.eos.installed { tr!("Updating EOS Overlay") } else { tr!("Installing EOS Overlay") };
        self.run_op_then(
            eos_overlay::APP_NAME,
            label,
            true,
            cx,
            |data_paths, lg, cancel, report| eos_overlay::install(data_paths, lg, cancel, &|p| report(Some(p))),
            move |this, r, cx| {
                this.eos_reload(true, cx);
                if r.ok {
                    if let Some(app) = enable_for {
                        this.eos_enable_game(&app, cx);
                    }
                }
            },
        );
    }

    /// Uninstalls the overlay (the caller has asked the user).
    pub fn eos_remove(&mut self, cx: &mut Context<Self>) {
        self.run_op_then(
            eos_overlay::APP_NAME,
            tr!("Removing EOS Overlay"),
            false,
            cx,
            |_, lg, _, _| eos_overlay::remove(lg),
            |this, r, cx| {
                if r.ok {
                    this.eos_games.clear();
                }
                this.eos_reload(true, cx);
            },
        );
    }

    /// Windows: turns the overlay on or off for every game.
    pub fn eos_set_global(&mut self, enable: bool, cx: &mut Context<Self>) {
        let label = if enable { tr!("Enabling EOS Overlay") } else { tr!("Disabling EOS Overlay") };
        self.run_op_then(
            eos_overlay::APP_NAME,
            label,
            false,
            cx,
            move |_, lg, _, _| if enable { eos_overlay::enable(lg, None) } else { eos_overlay::disable(lg, None) },
            move |this, r, cx| {
                if r.ok {
                    this.eos.enabled = Some(enable);
                }
                this.eos_reload(true, cx);
            },
        );
    }

    /// Whether the overlay is on for `app_name`'s prefix (once loaded).
    pub fn eos_game(&self, app_name: &str) -> Option<EosGame> {
        self.eos_games.get(app_name).map(|e| e.state)
    }

    /// Looks the state of a game up in the background. The context menu calls
    /// this on every frame it is open: the state is only read again when one
    /// of the files it depends on has changed since ([`EosStamp`]), and the
    /// old state stays on screen meanwhile.
    pub fn eos_load_game(&mut self, app_name: &str, cx: &mut Context<Self>) {
        if let Some(e) = self.eos_games.get_mut(app_name) {
            if e.loading || e.stamp.as_ref() == Some(&EosStamp::read(&self.data_paths, app_name, e.registry.as_deref())) {
                return;
            }
            e.loading = true;
        } else {
            self.eos_games.insert(app_name.to_string(), EosEntry { loading: true, ..EosEntry::set(EosGame::Loading) });
        }
        let (data_paths, app) = (self.data_paths.clone(), app_name.to_string());
        cx.spawn(async move |this, cx| {
            let (state, registry, stamp) = cx
                .background_spawn({
                    let app = app.clone();
                    async move {
                        let folder = eos_overlay::prefix_folder(&data_paths, &app);
                        let registry = match &folder {
                            PrefixFolder::Valid(prefix) => Some(Path::new(prefix).join("user.reg")),
                            _ => None,
                        };
                        // Before asking legendary: a change while it runs is seen next time.
                        let stamp = EosStamp::read(&data_paths, &app, registry.as_deref());
                        let state = match folder {
                            PrefixFolder::NotApplicable => EosGame::Unavailable,
                            PrefixFolder::Invalid => EosGame::Disabled,
                            PrefixFolder::Valid(prefix) => {
                                let enabled =
                                    Legendary::locate(&data_paths).map(|lg| eos_overlay::is_enabled(&lg, Some(&prefix))).unwrap_or(false);
                                if enabled {
                                    EosGame::Enabled
                                } else {
                                    EosGame::Disabled
                                }
                            }
                        };
                        (state, registry, stamp)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                // Cleared meanwhile (overlay removed): start over next time.
                if this.eos_games.get(&app).is_some_and(|e| e.loading) {
                    this.eos_games.insert(app, EosEntry { state, stamp: Some(stamp), registry, loading: false });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The "Enable / Disable EOS Overlay" item of a game.
    pub fn eos_toggle_game(&mut self, app_name: &str, cx: &mut Context<Self>) {
        match self.eos_game(app_name) {
            Some(EosGame::Enabled) => self.eos_set_game(app_name, false, cx),
            Some(EosGame::Disabled) => self.eos_enable_game(app_name, cx),
            _ => {}
        }
    }

    /// Enabling needs the overlay: when it is missing the Collection asks
    /// whether to install it first ([`DownloadsEvent::EosNotInstalled`]).
    fn eos_enable_game(&mut self, app_name: &str, cx: &mut Context<Self>) {
        // It may have been installed by Heronite since the page last looked.
        self.eos.installed = eos_overlay::status(&self.data_paths).installed;
        if !self.eos.installed {
            self.emit(cx, DownloadsEvent::EosNotInstalled { app_name: app_name.to_string() });
            return;
        }
        self.eos_set_game(app_name, true, cx);
    }

    /// Installs the overlay and enables it for the game (the answer to
    /// "The EOS Overlay is not installed. Do you want to install it now?").
    pub fn eos_install_and_enable(&mut self, app_name: &str, cx: &mut Context<Self>) {
        self.eos_install_then(Some(app_name.to_string()), cx);
    }

    fn eos_set_game(&mut self, app_name: &str, enable: bool, cx: &mut Context<Self>) {
        let app = app_name.to_string();
        let label = if enable { tr!("Enabling EOS Overlay") } else { tr!("Disabling EOS Overlay") };
        let app_for_work = app.clone();
        self.run_op_then(
            app_name,
            label,
            false,
            cx,
            move |data_paths, lg, _, _| {
                let prefix = match eos_overlay::prefix_folder(data_paths, &app_for_work) {
                    PrefixFolder::Valid(p) => Some(p),
                    PrefixFolder::NotApplicable => None,
                    // Nothing to enable / disable without a prefix (Heroic returns quietly).
                    PrefixFolder::Invalid if !enable => {
                        return OpResult { ok: true, aborted: false, warning: false, message: tr!("EOS Overlay disabled").into() }
                    }
                    PrefixFolder::Invalid => {
                        return OpResult {
                            ok: false,
                            aborted: false,
                            warning: false,
                            message: tr!("The Wine prefix of this game does not exist yet; start the game once first").into(),
                        }
                    }
                };
                if enable {
                    eos_overlay::enable(lg, prefix.as_deref())
                } else {
                    eos_overlay::disable(lg, prefix.as_deref())
                }
            },
            move |this, r, _| {
                if r.ok {
                    this.eos_games.insert(app, EosEntry::set(if enable { EosGame::Enabled } else { EosGame::Disabled }));
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{DataPaths, EosStamp};
    use std::time::{Duration, SystemTime};

    #[test]
    fn eos_stamp_changes_with_settings_and_prefix_registry() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        let reg = dir.path().join("pfx/user.reg");
        std::fs::create_dir_all(reg.parent().unwrap()).unwrap();
        std::fs::write(&reg, "WINE REGISTRY Version 2\n").unwrap();
        let before = EosStamp::read(&data_paths, "Game", Some(&reg));
        assert_eq!(before, EosStamp::read(&data_paths, "Game", Some(&reg)));

        let later = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options().write(true).open(&reg).unwrap().set_modified(later).unwrap();
        let touched = EosStamp::read(&data_paths, "Game", Some(&reg));
        assert_ne!(before, touched);

        std::fs::create_dir_all(data_paths.games_config_dir()).unwrap();
        std::fs::write(data_paths.game_config("Game"), "{}").unwrap();
        assert_ne!(touched, EosStamp::read(&data_paths, "Game", Some(&reg)));
    }
}
