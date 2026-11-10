//! What a script's `ctx` argument offers, and the state of the call it is
//! for (time left, requests and bytes used, why it was stopped).
//!
//! Every host function runs on the script's thread and blocks it; a
//! request is bounded by what is left of the call's time.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use heronite_metadata::util::normalize_title;
use heronite_metadata::Http;
use rquickjs::function::Opt;
use rquickjs::promise::PromiseState;
use rquickjs::{Ctx, Exception, Function, Object, Value};

use super::{convert, describe, html, request, CallError, Limit};
use crate::limits::Limits;
use crate::net::{HostRule, RateLimiter};

/// What a session of one extension may use; cheap to clone.
#[derive(Clone)]
pub struct HostEnv {
    /// The extension's id (log lines, thread names).
    pub id: String,
    pub http: Http,
    /// The manifest's `[network] hosts`.
    pub hosts: Arc<Vec<HostRule>>,
    /// Shared by every call of the extension.
    pub rate: Arc<RateLimiter>,
    pub limits: Limits,
    /// Its own options' values (`ctx.settings.get`).
    pub settings: Arc<serde_json::Map<String, serde_json::Value>>,
    /// Heroic language code of the interface (`pt-BR`), `ctx.language`.
    pub language: String,
}

pub(super) struct Host {
    pub(super) env: HostEnv,
    deadline: Cell<Instant>,
    /// Set with `abort`: the interrupt handler ends the call.
    stopped: Cell<bool>,
    abort: RefCell<Option<CallError>>,
    unreachable: RefCell<Option<String>>,
    pub(super) requests: Cell<u32>,
    pub(super) bytes: Cell<u64>,
    logs: Cell<u32>,
    pub(super) docs: RefCell<html::Docs>,
}

impl Host {
    pub(super) fn new(env: HostEnv) -> Self {
        Self {
            deadline: Cell::new(Instant::now() + env.limits.time),
            env,
            stopped: Cell::new(false),
            abort: RefCell::new(None),
            unreachable: RefCell::new(None),
            requests: Cell::new(0),
            bytes: Cell::new(0),
            logs: Cell::new(0),
            docs: RefCell::new(html::Docs::default()),
        }
    }

    /// A new call: its own time and budgets.
    pub(super) fn begin(&self) {
        self.deadline.set(Instant::now() + self.env.limits.time);
        self.stopped.set(false);
        self.abort.replace(None);
        self.unreachable.replace(None);
        self.requests.set(0);
        self.bytes.set(0);
        self.logs.set(0);
        self.docs.replace(html::Docs::default());
    }

    pub(super) fn deadline(&self) -> Instant {
        self.deadline.get()
    }

    /// The interrupt handler: whether the running script must stop.
    pub(super) fn should_stop(&self) -> bool {
        self.stopped.get() || Instant::now() >= self.deadline.get()
    }

    /// Ends the call with `e` (the first reason wins); the script may catch
    /// the exception thrown with it, but the call fails anyway.
    pub(super) fn abort(&self, e: CallError) {
        let mut abort = self.abort.borrow_mut();
        if abort.is_none() {
            *abort = Some(e);
        }
        self.stopped.set(true);
    }

    pub(super) fn aborted(&self) -> Option<CallError> {
        self.abort.borrow().clone()
    }

    pub(super) fn set_unreachable(&self, why: String) {
        self.unreachable.replace(Some(why));
    }

    pub(super) fn unreachable(&self) -> Option<String> {
        self.unreachable.borrow().clone()
    }

    /// What ended the call, given the engine's error `e`.
    pub(super) fn error(&self, ctx: &Ctx<'_>, e: rquickjs::Error) -> CallError {
        let text = describe(ctx, e);
        if let Some(a) = self.aborted() {
            return a;
        }
        if Instant::now() >= self.deadline.get() || text.contains("interrupted") {
            return CallError::Limit(Limit::Time, format!("ran longer than {:?}", self.env.limits.time));
        }
        // Out of memory, QuickJS cannot even make the error and throws
        // `null` (so a script's own `throw null` reads as this too).
        if text.contains("out of memory") || text.contains("Allocation failed") || text == "null" || text == "undefined" {
            return CallError::Limit(Limit::Memory, format!("used more than {} bytes of memory", self.env.limits.memory));
        }
        if text.contains("stack overflow") || text.contains("call stack size exceeded") {
            return CallError::Limit(Limit::Stack, "recursion too deep".into());
        }
        match self.unreachable() {
            Some(why) => CallError::Unreachable(why),
            None => CallError::Script(text),
        }
    }

