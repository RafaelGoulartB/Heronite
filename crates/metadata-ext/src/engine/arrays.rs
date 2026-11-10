//! The generic methods of `Array.prototype`, wrapped so they refuse an
//! object whose `length` no array of the sandbox can reach. Called on an
//! array-like such as `{ length: 2 ** 53 - 1 }`, the engine's own `join`,
//! `reverse`, `copyWithin`, `flat`... loop up to that length in C, where
//! the interrupt handler never runs. The wrappers are a module of their own
//! run before the extension's, so its code never sees the originals; they
//! are JavaScript, so the engine's collector sees what they hold.
//!
//! A first line only: a `length` that changes between this check and the
//! method's own read, or another built-in that loops the same way, still
//! gets through; the session's time limit ([`super::session`]) is what
//! keeps the caller from being blocked.

use std::sync::OnceLock;

use rquickjs::module::WriteOptions;
use rquickjs::{Context, Ctx, Module, Promise, Runtime};

const SOURCE: &str = r#"
const MAX_LENGTH = 2 ** 24;
const apply = Reflect.apply;
const define = Object.defineProperty;
const TooLarge = RangeError;
const proto = Array.prototype;
for (const name of ["at", "concat", "copyWithin", "every", "fill", "filter", "find", "findIndex", "findLast",
    "findLastIndex", "flat", "flatMap", "forEach", "includes", "indexOf", "join", "lastIndexOf", "map", "pop",
    "push", "reduce", "reduceRight", "reverse", "shift", "slice", "some", "sort", "splice", "toLocaleString",
    "toReversed", "toSorted", "toSpliced", "unshift", "with"]) {
  const original = proto[name];
  if (typeof original !== "function") continue;
  const wrapped = {
    [name](...args) {
      if ((typeof this === "object" && this !== null) || typeof this === "function") {
        const length = +this.length;
        if (length > MAX_LENGTH) throw new TooLarge(`Array.prototype.${name}: length ${length} is too large`);
      }
      return apply(original, this, args);
    },
  }[name];
  define(proto, name, { value: wrapped, writable: true, enumerable: false, configurable: true });
}
"#;

/// The wrappers' module as bytecode, compiled once.
fn bytecode() -> &'static [u8] {
    static CODE: OnceLock<Vec<u8>> = OnceLock::new();
    CODE.get_or_init(|| {
        let rt = Runtime::new().expect("a runtime");
        let ctx = Context::full(&rt).expect("a context");
        ctx.with(|ctx| Module::declare(ctx.clone(), "array-guard", SOURCE)?.write(WriteOptions::default()))
            .expect("the array guard compiles")
    })
}

/// Wraps the methods in `ctx`; returns the module's promise.
pub(super) fn guard<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Promise<'js>> {
    // SAFETY: the bytes are what `bytecode` compiled with this build's
    // engine; they live as long as the process.
    let module = unsafe { Module::load(ctx.clone(), bytecode()) }?;
    Ok(module.eval()?.1)
}
