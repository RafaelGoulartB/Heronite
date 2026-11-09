//! The emulators signed in to RetroAchievements with the account's token
//! (`heronite_achievements::emulators`): when the user signs in, at start
//! (an emulator installed since gets it too), when the options change and
//! after a game, for an emulator that was open the last time. Signing out
//! only clears the emulators that hold this account, so a login made by hand
//! inside an emulator with another account stays.

use gpui_kit::*;
use heronite_achievements::emulators::{self, ConfigPaths, Emulator, Login, State, Status};
use heronite_data::DataPaths;
use std::path::Path;

use super::Achievements;

fn with_configs(home: &Path, paths: &DataPaths, operation: impl FnOnce(&ConfigPaths) -> Vec<Status>) -> Vec<Status> {
    match ConfigPaths::load(home, paths) {
        Ok(configs) => operation(&configs),
        Err(message) => vec![Status { emulator: Emulator::RetroArch, config: None, state: State::Failed(message) }],
    }
}

impl Achievements {
    /// Each emulator's state at the last write or check.
    pub fn emulator_status(&self) -> &[Status] {
        &self.emulators
    }

    /// The RetroAchievements user the emulators get (signed in with a token).
    pub(super) fn signed_in_user(&self) -> Option<String> {
        self.accounts.retro_achievements.as_ref().filter(|a| !a.token.is_empty()).map(|a| a.username.clone())
    }

    /// Writes the login into every emulator, or clears it (from those that
    /// hold `before`, the user they may have been given earlier) when the
    /// account is gone or the option is off. Off the UI thread.
    pub(super) fn sync_emulators(&mut self, before: Option<String>, cx: &mut Context<Self>) {
        let account = self.accounts.retro_achievements.clone().filter(|a| !a.token.is_empty() && self.prefs.emulators);
        let (home, hardcore) = (self.home.clone(), self.prefs.hardcore);
        let paths = self.data_paths.clone();
        let clear = before.or_else(|| self.signed_in_user());
        if account.is_some() {
            // Signing in writes every file, the one that waited too.
            self.emulator_sign_out = None;
        }
        if account.is_none() && clear.is_none() {
            return;
        }
        let signing_out = account.is_none().then(|| clear.clone().unwrap_or_default());
        cx.spawn(async move |this, cx| {
            let statuses = cx
                .background_spawn(async move {
                    with_configs(&home, &paths, |configs| match &account {
                        Some(a) => emulators::apply(configs, Some(&Login { username: &a.username, token: &a.token, hardcore })),
                        None => {
                            let user = clear.unwrap_or_default();
                            emulators::sign_out(configs, &user)
                        }
                    })
                })
                .await;
            for s in &statuses {
                if let State::Failed(e) = &s.state {
                    log::warn!("RetroAchievements login of {}: {e}", s.emulator.name());
                }
            }
            this.update(cx, |this, cx| {
                if let Some(user) = signing_out {
                    this.emulator_sign_out = statuses.iter().any(|s| s.state == State::Running).then_some(user);
                }
                this.emulators = statuses;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads what the emulators hold now (Settings shows it).
    pub fn check_emulators(&mut self, cx: &mut Context<Self>) {
        let (home, user) = (self.home.clone(), self.signed_in_user());
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let statuses = cx
                .background_spawn(async move { with_configs(&home, &paths, |configs| emulators::status(configs, user.as_deref())) })
                .await;
            this.update(cx, |this, cx| {
                this.emulators = statuses;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// An emulator that was open at the last write gets the login (or, after
    /// a sign-out, loses it) once a game ends (it has closed by then, usually).
    pub(super) fn retry_running_emulators(&mut self, cx: &mut Context<Self>) {
        if self.emulators.iter().any(|s| s.state == State::Running) {
            self.sync_emulators(self.emulator_sign_out.clone(), cx);
        }
    }
}