    /// `value`, or what it settles to when it is a promise: runs the job
    /// queue until it settles. A promise left pending with no job to run
    /// can never settle (there are no timers).
    pub(super) fn settle<'js>(&self, ctx: &Ctx<'js>, value: Value<'js>) -> Result<Value<'js>, CallError> {
        let Some(promise) = value.as_promise().cloned() else { return Ok(value) };
        loop {
            match promise.state() {
                PromiseState::Resolved | PromiseState::Rejected => {
                    return match promise.result::<Value>() {
                        Some(Ok(v)) => Ok(v),
                        Some(Err(e)) => Err(self.error(ctx, e)),
                        None => Err(CallError::Script("the promise did not settle".into())),
                    };
                }
                PromiseState::Pending => {
                    if let Some(a) = self.aborted() {
                        return Err(a);
                    }
                    if self.should_stop() {
                        return Err(CallError::Limit(Limit::Time, format!("ran longer than {:?}", self.env.limits.time)));
                    }
                    if !ctx.execute_pending_job() {
                        return Err(CallError::Limit(Limit::NeverSettled, "it waits for something that never comes".into()));
                    }
                }
            }
        }
    }

    fn log(&self, message: &str) {
        let n = self.logs.get() + 1;
        self.logs.set(n);
        if n > self.env.limits.log_lines {
            return;
        }
        let line: String = message.chars().filter(|c| !c.is_control()).take(500).collect();
        log::info!("[extension {}] {line}", self.env.id);
        if n == self.env.limits.log_lines {
            log::info!("[extension {}] more lines of this call are left out", self.env.id);
        }
    }
}

/// `ctx`, the second argument of `find` and `fetch` and the only one of
/// `test`.
pub(super) fn api<'js>(ctx: &Ctx<'js>, host: &Rc<Host>) -> rquickjs::Result<Object<'js>> {
    let api = Object::new(ctx.clone())?;

    let http = Object::new(ctx.clone())?;
    for post in [false, true] {
        let h = host.clone();
        let f = Function::new(ctx.clone(), move |ctx: Ctx<'js>, url: String, opts: Opt<Object<'js>>| {
            request::send(&ctx, &h, post, &url, opts.0)
        })?;
        http.set(if post { "post" } else { "get" }, f)?;
    }
    api.set("http", http)?;

    let html_api = Object::new(ctx.clone())?;
    let h = host.clone();
    html_api.set("parse", Function::new(ctx.clone(), move |ctx: Ctx<'js>, text: String| html::parse(&ctx, &h, &text))?)?;
    api.set("html", html_api)?;

    let settings = Object::new(ctx.clone())?;
    let h = host.clone();
    let get = Function::new(ctx.clone(), move |ctx: Ctx<'js>, key: String| -> rquickjs::Result<Value<'js>> {
        match h.env.settings.get(&key) {
            Some(v) => convert::from_json(&ctx, v),
            None => Ok(Value::new_undefined(ctx)),
        }
    })?;
    settings.set("get", get)?;
    api.set("settings", settings)?;

    let date = Object::new(ctx.clone())?;
    date.set("parse", Function::new(ctx.clone(), |text: String| crate::record::parse_date(&text))?)?;
    api.set("date", date)?;

    let h = host.clone();
    api.set(
        "log",
        Function::new(ctx.clone(), move |message: rquickjs::Coerced<String>| {
            h.log(&message.0);
        })?,
    )?;
    api.set("normalize", Function::new(ctx.clone(), |title: String| normalize_title(&title))?)?;
    api.set("language", host.env.language.clone())?;
    Ok(api)
}

/// Throws a `TypeError` with `message`.
pub(super) fn type_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    Exception::throw_type(ctx, message)
}
