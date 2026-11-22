//! The EA account (optional, `heronite_clients::ea_account`): signed in on
//! EA's page in the login window, it brings the games the account owns that
//! are not installed into the Collection, as not installed. It is read after
//! a sync of the EA app's games, at most every six hours by itself, right
//! after signing in and when the user asks. The EA app must be set up: its
//! games install through it. A read that fails leaves the Collection as it
//! is; the installed games keep coming from the EA app's files as before.
//!
//! `HERONITE_DEBUG_EA_ACCOUNT=signed|out|error` shows the EA page's account
//! card in that state, without EA (for screenshots).

use futures::future::{FutureExt, Shared};
use gpui_kit::*;
use heronite_clients::collection::Report;
use heronite_clients::ea_account::{self, http::Net, Error, Session};
use heronite_clients::{owned, Client};
use heronite_data::clients::{ClientPrefs, LastSync};
use heronite_data::json_store;
use heronite_i18n::{tr, trn};

use super::{ea_vault, Clients, ClientsEvent};

/// The account is read at most this often by itself, whether the last read
/// worked or not.
const EVERY: chrono::TimeDelta = chrono::TimeDelta::hours(6);

/// The EA account, as the EA page shows it.
#[derive(Clone, Default)]
pub struct EaAccount {
    /// Signed in: the name EA shows (`""` when it gave none).
    pub name: Option<String>,
    pub signing_in: bool,
    pub reading: bool,
    /// The last read or sign-in, when the user is to see it.
    pub last: Option<Result<String, String>>,
    /// The last read that worked.
    pub owned: Option<LastSync>,
    /// When the last read of this run started (one that failed counts too:
    /// EA, or a locked keyring, is not asked again for six hours).
    tried: Option<std::time::Instant>,
    /// A renewed session being kept: a sign-out waits for it, then clears.
    saving: Option<Shared<Task<()>>>,
    /// The session, once taken from where it is kept.
    session: Option<Session>,
    debug: bool,
}

impl EaAccount {
    /// What `local_library/clients.json` says (no secret is read).
    pub(super) fn from_prefs(prefs: &ClientPrefs) -> Self {
        let mut this =
            Self { name: prefs.accounts.get(&Client::Ea).cloned(), owned: prefs.owned.get(&Client::Ea).cloned(), ..Default::default() };
        if let Some(state) = crate::env::value("HERONITE_DEBUG_EA_ACCOUNT") {
            this.debug = true;
            this.name = (state != "out").then(|| "Heronite Player".into());
            this.owned = this.name.as_ref().map(|_| LastSync { at: chrono::Utc::now().to_rfc3339(), games: 42 });
            if state == "error" {
                this.last = Some(Err(not_read(&Error::Changed("Cannot query field \"ownedGameProducts\"".into()))));
            }
        }
        this
    }

    /// Connected, or a session is held this run.
    pub(super) fn connected(&self) -> bool {
        self.name.is_some() || self.session.is_some()
    }

    /// A read or a sign-in runs: the EA app is not removed meanwhile.
    pub fn busy(&self) -> bool {
        self.reading || self.signing_in
    }

    fn due(&self) -> bool {
        let every = EVERY.to_std().expect("positive");
        self.tried.is_none_or(|t| t.elapsed() >= every)
            && self
                .owned
                .as_ref()
                .and_then(|l| chrono::DateTime::parse_from_rfc3339(&l.at).ok())
                .is_none_or(|t| chrono::Utc::now() - t.with_timezone(&chrono::Utc) >= EVERY)
    }
}

/// Why "Remove from Heronite" waits.
pub(super) fn wait_note() -> String {
    tr!("Wait for your EA account to finish, then try again.").to_string()
}

/// What the page says when a read failed (the details are in the log).
fn not_read(e: &Error) -> String {
    match e {
        Error::Changed(_) => tr!(
            "EA changed how its service answers, so your EA library cannot be read for now; a Heronite update may be needed. Your installed EA games are not affected."
        )
        .to_string(),
        e => tr!("Could not read your EA library: {error}. Your installed EA games are not affected.", error = e.to_string()),
    }
}

