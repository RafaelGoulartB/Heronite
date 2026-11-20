//! The sandbox: QuickJS (`rquickjs`) running an extension's `source.js`.
//!
//! Lifecycle: the script is compiled once, when the extension is loaded, to
//! QuickJS bytecode ([`Script`], shared by every call). Each lookup then
//! runs in a runtime of its own on a thread of its own ([`session`]): a
//! fresh engine with the memory, stack and time limits of [`Limits`], the
//! bytecode loaded into a context that has no `eval`, no module loader and
//! nothing of the host but the `ctx` argument. QuickJS runtimes are not
//! thread-safe; a runtime per lookup needs no lock, lets the gather ask the
//! same extension about several games at once, and drops whatever a script
//! left behind (prototype changes, globals) when the lookup ends. Creating
//! one costs well under a millisecond (`tests/sandbox.rs` measures it),
//! nothing next to a request.
//!
//! The thread has a known stack size, so the engine's stack limit holds
//! whatever thread asked.
//!
//! The caller never waits for the thread past a call's time limit and
//! [`GRACE`]. The interrupt handler stops a script's own code at the limit,
//! but not a loop inside one of the engine's built-in functions
//! (`Array.prototype.join.call({ length: 2 ** 53 - 1 })`) nor slow work in
//! the host (an HTML parse): such a thread cannot be stopped from outside.
//! It is left behind ([`Limit::Stuck`]): it keeps a processor core and up
//! to the memory limit until it ends by itself or the app quits. The
//! extension is stopped at once, at most [`MAX_STUCK_PER_EXTENSION`]
//! threads of one extension and [`MAX_STUCK`] in all may be left behind,
//! and past that no session starts. [`arrays`] refuses the known cases
//! before they run, so a thread is left behind only for what it misses.

mod arrays;
mod convert;
mod host;
mod html;
mod request;

use std::rc::Rc;
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use rquickjs::context::intrinsic;
use rquickjs::module::WriteOptions;
use rquickjs::{Context, Ctx, Function, Module, Object, Runtime, Value};

use host::Host;
pub use host::HostEnv;

use crate::ExtensionError;

/// The stack of a script's thread; the engine's own limit is below it.
const THREAD_STACK: usize = 8 * 1024 * 1024;

/// How long past a call's time limit its caller waits for the thread: the
/// interrupt handler, a request's own timeout and the host's work after it
/// fit in it.
pub const GRACE: Duration = Duration::from_secs(2);
/// Threads of one extension left running past their time; at this many it
/// starts no other session until the app restarts.
pub const MAX_STUCK_PER_EXTENSION: usize = 1;
/// Threads of all extensions left running past their time; at this many
/// no extension starts a session until the app restarts.
pub const MAX_STUCK: usize = 3;

/// The threads left running, by extension id.
static STUCK: Mutex<Vec<String>> = parking_lot::const_mutex(Vec::new());

/// How many threads of extension `id` are still running past their time.
pub fn stuck(id: &str) -> usize {
    STUCK.lock().iter().filter(|s| *s == id).count()
}

/// What the context offers besides the base objects: no `eval` (nor the
/// `Function` constructor, which needs it), no `performance`, no `WeakRef`.
type Intrinsics = (
    intrinsic::Date,
    intrinsic::RegExpCompiler,
    intrinsic::RegExp,
    intrinsic::Json,
    intrinsic::MapSet,
    intrinsic::TypedArrays,
    intrinsic::Promise,
    intrinsic::Proxy,
);

/// Which limit a call went over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Time,
    Memory,
    Stack,
    Requests,
    Bytes,
    /// It returned a promise nothing could settle.
    NeverSettled,
    /// It returned more data than a source may give.
    Size,
    /// It ran past its time and [`GRACE`] where nothing can interrupt it
    /// (inside a built-in function, or the host's own work); its thread was
    /// left behind. The extension is stopped at once.
    Stuck,
}

