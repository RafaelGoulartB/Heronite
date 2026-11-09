//! The Steam account of the Steam library service: the sign-in with a QR
//! code (`heronite_steam::account::auth`) and signing out. Reading the
//! library from it is the service's own read ([`SteamLibrary::sync`]).

use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_steam::account::auth::{self, Poll, QrSession};
use heronite_steam::account::http::Net;

use super::SteamLibrary;

/// How many failed checks in a row end a sign-in (the network is gone).
const POLL_FAILURES: u32 = 3;

/// The account, as the Steam page shows it.
#[derive(Default)]
pub struct AccountState {
    /// The signed-in account's name; `None`: signed out.
    pub name: Option<String>,
    /// A sign-in under way.
    pub qr: Option<QrState>,
    /// A sign-in is being started or finished, or a sign-out is under way.
    pub busy: bool,
    /// The last sign-in or sign-out, when the user is to see it.
    pub last: Option<Result<String, String>>,
    /// When the library was last read from it, this session.
    read_at: Option<Instant>,
    /// Numbers the sign-ins, so the checks of an old one stop.
    next_id: u64,
}

/// A QR code waiting to be scanned and approved.
#[derive(Clone)]
pub struct QrState {
    id: u64,
    session: QrSession,
    /// Scanned: the phone asks the user to approve.
    pub scanned: bool,
    /// The code expired (or was refused): a new one is needed.
    pub ended: bool,
    /// Approved on the phone: the sign-in is being saved.
    pub approved: bool,
    failures: u32,
}

impl QrState {
    /// What the QR code holds.
    pub fn url(&self) -> &str {
        &self.session.challenge_url
    }
}

impl AccountState {
    /// The account saved here. Blocking (one small file).
    pub fn load(paths: &DataPaths) -> Self {
        Self { name: heronite_steam::account::load(paths).map(|a| a.label().to_string()), ..Default::default() }
    }

    pub fn signed_in(&self) -> bool {
        self.name.is_some()
    }

    /// Whether a quiet read from the account is due.
    pub(super) fn due(&self, every: Duration) -> bool {
        self.signed_in() && self.read_at.is_none_or(|t| t.elapsed() >= every)
    }

    pub(super) fn read_now(&mut self) {
        self.read_at = Some(Instant::now());
    }

    pub(super) fn signed_out(&mut self) {
        self.name = None;
        self.read_at = None;
    }

    /// `HERONITE_DEBUG_STEAM_ACCOUNT=qr|scanned|expired|signed` shows that
    /// state of the card, without Steam (for screenshots).
    pub(super) fn debug(&mut self) {
        let Some(kind) = crate::env::value("HERONITE_DEBUG_STEAM_ACCOUNT") else { return };
        if kind == "signed" {
            self.name = Some("gaben".into());
            return;
        }
        let session = QrSession {
            client_id: "0".into(),
            request_id: String::new(),
            challenge_url: "https://s.team/q/1/8406597215213412810".into(),
            interval: Duration::from_secs(3600),
        };
        // An id no sign-in uses: nothing polls it.
        self.qr =
            Some(QrState { id: u64::MAX, session, scanned: kind == "scanned", ended: kind == "expired", approved: false, failures: 0 });
    }
}

/// How the sign-in shows in the Steam app's list of devices.
fn device_name() -> String {
    format!("Heronite ({})", std::env::consts::OS)
}