/// Records in `clients.json` that the account is signed in (`Some`, its
/// name) or not.
fn save_signed_in(paths: &heronite_data::DataPaths, name: Option<String>) -> anyhow::Result<()> {
    json_store::update_if_changed::<ClientPrefs, _>(&paths.clients(), |p| {
        match name {
            Some(n) => p.accounts.insert(Client::Ea, n),
            None => p.accounts.remove(&Client::Ea),
        };
        Ok(())
    })
}

impl Clients {
    /// The EA app is not set up, or is being removed: its account is not
    /// read or connected then (its games would come back).
    pub fn ea_app_leaving(&self) -> bool {
        let st = self.state(Client::Ea);
        !st.installed() || st.activity == super::Activity::Forgetting
    }

    /// The EA app's note asking to wait (a removal refused meanwhile) is
    /// over once the account read or sign-in ends.
    fn ea_done_waiting(&mut self) {
        let st = self.state_mut(Client::Ea);
        if st.notice.as_ref().is_some_and(|n| n.text == wait_note()) {
            st.notice = None;
        }
    }

    /// The login window printed `output` (the EA session's cookies): checks
    /// it with EA, keeps it, and reads the account.
    pub fn ea_signed_in(&mut self, output: String, cx: &mut Context<Self>) {
        if self.ea.signing_in || self.ea.debug {
            return;
        }
        // The EA app went (or is going) away while the login window was open.
        // (Its card is gone with it: nothing to show; the session is dropped.)
        if self.ea_app_leaving() {
            log::info!("EA account: the EA app went away during the sign-in; not connected");
            return;
        }
        self.ea.signing_in = true;
        self.ea.last = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { ea_account::sign_in(&Net::default(), &output) }).await;
            let session = match res {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("EA account: the sign-in did not work: {e}");
                    let text = match e {
                        Error::SignedOut => tr!("EA did not finish the sign-in. Try again.").to_string(),
                        Error::Changed(_) => not_read(&e),
                        e => tr!("Could not sign in to EA: {error}", error = e.to_string()),
                    };
                    this.update(cx, |this, cx| {
                        this.ea.signing_in = false;
                        this.ea_done_waiting();
                        this.ea.last = Some(Err(text));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let Ok(stored) = this.update(cx, |this, cx| ea_vault::store(&this.paths, &session, cx)) else { return };
            let stored = stored.await;
            let name = session.name.clone().unwrap_or_default();
            let saved = match stored {
                Ok(()) => {
                    let name = name.clone();
                    cx.background_spawn(async move { save_signed_in(&paths, Some(name)).map_err(|e| format!("{e:#}")) }).await
                }
                Err(e) => Err(e),
            };
            this.update(cx, |this, cx| {
                this.ea.signing_in = false;
                this.ea_done_waiting();
                if saved.is_err() {
                    // Nothing says it is signed in: nothing is kept either.
                    ea_vault::clear(&this.paths, cx).detach();
                }
                match saved {
                    Ok(()) => {
                        log::info!("Signed in to the EA account");
                        this.ea.name = Some(name);
                        this.ea.session = Some(session);
                        this.read_ea(true, cx);
                    }
                    Err(e) => {
                        log::warn!("EA account: the sign-in could not be kept: {e}");
                        this.ea.last = Some(Err(tr!("Could not keep the EA sign-in: {error}", error = e)));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Reads the account's games into the Collection (`asked`: now, else
    /// when the last read is six hours old). Only with the EA app set up.
    pub fn read_ea(&mut self, asked: bool, cx: &mut Context<Self>) {
        let ea = &self.ea;
        if ea.debug || ea.name.is_none() || ea.busy() || self.ea_app_leaving() || !(asked || ea.due()) {
            return;
        }
        self.ea.reading = true;
        self.ea.tried = Some(std::time::Instant::now());
        let paths = self.paths.clone();
        let cached = self.ea.session.clone();
        let load = cached.is_none().then(|| ea_vault::load(&self.paths, cx));
        cx.spawn(async move |this, cx| {
            let session = match load {
                None => Ok(cached),
                Some(task) => task.await,
            };
            let mut session = match session {
                Ok(Some(s)) => s,
                Ok(None) => {
                    // Nothing kept (another PC's, or the keyring lost it).
                    this.update(cx, |this, cx| this.ea_read_done(asked, Err(Error::SignedOut), None, cx)).ok();
                    return;
                }
                Err(e) => {
                    let e = Error::Failed(anyhow::anyhow!("the keyring: {e}"));
                    this.update(cx, |this, cx| this.ea_read_done(asked, Err(e), None, cx)).ok();
                    return;
                }
            };
            let before = session.clone();
            let res = cx
                .background_spawn(async move {
                    let res = ea_account::read(&Net::default(), &mut session).and_then(|games| {
                        let n = games.len();
                        let report = owned::sync(&paths, Client::Ea, &games).map_err(Error::Failed)?;
                        Ok((report, n, ClientPrefs::load(&paths).owned.get(&Client::Ea).cloned()))
                    });
                    (res, session)
                })
                .await;
            let (res, session) = res;
            // EA renewed the session: the new one is kept.
            let renewed = (session != before && !session.cookies.is_empty()).then_some(session);
            this.update(cx, |this, cx| this.ea_read_done(asked, res, renewed, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn ea_read_done(
        &mut self,
        asked: bool,
        res: Result<(Report, usize, Option<LastSync>), Error>,
        renewed: Option<Session>,
        cx: &mut Context<Self>,
    ) {
        self.ea.reading = false;
        self.ea_done_waiting();
        // A session EA ended, or the user dropped meanwhile, is not kept again.
        if let Some(session) = renewed.filter(|_| self.ea.name.is_some() && !matches!(res, Err(Error::SignedOut))) {
            let before = self.ea.saving.take();
            let (paths, kept) = (self.paths.clone(), session.clone());
            let saving = cx
                .spawn(async move |_, cx| {
                    // In order: the write starts once an earlier one ended.
                    if let Some(before) = before {
                        before.await;
                    }
                    let store = cx.update(|cx| ea_vault::store(&paths, &kept, cx));
                    if let Err(e) = store.await {
                        log::warn!("EA account: keeping the renewed sign-in: {e}");
                    }
                })
                .shared();
            cx.background_spawn(saving.clone()).detach();
            self.ea.saving = Some(saving);
            self.ea.session = Some(session);
        }
        let outcome = match res {
            Ok((report, n, last)) => {
                self.ea.owned = last;
                if report.added > 0 {
                    cx.emit(ClientsEvent::Synced(Client::Ea, report));
                }
                Ok(trn!(n, "Read {n} game from your EA account.", "Read {n} games from your EA account."))
            }
            Err(Error::SignedOut) => {
                log::info!("EA account: signed out by EA");
                self.forget_ea(cx);
                Err(tr!("EA ended the connection. Connect your EA account again to see the games you own that are not installed.")
                    .to_string())
            }
            Err(e) => {
                log::warn!("EA account: {e}");
                Err(not_read(&e))
            }
        };
        if asked || outcome.is_err() {
            self.ea.last = Some(outcome);
        } else if matches!(self.ea.last, Some(Err(_))) {
            // A quiet read that worked: an earlier failure is over.
            self.ea.last = None;
        }
        cx.notify();
    }

    /// Signs out of the EA account. The games it brought stay in the
    /// Collection.
    pub fn ea_sign_out(&mut self, cx: &mut Context<Self>) {
        // A read under way may renew the session and keep it again.
        if self.ea.reading || self.ea.signing_in || self.ea.debug {
            return;
        }
        self.forget_ea(cx);
        self.ea.last = Some(Ok(tr!("Disconnected. The games your EA account brought stay in your Collection.").to_string()));
        cx.notify();
    }

    /// Drops the session, where it is kept too.
    pub(super) fn forget_ea(&mut self, cx: &mut Context<Self>) {
        self.ea.name = None;
        self.ea.session = None;
        let saving = self.ea.saving.take();
        let paths = self.paths.clone();
        cx.spawn(async move |_, cx| {
            // After a renewed session is kept, or it would come back.
            if let Some(saving) = saving {
                saving.await;
            }
            let clear = cx.update(|cx| ea_vault::clear(&paths, cx));
            if let Err(e) = clear.await {
                log::warn!("EA account: forgetting the sign-in: {e}");
            }
            if let Err(e) = cx.background_spawn(async move { save_signed_in(&paths, None) }).await {
                log::warn!("EA account: {e:#}");
            }
        })
        .detach();
    }
}
