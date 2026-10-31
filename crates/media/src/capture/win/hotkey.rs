//! A system-wide keyboard shortcut on Windows: `RegisterHotKey` on a thread
//! of its own, whose message loop receives the `WM_HOTKEY` of each press.
//!
//! `RegisterHotKey` rather than a low-level keyboard hook: Windows says when
//! another program holds the same keys (so the settings can say so), it
//! fires while a game running as administrator has the focus (a hook does
//! not see that game's keys), and it costs nothing between presses (a hook
//! sits in the path of every key typed). The price: while it is registered,
//! the focused window, the game included, does not get those keys. That is
//! why it only exists while a game runs; [`HotkeySpec::consume`] cannot be
//! turned off here.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

use heronite_i18n::tr;
use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY, WM_QUIT};

use super::super::{to_windows_hotkey, HotkeySpec, ShortcutBackend};

/// The id of the hotkey on its thread (each thread has one).
const HOTKEY_ID: i32 = 1;

type Ready = Box<dyn FnOnce(Result<ShortcutBackend, String>) + Send>;

/// A registered shortcut. Dropping it unregisters it and ends its thread
/// (at once: the thread only waits for messages).
pub struct GlobalHotkey {
    /// The message loop's thread, 0 when nothing was started.
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl GlobalHotkey {
    /// Registers `spec`. `on_press` runs on the shortcut's thread for each
    /// press and must return quickly (start the work elsewhere); `on_ready`
    /// gets the registration result once.
    pub fn start(spec: HotkeySpec, on_press: Arc<dyn Fn() + Send + Sync>, on_ready: Ready) -> Self {
        let none = Self { thread_id: 0, thread: None };
        let Some(hotkey) = to_windows_hotkey(&spec.accelerator) else {
            on_ready(Err(tr!("The shortcut {accelerator} is unavailable", accelerator = &spec.accelerator)));
            return none;
        };
        let (started, thread_id) = mpsc::channel();
        let accelerator = spec.accelerator.clone();
        let spawned = std::thread::Builder::new().name(format!("heronite-hotkey-{}", spec.id)).spawn(move || {
            let mut msg = MSG::default();
            // SAFETY: creates this thread's message queue, so a quit posted
            // from now on is not lost; `msg` is a valid MSG.
            unsafe {
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                let _ = started.send(GetCurrentThreadId());
            }
            let modifiers = HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT;
            // SAFETY: a thread hotkey (no window); unregistered on this thread below.
            if let Err(e) = unsafe { RegisterHotKey(None, HOTKEY_ID, modifiers, hotkey.vk) } {
                let error = if e.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() {
                    tr!("The shortcut {accelerator} is already used by another program", accelerator = &accelerator)
                } else {
                    tr!("The shortcut {accelerator} could not be registered: {error}", accelerator = &accelerator, error = e.message())
                };
                on_ready(Err(error));
                return;
            }
            on_ready(Ok(ShortcutBackend::Windows));
            // SAFETY: `msg` is a valid MSG; 0 (WM_QUIT) or -1 (error) ends the loop.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_ID as usize {
                    on_press();
                }
            }
            // SAFETY: registered on this thread above.
            if let Err(e) = unsafe { UnregisterHotKey(None, HOTKEY_ID) } {
                log::warn!("Could not unregister the shortcut {accelerator}: {e}");
            }
        });
        match spawned {
            Ok(thread) => Self { thread_id: thread_id.recv().unwrap_or(0), thread: Some(thread) },
            Err(e) => {
                log::warn!("could not start the shortcut thread: {e}");
                none
            }
        }
    }
}

impl Drop for GlobalHotkey {
    fn drop(&mut self) {
        if self.thread_id == 0 {
            return;
        }
        // SAFETY: posting to a thread id; failing means the thread ended.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        // Not when the last owner goes away on the shortcut's own thread
        // (from `on_press`): it ends by itself after the quit.
        // SAFETY: no preconditions.
        if unsafe { GetCurrentThreadId() } != self.thread_id {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
