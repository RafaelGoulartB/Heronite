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

mod convert;
mod host;
mod html;
mod request;

use std::rc::Rc;
use std::sync::Arc;

use rquickjs::context::intrinsic;
use rquickjs::module::WriteOptions;
use rquickjs::{Context, Ctx, Function, Module, Object, Runtime, Value};

use host::Host;
pub use host::HostEnv;

use crate::ExtensionError;

/// The stack of a script's thread; the engine's own limit is below it.
const THREAD_STACK: usize = 8 * 1024 * 1024;

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
}

impl CallError {
    /// Whether it counts towards the failures that stop the extension (an
    /// unreachable network does not).
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Unreachable(_))
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(m) => write!(f, "unreachable: {m}"),
            Self::Limit(l, m) => write!(f, "limit ({l:?}): {m}"),
            Self::Denied(m) => write!(f, "denied: {m}"),
            Self::Script(m) => write!(f, "script error: {m}"),
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

/// Runs `f` with the exported functions of `script`, in a fresh runtime on
/// a thread of its own (see the module docs). Blocking.
pub fn session<T: Send>(script: &Script, env: &HostEnv, f: impl FnOnce(&mut dyn Calls) -> T + Send) -> Result<T, CallError> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name(format!("ext:{}", env.id))
            .stack_size(THREAD_STACK)
            .spawn_scoped(scope, || run(script, env, f))
            .map_err(|e| CallError::Script(format!("could not start the engine: {e}")))?;
        thread.join().unwrap_or_else(|_| Err(CallError::Script("the engine stopped unexpectedly".into())))
    })
}

fn run<T>(script: &Script, env: &HostEnv, f: impl FnOnce(&mut dyn Calls) -> T) -> Result<T, CallError> {
    let engine = |e: rquickjs::Error| CallError::Script(format!("engine: {e}"));
    // Declared before the runtime so it is dropped after it: the module is
    // read with `JS_READ_OBJ_ROM_DATA`, which keeps pointing into the buffer.
    let bytecode = script.bytecode.clone();
    let host = Rc::new(Host::new(env.clone()));
    let rt = Runtime::new().map_err(engine)?;
    rt.set_memory_limit(env.limits.memory);
    rt.set_max_stack_size(env.limits.stack);
    let stop = host.clone();
    rt.set_interrupt_handler(Some(Box::new(move || stop.should_stop())));
    let context = Context::custom::<Intrinsics>(&rt).map_err(engine)?;
    let out = context.with(|ctx| -> Result<T, CallError> {
        host.begin();
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