/// Why a call of a script gave nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// A request got no answer (offline, DNS, timeout).
    Unreachable(String),
    Limit(Limit, String),
    /// It asked for something it may not (a host not in its manifest).
    Denied(String),
    /// It threw, or its module did not load.
    Script(String),
    /// Not started: too many threads of other extensions are stuck
    /// ([`MAX_STUCK`]). Not this extension's fault.
    Busy(String),
}

impl CallError {
    /// Whether it counts towards the failures that stop the extension (an
    /// unreachable network or other extensions' stuck threads do not).
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Unreachable(_) | Self::Busy(_))
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(m) => write!(f, "unreachable: {m}"),
            Self::Limit(l, m) => write!(f, "limit ({l:?}): {m}"),
            Self::Denied(m) => write!(f, "denied: {m}"),
            Self::Script(m) => write!(f, "script error: {m}"),
            Self::Busy(m) => write!(f, "not started: {m}"),
        }
    }
}

/// A compiled `source.js`.
#[derive(Clone)]
pub struct Script {
    bytecode: Arc<Vec<u8>>,
    /// The functions it exports.
    exports: Arc<Vec<String>>,
}

impl Script {
    /// Compiles `source` (an ES module) and loads it once in the sandbox to
    /// see what it exports; `find` and `fetch` are required.
    pub fn compile(source: &str, env: &HostEnv) -> Result<Self, ExtensionError> {
        let fail = |m: String| ExtensionError::Script(m);
        let rt = Runtime::new().map_err(|e| fail(e.to_string()))?;
        rt.set_memory_limit(64 * 1024 * 1024);
        // Compiling needs `eval`; the bytecode then runs where there is none.
        let ctx = Context::full(&rt).map_err(|e| fail(e.to_string()))?;
        let bytecode = ctx.with(|ctx| {
            let module = Module::declare(ctx.clone(), "source.js", source).map_err(|e| describe(&ctx, e))?;
            module.write(WriteOptions::default()).map_err(|e| describe(&ctx, e))
        });
        drop(ctx);
        drop(rt);
        let mut script = Self { bytecode: Arc::new(bytecode.map_err(fail)?), exports: Arc::new(Vec::new()) };
        let exports = session(&script, env, |calls| calls.exports()).map_err(|e| fail(e.to_string()))?;
        for required in ["find", "fetch"] {
            if !exports.iter().any(|e| e == required) {
                return Err(fail(format!("source.js does not export a function {required}")));
            }
        }
        script.exports = Arc::new(exports);
        Ok(script)
    }

    pub fn exports(&self, name: &str) -> bool {
        self.exports.iter().any(|e| e == name)
    }
}

/// The text of the exception `e` left in `ctx`: its message and stack.
fn describe(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    if !matches!(e, rquickjs::Error::Exception) {
        return e.to_string();
    }
    exception_text(&ctx.catch())
}

fn exception_text(v: &Value<'_>) -> String {
    let text = |o: &Object<'_>, k: &str| o.get::<_, Option<rquickjs::Coerced<String>>>(k).ok().flatten().map(|s| s.0);
    let out = match v.as_object() {
        Some(o) => {
            let message = text(o, "message").unwrap_or_else(|| "error".to_owned());
            let name = text(o, "name").filter(|n| !n.is_empty());
            let stack = text(o, "stack").filter(|s| !s.trim().is_empty());
            let head = match name {
                Some(n) => format!("{n}: {message}"),
                None => message,
            };
            match stack {
                Some(s) => format!("{head}\n{}", s.trim_end()),
                None => head,
            }
        }
        None => v.get::<rquickjs::Coerced<String>>().map(|s| s.0).unwrap_or_else(|_| "error".to_owned()),
    };
    out.chars().take(2000).collect()
}

/// The functions a script exports, called one at a time.
pub trait Calls {
    /// Names of the exported functions.
    fn exports(&self) -> Vec<String>;

    /// Calls export `name` with `arg` (if any) and the `ctx` object, waits
    /// for its promise, and returns what it settled to as JSON. Each call
    /// has its own time, request and byte budget.
    fn call(&mut self, name: &str, arg: Option<serde_json::Value>) -> Result<serde_json::Value, CallError>;

