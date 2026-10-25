//! The emulators signed in to RetroAchievements with the account's token
//! (`heronite_achievements::emulators`): when the user signs in, at start
//! (an emulator installed since gets it too), when the options change and
//! after a game, for an emulator that was open the last time. Signing out
//! only clears the emulators that hold this account, so a login made by hand
//! inside an emulator with another account stays.

use gpui_kit::*;
use heronite_achievements::emulators::{self, Login, State, Status};

use super::Achievements;

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
        let clear = before.or_else(|| self.signed_in_user());
        if account.is_none() && clear.is_none() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let statuses = cx
                .background_spawn(async move {
                    match &account {
                        Some(a) => emulators::apply(&home, Some(&Login { username: &a.username, token: &a.token, hardcore })),
                        None => {
                            let user = clear.unwrap_or_default();
                            let ours: Vec<_> = emulators::status(&home, Some(&user))
                                .into_iter()
                                .filter(|s| s.state == State::SignedIn)
                                .map(|s| s.emulator)
                                .collect();
                            if !ours.is_empty() {
                                emulators::apply_to(&home, &ours, None);
                            }
                            emulators::status(&home, None)
                        }
                    }
                })
                .await;
            for s in &statuses {
                if let State::Failed(e) = &s.state {
                    log::warn!("RetroAchievements login of {}: {e}", s.emulator.name());
                }
            }
            this.update(cx, |this, cx| {
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
        cx.spawn(async move |this, cx| {
            let statuses = cx.background_spawn(async move { emulators::status(&home, user.as_deref()) }).await;
            this.update(cx, |this, cx| {
                this.emulators = statuses;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// An emulator that was open at the last write gets the login once a
    /// game ends (it has closed by then, usually).
    pub(super) fn retry_running_emulators(&mut self, cx: &mut Context<Self>) {
        if self.emulators.iter().any(|s| s.state == State::Running) {
            self.sync_emulators(None, cx);
        }
    }
}