impl SteamLibrary {
    /// Starts a sign-in: a QR code to scan with the Steam app.
    pub fn begin_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.account.busy || self.account.signed_in() {
            return;
        }
        self.account.busy = true;
        self.account.last = None;
        self.account.qr = None;
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { auth::begin(&Net::default(), &device_name()) }).await;
            this.update(cx, |this, cx| {
                this.account.busy = false;
                match res {
                    Ok(session) => {
                        this.account.next_id += 1;
                        let id = this.account.next_id;
                        this.account.qr = Some(QrState { id, session, scanned: false, ended: false, approved: false, failures: 0 });
                        this.poll_sign_in(id, cx);
                    }
                    Err(e) => {
                        log::warn!("Steam sign-in: {e:#}");
                        this.account.last = Some(Err(tr!("Could not start the sign-in: {error}", error = format!("{e:#}"))));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Stops showing the QR code; the sign-in is dropped.
    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        self.account.qr = None;
        cx.notify();
    }

    /// The sign-in `id`, still current: its session and how often to ask.
    fn current(&self, id: u64) -> Option<QrSession> {
        self.account.qr.as_ref().filter(|q| q.id == id && !q.ended && !q.approved).map(|q| q.session.clone())
    }

    /// Asks Steam, every few seconds, whether the sign-in `id` was approved.
    fn poll_sign_in(&mut self, id: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            let Some(session) = this.read_with(cx, |this, _| this.current(id)).ok().flatten() else { return };
            cx.background_executor().timer(session.interval).await;
            // Cancelled, or replaced by a new code, while waiting.
            if this.read_with(cx, |this, _| this.current(id).is_none()).unwrap_or(true) {
                return;
            }
            let res = cx
                .background_spawn(async move {
                    let mut session = session;
                    let poll = auth::poll(&Net::default(), &mut session);
                    (session, poll)
                })
                .await;
            let done = this.update(cx, |this, cx| this.on_poll(id, res, cx)).unwrap_or(true);
            if done {
                return;
            }
        })
        .detach();
    }

    /// Takes a check's answer; whether the checks are over.
    fn on_poll(&mut self, id: u64, (session, poll): (QrSession, anyhow::Result<Poll>), cx: &mut Context<Self>) -> bool {
        let Some(qr) = self.account.qr.as_mut().filter(|q| q.id == id) else { return true };
        qr.session = session;
        let done = match poll {
            Ok(Poll::Waiting { scanned }) => {
                qr.scanned = scanned;
                qr.failures = 0;
                false
            }
            Ok(Poll::Ended) => {
                qr.ended = true;
                true
            }
            Ok(Poll::Approved(tokens)) => {
                qr.approved = true;
                self.finish_sign_in(tokens, cx);
                return true;
            }
            Err(e) => {
                log::warn!("Steam sign-in: {e:#}");
                qr.failures += 1;
                if qr.failures >= POLL_FAILURES {
                    qr.ended = true;
                    self.account.last = Some(Err(tr!("Lost touch with Steam during the sign-in: {error}", error = format!("{e:#}"))));
                }
                qr.ended
            }
        };
        cx.notify();
        done
    }

    /// Saves the approved sign-in and reads the library from it.
    fn finish_sign_in(&mut self, tokens: auth::Tokens, cx: &mut Context<Self>) {
        self.account.busy = true;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_steam::account::finish(&paths, &Net::default(), tokens) }).await;
            this.update(cx, |this, cx| {
                this.account.busy = false;
                this.account.qr = None;
                match res {
                    Ok(account) => {
                        log::info!("Signed in to Steam");
                        this.account.name = Some(account.label().to_string());
                        this.account.last = Some(Ok(tr!(
                            "Signed in. Heronite reads your library from your Steam account when Steam on this PC cannot be read."
                        )
                        .to_string()));
                        this.sync(cx);
                    }
                    Err(e) => {
                        log::warn!("Steam sign-in: {e:#}");
                        this.account.last = Some(Err(tr!("Could not finish the sign-in: {error}", error = format!("{e:#}"))));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Signs out: ends the sign-in on Steam too, then forgets it here. What
    /// it brought stays in the Collection.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        // A read under way may renew the tokens and save them again.
        if self.syncing || self.account.busy {
            return;
        }
        self.account.busy = true;
        self.account.last = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_steam::account::sign_out_on_steam(&paths, &Net::default()) }).await;
            this.update(cx, |this, cx| {
                this.account.busy = false;
                match res {
                    Ok(ended) => {
                        log::info!("Signed out of Steam");
                        this.account.signed_out();
                        this.account.last = Some(Ok(if ended {
                            tr!("Signed out. Steam no longer accepts this sign-in, and Heronite left your account's authorized devices.")
                        } else {
                            // Steam out of reach or refusing: it still lists the sign-in.
                            tr!("Signed out. To also remove Heronite from your account, go to store.steampowered.com/account/authorizeddevices.")
                        }
                        .to_string()));
                    }
                    Err(e) => {
                        log::warn!("Steam sign-out: {e:#}");
                        this.account.last = Some(Err(format!("{e:#}")));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
