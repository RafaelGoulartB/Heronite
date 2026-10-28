//! Bounded-concurrency, prioritised, de-duplicating background loader built on
//! plain `std::thread` (no async runtime).

use crate::{ImageError, ImageRef, Opts, Variant};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

/// Result delivered to callbacks. `Arc` because identical in-flight requests
/// share one decode; take the buffer with [`Decoded::from_shared`](crate::Decoded::from_shared)
/// (zero-copy when you are the only holder, which is the common case).
pub type LoadResult = Result<Arc<crate::Decoded>, ImageError>;

/// Identity of a request: same chain + size + variant => same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestKey(pub u64);

#[derive(Debug, Clone)]
pub struct Request {
    pub chain: Vec<ImageRef>,
    pub width: u32,
    pub height: u32,
    pub variant: Variant,
    /// Higher runs first (e.g. visible tiles 100, near-viewport 10, rest 0).
    /// Equal priorities run FIFO.
    pub priority: i32,
}

impl Request {
    pub fn cover(chain: Vec<ImageRef>, width: u32, height: u32, priority: i32) -> Self {
        Self { chain, width, height, variant: Variant::Cover, priority }
    }

    pub fn key(&self) -> RequestKey {
        // DefaultHasher::new() uses fixed keys: deterministic within a process.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.chain.hash(&mut h);
        self.width.hash(&mut h);
        self.height.hash(&mut h);
        self.variant.hash(&mut h);
        RequestKey(h.finish())
    }
}

#[derive(Debug, Clone)]
pub struct LoaderConfig {
    /// Worker threads = max concurrent decodes (default 3).
    pub threads: usize,
    pub opts: Opts,
}

impl LoaderConfig {
    pub fn new(opts: Opts) -> Self {
        Self { threads: 3, opts }
    }
}

type Callback = Box<dyn FnOnce(LoadResult) + Send + 'static>;

struct Entry {
    req: Request,
    seq: u64,
    running: bool,
    waiters: Vec<Callback>,
}

#[derive(Default)]
struct State {
    entries: HashMap<RequestKey, Entry>,
    next_seq: u64,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    opts: Opts,
}

pub struct Loader {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl Loader {
    pub fn new(cfg: LoaderConfig) -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), cv: Condvar::new(), opts: cfg.opts });
        let workers = (0..cfg.threads.max(1))
            .map(|i| {
                let s = shared.clone();
                std::thread::Builder::new()
                    .name(format!("heronite-img-{i}"))
                    // Decoding needs little stack; keeps per-thread RSS low.
                    .stack_size(512 * 1024)
                    .spawn(move || worker(s))
                    .expect("spawn image worker")
            })
            .collect();
        Self { shared, workers }
    }

    /// Queue `req`. `done` is called on a worker thread with the result.
    ///
    /// If an identical request (same [`RequestKey`]) is queued or running, no
    /// new work is created: `done` joins its waiters and, if still queued, the
    /// priority is raised to the max of both. Returns the request key (usable
    /// with [`set_priority`](Self::set_priority) / [`cancel`](Self::cancel)).
    pub fn submit(&self, req: Request, done: impl FnOnce(LoadResult) + Send + 'static) -> RequestKey {
        let key = req.key();
        let mut st = self.lock();
        if st.shutdown {
            drop(st);
            done(Err(ImageError::Panic("loader shut down".into())));
            return key;
        }
        let seq = st.next_seq;
        match st.entries.get_mut(&key) {
            Some(e) => {
                if !e.running && req.priority > e.req.priority {
                    e.req.priority = req.priority;
                }
                e.waiters.push(Box::new(done));
            }
            None => {
                st.next_seq += 1;
                st.entries.insert(key, Entry { req, seq, running: false, waiters: vec![Box::new(done)] });
                self.shared.cv.notify_one();
            }
        }
        key
    }

    /// Convenience: deliver the result (tagged with the key) over a std channel.
    pub fn submit_channel(&self, req: Request, tx: std::sync::mpsc::Sender<(RequestKey, LoadResult)>) -> RequestKey {
        let key = req.key();
        self.submit(req, move |r| {
            let _ = tx.send((key, r));
        })
    }

    /// Change the priority of a queued request (no effect once running).
    pub fn set_priority(&self, key: RequestKey, priority: i32) {
        if let Some(e) = self.lock().entries.get_mut(&key) {
            if !e.running {
                e.req.priority = priority;
            }
        }
    }

    /// Drop a request that has not started (e.g. its tile scrolled far away);
    /// its callbacks are discarded without being called. Running requests are
    /// left to finish. Returns true if a queued request was removed.
    pub fn cancel(&self, key: RequestKey) -> bool {
        let mut st = self.lock();
        match st.entries.get(&key) {
            Some(e) if !e.running => {
                st.entries.remove(&key);
                true
            }
            _ => false,
        }
    }

    /// Number of requests queued or running.
    pub fn pending(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn opts(&self) -> &Opts {
        &self.shared.opts
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        {
            let mut st = self.lock();
            st.shutdown = true;
            // Queued (not running) requests are dropped without callbacks.
            st.entries.retain(|_, e| e.running);
        }
        self.shared.cv.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// Runs a decode and turns a panic into [`ImageError::Panic`]. The decoders get
/// broken or hostile pictures, so this is the one place where a panic is
/// contained (see `heronite_data::panics`). The callbacks run outside it: a
/// panic there is app code and closes the app.
fn decode_contained(decode: impl FnOnce() -> Result<crate::Decoded, ImageError>) -> LoadResult {
    match heronite_data::panics::contained(decode) {
        Ok(r) => r.map(Arc::new),
        Err(p) => Err(ImageError::Panic(heronite_data::panics::message(&*p))),
    }
}

fn worker(s: Arc<Shared>) {
    loop {
        // Pick the highest-priority queued entry (FIFO among equals).
        let (key, req) = {
            let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if st.shutdown {
                    return;
                }
                let best = st
                    .entries
                    .iter()
                    .filter(|(_, e)| !e.running)
                    .max_by(|a, b| a.1.req.priority.cmp(&b.1.req.priority).then(b.1.seq.cmp(&a.1.seq)))
                    .map(|(k, _)| *k);
                if let Some(k) = best {
                    let e = st.entries.get_mut(&k).unwrap();
                    e.running = true;
                    break (k, e.req.clone());
                }
                st = s.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };

        let result = decode_contained(|| crate::load(&req.chain, req.width, req.height, req.variant, &s.opts));

        let waiters = s.state.lock().unwrap_or_else(|e| e.into_inner()).entries.remove(&key).map(|e| e.waiters).unwrap_or_default();
        let n = waiters.len();
        let mut result = Some(result);
        for (i, cb) in waiters.into_iter().enumerate() {
            // The last waiter gets the only Arc, so `Arc::try_unwrap` succeeds
            // in the common single-waiter case (zero-copy hand-off).
            let r = if i + 1 == n { result.take().unwrap() } else { result.as_ref().unwrap().clone() };
            cb(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decoder_panic_becomes_an_error() {
        let r = decode_contained(|| panic!("bad jpeg {}", 7));
        match r {
            Err(ImageError::Panic(m)) => assert_eq!(m, "bad jpeg 7"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_decoder_runs_in_a_contained_scope() {
        let _ = decode_contained(|| {
            assert!(heronite_data::panics::is_contained());
            Err(ImageError::Panic(String::new()))
        });
        assert!(!heronite_data::panics::is_contained());
    }
}
