//! The console options that act while console mode is off: how the app
//! opens and what a controller connecting does. They are kept in
//! `console.json` and mirrored in `ui-state.json` (read at start anyway), so a
//! user who never turns these on pays for no file read and no watcher.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use heronite_data::console_theme::{AutoConsole, OpenIn};
use heronite_data::paths::AppPaths;
use heronite_data::ui_state;

pub fn set_open_in(app: &AppPaths, v: OpenIn) {
    super::themes::update(app, |p| p.open_in = v);
    if let Err(e) = ui_state::set_console_open_in(app, v) {
        log::warn!("mirroring openIn: {e:#}");
    }
}

pub fn set_auto_console(app: &AppPaths, v: AutoConsole) {
    LATEST_AUTO.store(AutoConsole::ALL.iter().position(|a| *a == v).map_or(NONE, |i| i as u8), Ordering::Relaxed);
    super::themes::update(app, |p| p.auto_console = v);
    if let Err(e) = ui_state::set_console_auto(app, v) {
        log::warn!("mirroring autoConsole: {e:#}");
    }
}

/// The last `autoConsole` set in this run, as an index of `AutoConsole::ALL`.
static LATEST_AUTO: AtomicU8 = AtomicU8::new(NONE);
const NONE: u8 = u8::MAX;

/// `autoConsole` as it is now: what was set in this run, else `loaded`.
pub fn auto_console(loaded: AutoConsole) -> AutoConsole {
    AutoConsole::ALL.get(usize::from(LATEST_AUTO.load(Ordering::Relaxed))).copied().unwrap_or(loaded)
}

/// Whether the app starts in console mode: `Last` repeats how it was left.
pub fn opens_in_console(open_in: OpenIn, left_in_console: bool) -> bool {
    match open_in {
        OpenIn::Last => left_in_console,
        OpenIn::Console => true,
        OpenIn::Desktop => false,
    }
}

/// How often the watcher looks for a controller.
const LOOK_EVERY: Duration = Duration::from_secs(3);
/// `BTN_SOUTH` (`BTN_GAMEPAD`): what separates a game controller from a
/// keyboard or mouse among the input devices.
const BTN_SOUTH: usize = 0x130;

/// Whether the `key` capability bitmap of an input device (the hex words of
/// `/sys/class/input/eventN/device/capabilities/key`, highest first) has the
/// south button.
fn has_gamepad_buttons(caps: &str) -> bool {
    let bits = usize::BITS as usize;
    let words: Vec<usize> = caps.split_whitespace().filter_map(|w| usize::from_str_radix(w, 16).ok()).collect();
    words.len().checked_sub(1 + BTN_SOUTH / bits).and_then(|ix| words.get(ix)).is_some_and(|w| w >> (BTN_SOUTH % bits) & 1 == 1)
}

/// The input devices under `root` (`/sys/class/input`) that are controllers.
fn gamepads(root: &Path) -> BTreeSet<String> {
    let Ok(dir) = std::fs::read_dir(root) else { return BTreeSet::new() };
    dir.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let caps = std::fs::read_to_string(e.path().join("device/capabilities/key")).ok()?;
            (name.starts_with("event") && has_gamepad_buttons(&caps)).then_some(name)
        })
        .collect()
}

/// A controller in `now` that `known` did not have.
fn appeared(known: &BTreeSet<String>, now: &BTreeSet<String>) -> bool {
    now.difference(known).next().is_some()
}

/// Looks for controllers while console mode is off. Dropping it stops the
/// thread.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    thread: std::thread::Thread,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.unpark();
    }
}

/// Starts watching: the receiver gets `()` each time a controller appears
/// (`including_present`: also for the ones there when it starts). `None`
/// when the thread cannot start.
pub fn watch(including_present: bool) -> Option<(Watcher, UnboundedReceiver<()>)> {
    let (tx, rx) = unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = std::thread::Builder::new()
        .name("pad-watch".into())
        .spawn(move || {
            let root = Path::new("/sys/class/input");
            let mut known = if including_present { BTreeSet::new() } else { gamepads(root) };
            while !flag.load(Ordering::Relaxed) {
                let now = gamepads(root);
                if appeared(&known, &now) && tx.unbounded_send(()).is_err() {
                    break;
                }
                known = now;
                std::thread::park_timeout(LOOK_EVERY);
            }
        })
        .map_err(|e| log::warn!("controller watcher: {e}"))
        .ok()?;
    Some((Watcher { stop, thread: handle.thread().clone() }, rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_where_the_option_says() {
        assert!(opens_in_console(OpenIn::Last, true));
        assert!(!opens_in_console(OpenIn::Last, false));
        assert!(opens_in_console(OpenIn::Console, false));
        assert!(!opens_in_console(OpenIn::Desktop, true));
    }

    #[test]
    fn gamepads_have_the_south_button() {
        let bits = usize::BITS as usize;
        let words_for = |bit: usize| -> String {
            let n = bit / bits + 1;
            (0..n)
                .rev()
                .map(|i| if i == bit / bits { format!("{:x}", 1usize << (bit % bits)) } else { "0".to_string() })
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert!(has_gamepad_buttons(&words_for(BTN_SOUTH)));
        // A keyboard has keys up to 0x100 or so, and no gamepad bit.
        assert!(!has_gamepad_buttons(&words_for(0x72)));
        assert!(!has_gamepad_buttons(""));
        assert!(!has_gamepad_buttons("ffff ffff"));
    }

    #[test]
    fn only_new_controllers_count() {
        let set = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert!(appeared(&set(&[]), &set(&["event5"])));
        assert!(!appeared(&set(&["event5"]), &set(&["event5"])));
        assert!(!appeared(&set(&["event5"]), &set(&[])));
        assert!(appeared(&set(&["event5"]), &set(&["event5", "event9"])));
    }
}
