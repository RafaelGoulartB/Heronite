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

use super::{convert, describe, html, request, CallError, Limit, Watch};
use crate::limits::Limits;
use crate::net::{scrub, HostRule, RateLimiter};

/// What `ctx.unreachable()` says without a message.
const DEFAULT_UNREACHABLE: &str = "the service cannot be asked now";
/// Errors `ctx.unreachable` keeps per call; one made past them is still
/// returned, but ends the call as a failure.
const MAX_DECLARED: usize = 16;

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
    /// The deadline as the session's caller sees it.
    watch: Arc<Watch>,
    /// Set with `abort`: the interrupt handler ends the call.
    stopped: Cell<bool>,
    abort: RefCell<Option<CallError>>,
    /// The last request that got no answer: why, and the message of the
    /// error it threw to the script.
    unreachable: RefCell<Option<(String, String)>>,
    /// The messages of the errors `ctx.unreachable` made in this call.
    declared: RefCell<Vec<String>>,
    pub(super) requests: Cell<u32>,
    pub(super) bytes: Cell<u64>,
    logs: Cell<u32>,
    pub(super) docs: RefCell<html::Docs>,
}

impl Host {
    pub(super) fn new(env: HostEnv, watch: Arc<Watch>) -> Self {
        Self {
            deadline: Cell::new(Instant::now() + env.limits.time),
            watch,
            env,
            stopped: Cell::new(false),
            abort: RefCell::new(None),
            unreachable: RefCell::new(None),
            declared: RefCell::new(Vec::new()),
            requests: Cell::new(0),
            bytes: Cell::new(0),
            logs: Cell::new(0),
            docs: RefCell::new(html::Docs::default()),
        }
    }

    /// A new call: its own time and budgets.
    pub(super) fn begin(&self) {
        self.deadline.set(Instant::now() + self.env.limits.time);
        self.watch.set_deadline(self.deadline.get());
        self.stopped.set(false);
        self.abort.replace(None);
        self.unreachable.replace(None);
        self.declared.replace(Vec::new());
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

    /// A request got no answer for `why`; it threw an error with `message`.
    pub(super) fn set_unreachable(&self, why: String, message: String) {
        self.unreachable.replace(Some((why, message)));
    }

    pub(super) fn unreachable(&self) -> Option<String> {
        self.unreachable.borrow().as_ref().map(|(why, _)| why.clone())
    }

    /// `ctx.unreachable(message)`: the script says the service cannot be
    /// asked now (busy, down). The error it gets back ends the call as
    /// unreachable when thrown, like a request that got no answer.
    fn declare_unreachable(&self, message: &str) -> String {
        let line: String =
            message.chars().map(|c| if c.is_control() || crate::manifest::is_format_char(c) { ' ' } else { c }).take(300).collect();
        let line = match line.split_whitespace().collect::<Vec<_>>().join(" ") {
            l if l.is_empty() => DEFAULT_UNREACHABLE.to_owned(),
            l => l,
        };
        let mut declared = self.declared.borrow_mut();
        if declared.len() < MAX_DECLARED && !declared.contains(&line) {
            declared.push(line.clone());
        }
        line
    }

    /// What ended the call, given the engine's error `e`. It is unreachable
    /// only when the error is the one a request threw for it, even if the
    /// wait for that request used up the call's time: an error of the
    /// script's own after it, or code run until the time limit, is the
    /// script's failure.
    pub(super) fn error(&self, ctx: &Ctx<'_>, e: rquickjs::Error) -> CallError {
        let text = describe(ctx, e);
        if let Some(a) = self.aborted() {
            return a;
        }
        let interrupted = text.contains("interrupted");
        let first = text.lines().next().unwrap_or("");
        if let Some((why, message)) = self.unreachable.borrow().as_ref() {
            if !interrupted && first == format!("Error: {message}") {
                return CallError::Unreachable(why.clone());
            }
        }
        if let Some(said) = self.declared.borrow().iter().find(|m| first == format!("Error: {m}")) {
            if !interrupted {
                return CallError::Unreachable(scrub(said));
            }
        }
        if Instant::now() >= self.deadline.get() || interrupted {
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
        CallError::Script(text)
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
        // No control or format characters: a line of the log stays one
        // line, read in the order it is written.
        let line: String = message.chars().filter(|c| !c.is_control() && !crate::manifest::is_format_char(*c)).take(500).collect();
        let line = crate::net::scrub(&line);
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
        let f = Function::new(ctx.clone(), move |ctx: Ctx<'js>, url: rquickjs::String<'js>, opts: Opt<Object<'js>>| {
            request::send(&ctx, &h, post, url, opts.0)
        })?;
        http.set(if post { "post" } else { "get" }, f)?;
    }
    api.set("http", http)?;

    let html_api = Object::new(ctx.clone())?;
    let h = host.clone();
    html_api.set("parse", Function::new(ctx.clone(), move |ctx: Ctx<'js>, text: rquickjs::String<'js>| html::parse(&ctx, &h, &text))?)?;
    api.set("html", html_api)?;

    let settings = Object::new(ctx.clone())?;
    let h = host.clone();
    let get = Function::new(ctx.clone(), move |ctx: Ctx<'js>, key: rquickjs::String<'js>| -> rquickjs::Result<Value<'js>> {
        match short(&key, 100)?.and_then(|key| h.env.settings.get(&key)) {
            Some(v) => convert::from_json(&ctx, v),
            None => Ok(Value::new_undefined(ctx)),
        }
    })?;
    settings.set("get", get)?;
    api.set("settings", settings)?;

    let date = Object::new(ctx.clone())?;
    date.set(
        "parse",
        Function::new(ctx.clone(), |text: rquickjs::String<'js>| -> rquickjs::Result<Option<String>> {
            Ok(short(&text, 200)?.and_then(|t| crate::record::parse_date(&t)))
        })?,
    )?;
    api.set("date", date)?;

    let h = host.clone();
    api.set(
        "log",
        Function::new(ctx.clone(), move |message: rquickjs::Coerced<rquickjs::String<'js>>| -> rquickjs::Result<()> {
            // Only the start of a long message is copied out (a line keeps 500).
            let c = message.0.to_cstring()?;
            let text = c.as_str();
            let end = (0..=text.len().min(2048)).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
            h.log(&text[..end]);
            Ok(())
        })?,
    )?;
    api.set(
        "normalize",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, title: rquickjs::String<'js>| -> rquickjs::Result<String> {
            let title = short(&title, 2000)?.ok_or_else(|| type_error(&ctx, "the title is too long"))?;
            Ok(normalize_title(&title))
        })?,
    )?;
    let h = host.clone();
    api.set(
        "unreachable",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, message: Opt<rquickjs::Coerced<rquickjs::String<'js>>>| -> rquickjs::Result<Value<'js>> {
                let text = match message.0 {
                    Some(m) => {
                        let c = m.0.to_cstring()?;
                        let text = c.as_str();
                        let end = (0..=text.len().min(2048)).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
                        h.declare_unreachable(&text[..end])
                    }
                    None => h.declare_unreachable(""),
                };
                Ok(Exception::from_message(ctx, &text)?.into_value())
            },
        )?,
    )?;
    api.set("language", host.env.language.clone())?;
    Ok(api)
}

/// `s` copied out when it is at most `max` bytes, else `None`.
fn short(s: &rquickjs::String<'_>, max: usize) -> rquickjs::Result<Option<String>> {
    convert::text_within(s, max)
}

/// Throws a `TypeError` with `message`.
pub(super) fn type_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    Exception::throw_type(ctx, message)
}