    /// Why the last call's last failed request got no answer, if one did.
    fn unreachable(&self) -> Option<String>;
}

struct JsCalls<'js> {
    ctx: Ctx<'js>,
    namespace: Object<'js>,
    api: Object<'js>,
    host: Rc<Host>,
}

impl Calls for JsCalls<'_> {
    fn exports(&self) -> Vec<String> {
        let mut out = Vec::new();
        for key in self.namespace.keys::<String>().flatten() {
            if self.namespace.get::<_, Value>(&key).is_ok_and(|v| v.is_function()) {
                out.push(key);
            }
        }
        out
    }

    fn call(&mut self, name: &str, arg: Option<serde_json::Value>) -> Result<serde_json::Value, CallError> {
        let ctx = &self.ctx;
        self.host.begin();
        let func: Function = self
            .namespace
            .get::<_, Option<Function>>(name)
            .ok()
            .flatten()
            .ok_or_else(|| CallError::Script(format!("source.js does not export a function {name}")))?;
        let result = match arg {
            Some(arg) => {
                let arg = convert::from_json(ctx, &arg).map_err(|e| self.host.error(ctx, e))?;
                func.call::<_, Value>((arg, self.api.clone()))
            }
            None => func.call::<_, Value>((self.api.clone(),)),
        };
        let value = result.map_err(|e| self.host.error(ctx, e))?;
        let value = self.host.settle(ctx, value)?;
        // A violation the script caught still ends the call.
        if let Some(e) = self.host.aborted() {
            return Err(e);
        }
        convert::to_json(&value).map_err(|m| CallError::Limit(Limit::Size, m))
    }

    fn unreachable(&self) -> Option<String> {
        self.host.unreachable()
    }
}

/// When the call running on a session's thread must end, and whether its
/// caller stopped waiting; shared by the thread and its caller.
pub(crate) struct Watch {
    state: Mutex<WatchState>,
}

struct WatchState {
    deadline: Instant,
    done: bool,
    abandoned: bool,
}

impl Watch {
    fn new(deadline: Instant) -> Self {
        Self { state: Mutex::new(WatchState { deadline, done: false, abandoned: false }) }
    }

    /// A new call (or the module's load) starts and must end by `deadline`.
    pub(crate) fn set_deadline(&self, deadline: Instant) {
        self.state.lock().deadline = deadline;
    }

    fn deadline(&self) -> Instant {
        self.state.lock().deadline
    }

    /// The caller gives up on thread of extension `id`: `false` when the
    /// thread finished meanwhile (its answer is in the channel).
    fn abandon(&self, id: &str) -> bool {
        let mut state = self.state.lock();
        if state.done {
            return false;
        }
        state.abandoned = true;
        STUCK.lock().push(id.to_owned());
        true
    }
}

/// Marks the thread's work done when dropped, and frees its place among
/// the stuck ones if its caller had given up on it.
struct Finished {
    watch: Arc<Watch>,
    id: String,
}

impl Drop for Finished {
    fn drop(&mut self) {
        let mut state = self.watch.state.lock();
        state.done = true;
        if state.abandoned {
            let mut stuck = STUCK.lock();
            if let Some(i) = stuck.iter().position(|s| *s == self.id) {
                stuck.swap_remove(i);
            }
            log::info!("Extension {}: a call left running past its time has ended", self.id);
        }
    }
}

