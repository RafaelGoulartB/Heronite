//! Sleeping that ends early. A tracking thread waits here instead of waking
//! up every few milliseconds to look at its stop flags: whoever changes a
//! flag calls [`Wake::notify`], so the loops can poll the game itself only
//! once a second and still react to a stop at once.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Wake {
    lock: Mutex<()>,
    cv: Condvar,
}

impl Wake {
    /// Wakes every waiter. Call it after changing what their `keep_waiting`
    /// reads (the lock orders the change before their next check).
    pub fn notify(&self) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.cv.notify_all();
    }

    /// Sleeps up to `timeout`, or less once `keep_waiting` turns false.
    pub fn wait_while(&self, timeout: Duration, keep_waiting: impl Fn() -> bool) {
        let deadline = Instant::now().checked_add(timeout);
        let mut guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        while keep_waiting() {
            let left = match deadline {
                Some(d) => d.saturating_duration_since(Instant::now()),
                None => Duration::from_secs(3600),
            };
            if left.is_zero() {
                return;
            }
            guard = self.cv.wait_timeout(guard, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Wake;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn times_out_when_nothing_changes() {
        let wake = Wake::default();
        let t = Instant::now();
        wake.wait_while(Duration::from_millis(30), || true);
        assert!(t.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn returns_at_once_when_notified() {
        let wake = Arc::new(Wake::default());
        let flag = Arc::new(AtomicBool::new(false));
        let (w, f) = (wake.clone(), flag.clone());
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            f.store(true, Ordering::SeqCst);
            w.notify();
        });
        let t = Instant::now();
        wake.wait_while(Duration::from_secs(10), || !flag.load(Ordering::SeqCst));
        assert!(t.elapsed() < Duration::from_secs(5));
        setter.join().unwrap();
    }

    #[test]
    fn does_not_wait_when_already_done() {
        let wake = Wake::default();
        let t = Instant::now();
        wake.wait_while(Duration::from_secs(10), || false);
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}