/// Runs `f` with the exported functions of `script`, in a fresh runtime on
/// a thread of its own (see the module docs). Blocking, but never longer
/// than each call's time limit and [`GRACE`]: past that the thread is left
/// behind and this returns [`Limit::Stuck`].
pub fn session<T: Send + 'static>(
    script: &Script,
    env: &HostEnv,
    f: impl FnOnce(&mut dyn Calls) -> T + Send + 'static,
) -> Result<T, CallError> {
    {
        let stuck = STUCK.lock();
        let own = stuck.iter().filter(|s| **s == env.id).count();
        if own >= MAX_STUCK_PER_EXTENSION {
            return Err(CallError::Limit(
                Limit::Stuck,
                "a call of it that ran past its time limit is still running; it is not started again until the app restarts".into(),
            ));
        }
        if stuck.len() >= MAX_STUCK {
            return Err(CallError::Busy(format!("{} calls of extensions are still running past their time limit", stuck.len())));
        }
    }
    let watch = Arc::new(Watch::new(Instant::now() + env.limits.time));
    let (tx, rx) = sync_channel(1);
    let (script, thread_env, thread_watch) = (script.clone(), env.clone(), watch.clone());
    std::thread::Builder::new()
        .name(format!("ext:{}", env.id))
        .stack_size(THREAD_STACK)
        .spawn(move || {
            let _finished = Finished { watch: thread_watch.clone(), id: thread_env.id.clone() };
            let _ = tx.send(run(&script, &thread_env, &thread_watch, f));
        })
        .map_err(|e| CallError::Script(format!("could not start the engine: {e}")))?;
    loop {
        let until = watch.deadline() + GRACE;
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(out) => return out,
            Err(RecvTimeoutError::Disconnected) => return Err(CallError::Script("the engine stopped unexpectedly".into())),
            // A later call of the session moved the deadline on.
            Err(RecvTimeoutError::Timeout) if Instant::now() < watch.deadline() + GRACE => {}
            Err(RecvTimeoutError::Timeout) => {
                if !watch.abandon(&env.id) {
                    return rx.recv().unwrap_or_else(|_| Err(CallError::Script("the engine stopped unexpectedly".into())));
                }
                log::warn!("Extension {}: a call ran past its time limit and could not be stopped; its thread is left running", env.id);
                return Err(CallError::Limit(
                    Limit::Stuck,
                    format!(
                        "it ran past its time limit ({:?}) inside the script engine and could not be stopped; that call keeps using a processor core until it ends or the app restarts",
                        env.limits.time
                    ),
                ));
            }
        }
    }
}

fn run<T>(script: &Script, env: &HostEnv, watch: &Arc<Watch>, f: impl FnOnce(&mut dyn Calls) -> T) -> Result<T, CallError> {
    let engine = |e: rquickjs::Error| CallError::Script(format!("engine: {e}"));
    // Declared before the runtime so it is dropped after it: the module is
    // read with `JS_READ_OBJ_ROM_DATA`, which keeps pointing into the buffer.
    let bytecode = script.bytecode.clone();
    let host = Rc::new(Host::new(env.clone(), watch.clone()));
    let rt = Runtime::new().map_err(engine)?;
    rt.set_memory_limit(env.limits.memory);
    rt.set_max_stack_size(env.limits.stack);
    let stop = host.clone();
    rt.set_interrupt_handler(Some(Box::new(move || stop.should_stop())));
    let context = Context::custom::<Intrinsics>(&rt).map_err(engine)?;
    let out = context.with(|ctx| -> Result<T, CallError> {
        host.begin();
        let guarded = arrays::guard(&ctx).map_err(|e| host.error(&ctx, e))?;
        host.settle(&ctx, guarded.into_value())?;
        // SAFETY: the bytes are what `Script::compile` wrote with this
        // build's engine, never read from disk or from the extension.
        let module = unsafe { Module::load(ctx.clone(), &bytecode) }.map_err(|e| host.error(&ctx, e))?;
        let (module, done) = module.eval().map_err(|e| host.error(&ctx, e))?;
        host.settle(&ctx, done.into_value())?;
        if let Some(e) = host.aborted() {
            return Err(e);
        }
        let namespace = module.namespace().map_err(|e| host.error(&ctx, e))?;
        let api = host::api(&ctx, &host).map_err(|e| host.error(&ctx, e))?;
        let mut calls = JsCalls { ctx: ctx.clone(), namespace, api, host: host.clone() };
        Ok(f(&mut calls))
    });
    drop(context);
    drop(rt);
    out
}
