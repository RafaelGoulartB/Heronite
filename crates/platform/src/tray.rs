//! An icon in the notification area of the Windows taskbar, with a menu.
//!
//! [`Tray::spawn`] adds the icon (`Shell_NotifyIconW`) and keeps it on a
//! thread of its own, where a hidden window receives its clicks. A left click
//! (or Enter on the focused icon) calls [`Options::on_click`]; a right click
//! (or the menu key) asks [`Options::menu`] for the rows and shows them, so
//! the menu is always current. When Explorer restarts, the icon is added
//! again. [`Tray::remove`], or dropping the [`Tray`], takes it away and ends
//! the thread. The callbacks run on that thread.

use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::{io, mem, ptr};

use windows_sys::core::PCWSTR;
use windows_sys::w;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForSystem, GetSystemMetricsForDpi, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NINF_KEY, NIN_SELECT,
    NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW,
    FindWindowW, GetMessageW, GetSystemMetrics, KillTimer, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW,
    SetForegroundWindow, SetTimer, TrackPopupMenuEx, TranslateMessage, HICON, MENU_ITEM_FLAGS, MF_SEPARATOR, MF_STRING, MSG, SM_CXSMICON,
    SM_MENUDROPALIGNMENT, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_NULL, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

/// The icon's clicks reach the window as this message.
const WM_TRAY: u32 = WM_APP + 1;
/// [`Tray`] left a [`Cmd`] in the channel.
const WM_TRAY_CMD: u32 = WM_APP + 2;
/// Enter or Space on the focused icon (not in `windows-sys`).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const ICON_ID: u32 = 1;
/// While Explorer refuses the icon (it is still starting), it is offered
/// again every [`RETRY_MS`].
const RETRY_TIMER: usize = 1;
const RETRY_MS: u32 = 2000;
/// How long [`Tray::remove`] waits for the icon to go.
const REMOVE_WAIT: Duration = Duration::from_secs(1);
const CLASS: PCWSTR = w!("HeronitePlatformTray");

/// `RegisterWindowMessageW("TaskbarCreated")`: Explorer (re)started and has
/// no icons; 0 until the first tray registers it.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// A picture of the icon: `width × height` RGBA pixels, rows from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Icon {
    /// `None` when a side is 0 or `rgba` is not `width * height * 4` bytes.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Icon> {
        let len = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        (width > 0 && height > 0 && rgba.len() == len).then_some(Icon { width, height, rgba })
    }
}

/// A row of the menu.
pub enum MenuItem {
    /// Runs `action` when picked. `label` is plain text: an `&` shows as is.
    Entry {
        label: String,
        action: Box<dyn FnOnce()>,
    },
    Separator,
}

impl MenuItem {
    pub fn entry(label: impl Into<String>, action: impl FnOnce() + 'static) -> MenuItem {
        MenuItem::Entry { label: label.into(), action: Box::new(action) }
    }
}

/// What the icon shows and does. The callbacks run on the tray's thread.
pub struct Options {
    /// Shown when the pointer rests on the icon; cut to 127 UTF-16 units.
    pub tooltip: String,
    /// The icon at one or more sizes. The notification area draws it at 16 px
    /// at 100 % scaling (32 px at 200 %): that size is used, else the next
    /// larger one, else the largest.
    pub icons: Vec<Icon>,
    /// A left click, or Enter on the focused icon. A double click counts as
    /// one: people double-click tray icons out of habit, and a second call
    /// would undo the first when the click toggles something.
    pub on_click: Box<dyn FnMut() + Send>,
    /// The rows of the menu, asked for each time it opens (a right click, or
    /// the menu key on the focused icon). The picked row's action runs once
    /// the menu has closed.
    pub menu: Box<dyn FnMut() -> Vec<MenuItem> + Send>,
}

/// The icon, while it lives. Dropping it removes the icon.
pub struct Tray {
    /// The hidden window, as a number so the handle can go to other threads.
    hwnd: isize,
    cmds: Sender<Cmd>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

enum Cmd {
    SetIcon(Vec<Icon>),
    Remove,
}

impl Tray {
    /// Adds the icon. Fails when its thread or window cannot be made, or when
    /// there is no notification area (Explorer is not running). Explorer may
    /// refuse the icon while it starts: it is then offered again until taken.
    pub fn spawn(options: Options) -> io::Result<Tray> {
        let (cmds, rx) = mpsc::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new().name("tray".into()).spawn(move || run(options, rx, ready_tx))?;
        match ready.recv() {
            Ok(Ok(hwnd)) => Ok(Tray { hwnd, cmds, thread: Mutex::new(Some(thread)) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(io::Error::other("the tray thread stopped")),
        }
    }

    /// Changes the picture (chosen among `icons` like [`Options::icons`]).
    pub fn set_icon(&self, icons: Vec<Icon>) {
        self.send(Cmd::SetIcon(icons));
    }

    /// Takes the icon away and ends its thread; later calls do nothing. Waits
    /// up to a second for the icon to go: one left by a process that exited
    /// stays in the notification area until the pointer passes over it.
    pub fn remove(&self) {
        let Some(thread) = self.thread.lock().unwrap_or_else(PoisonError::into_inner).take() else { return };
        self.send(Cmd::Remove);
        // From a callback: the thread ends once the callback returns.
        if thread.thread().id() == std::thread::current().id() {
            return;
        }
        let deadline = Instant::now() + REMOVE_WAIT;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            log::warn!("the tray icon was not removed in {REMOVE_WAIT:?}");
        }
    }

    fn send(&self, cmd: Cmd) {
        // The channel closes when the thread ends; its window is gone then.
        if self.cmds.send(cmd).is_ok() {
            // SAFETY: posting to our window; one that just went away only makes it fail.
            unsafe { PostMessageW(self.hwnd as HWND, WM_TRAY_CMD, 0, 0) };
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

thread_local! {
    /// The tray of this thread, for the window procedure.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    hwnd: HWND,
    tip: [u16; 128],
    icons: Vec<Icon>,
    /// Made from the best of `icons`; Explorer keeps a copy of its own.
    hicon: HICON,
    /// Explorer has the icon.
    shown: bool,
    /// The retry timer runs (Explorer refused the icon).
    retrying: bool,
    /// When the last click that counted came.
    last_click: Option<Instant>,
    on_click: Box<dyn FnMut() + Send>,
    menu: Box<dyn FnMut() -> Vec<MenuItem> + Send>,
    cmds: Receiver<Cmd>,
}

/// The tray's thread: its window, the icon, then the message loop until
/// [`Cmd::Remove`].
fn run(options: Options, cmds: Receiver<Cmd>, ready: SyncSender<io::Result<isize>>) {
    // Sharp menus and the real icon size on a scaled screen, whatever the
    // rest of the process declares.
    // SAFETY: changes this thread's DPI awareness only.
    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    // SAFETY: the string is NUL-terminated.
    TASKBAR_CREATED.store(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }, Ordering::Relaxed);
    let hwnd = match create_window() {
        Ok(hwnd) => hwnd,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let mut state = State {
        hwnd,
        tip: tooltip(&options.tooltip),
        hicon: make_icon(&options.icons),
        icons: options.icons,
        shown: false,
        retrying: false,
        last_click: None,
        on_click: options.on_click,
        menu: options.menu,
        cmds,
    };
    // SAFETY: the class name is NUL-terminated.
    if !state.show() && unsafe { FindWindowW(w!("Shell_TrayWnd"), ptr::null()) }.is_null() {
        drop(state);
        // SAFETY: our window, destroyed on its own thread.
        unsafe { DestroyWindow(hwnd) };
        let _ = ready.send(Err(io::Error::other("no notification area (Explorer is not running)")));
        return;
    }
    STATE.with(|s| *s.borrow_mut() = Some(state));
    let _ = ready.send(Ok(hwnd as isize));
    // SAFETY: MSG is plain data; all zeros is a valid value.
    let mut msg: MSG = unsafe { mem::zeroed() };
    // SAFETY: `msg` is ours to fill; 0 is WM_QUIT (posted on WM_DESTROY), -1 an error.
    while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
        // SAFETY: a message GetMessageW just filled.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    STATE.with(|s| s.borrow_mut().take());
}

fn module() -> HINSTANCE {
    // SAFETY: null asks for this executable, whose handle needs no release.
    unsafe { GetModuleHandleW(ptr::null()) }
}

fn create_window() -> io::Result<HWND> {
    let class = WNDCLASSEXW {
        cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: module(),
        lpszClassName: CLASS,
        ..Default::default()
    };
    // SAFETY: `class` is filled and names a static string. A second tray in
    // the process finds the class registered.
    if unsafe { RegisterClassExW(&class) } == 0 {
        // SAFETY: reads this thread's last error, set by RegisterClassExW.
        let code = unsafe { GetLastError() };
        if code != ERROR_CLASS_ALREADY_EXISTS {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
    }
    // A hidden top-level window: a message-only one would miss Explorer's
    // `TaskbarCreated` broadcast.
    // SAFETY: the class is registered above; every pointer is static or null.
    let hwnd = unsafe {
        CreateWindowExW(WS_EX_TOOLWINDOW, CLASS, w!(""), WS_OVERLAPPED, 0, 0, 0, 0, ptr::null_mut(), ptr::null_mut(), module(), ptr::null())
    };
    if hwnd.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(hwnd)
}

impl State {
    fn data(&self) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW {
            cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            // NIF_SHOWTIP: version 4 shows the tooltip only when asked to.
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.hicon,
            szTip: self.tip,
            ..Default::default()
        };
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        d
    }

    /// Hands the icon to Explorer; while it refuses, a timer tries again.
    fn show(&mut self) -> bool {
        let d = self.data();
        // SAFETY: `d` is a filled NOTIFYICONDATAW of its full size. A refused
        // add may still have added the icon (Explorer busy at sign-in): then
        // modifying it works.
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &d) != 0 || Shell_NotifyIconW(NIM_MODIFY, &d) != 0 };
        self.shown = ok;
        if ok {
            // Version 4: clicks come as NIN_SELECT and WM_CONTEXTMENU, with
            // the icon's position.
            // SAFETY: as above.
            unsafe { Shell_NotifyIconW(NIM_SETVERSION, &d) };
            if mem::take(&mut self.retrying) {
                // SAFETY: our window and timer.
                unsafe { KillTimer(self.hwnd, RETRY_TIMER) };
                log::info!("tray icon added");
            }
        } else if !self.retrying {
            self.retrying = true;
            log::warn!("Explorer did not take the tray icon; trying again every {RETRY_MS} ms");
            // SAFETY: our window; WM_TIMER comes to its procedure.
            unsafe { SetTimer(self.hwnd, RETRY_TIMER, RETRY_MS, None) };
        }
        ok
    }

    fn hide(&mut self) {
        if mem::take(&mut self.retrying) {
            // SAFETY: our window and timer.
            unsafe { KillTimer(self.hwnd, RETRY_TIMER) };
        }
        if mem::take(&mut self.shown) {
            let d = self.data();
            // SAFETY: `d` is a filled NOTIFYICONDATAW of its full size.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
        }
    }

    /// Makes the icon again from `icons`, at the size wanted now.
    fn remake_icon(&mut self) {
        let old = mem::replace(&mut self.hicon, make_icon(&self.icons));
        if self.shown {
            let mut d = self.data();
            d.uFlags = NIF_ICON;
            // SAFETY: `d` is a filled NOTIFYICONDATAW of its full size.
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
        }
        destroy_icon(old);
    }

    /// A click on the icon, unless it is the second of a double click.
    fn click(&mut self) {
        let now = Instant::now();
        // SAFETY: a plain query (milliseconds).
        let double = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
        if !second_click(self.last_click.take(), now, double) {
            self.last_click = Some(now);
            guard("click", &mut self.on_click);
        }
    }

    fn run_cmds(&mut self) {
        while let Ok(cmd) = self.cmds.try_recv() {
            match cmd {
                Cmd::SetIcon(icons) => {
                    self.icons = icons;
                    self.remake_icon();
                }
                Cmd::Remove => {
                    self.hide();
                    // SAFETY: our window, on its own thread; WM_DESTROY ends the loop.
                    unsafe { DestroyWindow(self.hwnd) };
                    return;
                }
            }
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.hide();
        destroy_icon(self.hicon);
    }
}

/// A message, kept to post it again.
type Msg = (HWND, u32, WPARAM, LPARAM);

/// Runs `f` on this thread's tray. A message that comes while the tray is in
/// use (Windows can deliver one during a Shell call or a callback) is posted
/// again, for later.
fn with_state<R>(m: Msg, f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => state.as_mut().map(f),
        Err(_) => {
            // SAFETY: our window; the message carries no pointers.
            unsafe { PostMessageW(m.0, m.1, m.2, m.3) };
            None
        }
    })
}

/// Runs a callback of the caller's. A panic is logged here: unwinding into
/// Windows would abort the process.
fn guard<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| log::error!("tray: the {what} callback panicked")).ok()
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let m = (hwnd, msg, wparam, lparam);
    match msg {
        // Version 4: the event in the low word of `lparam`, the icon's position in `wparam`.
        WM_TRAY => match (lparam & 0xffff) as u32 {
            NIN_SELECT | NIN_KEYSELECT => {
                with_state(m, State::click);
            }
            WM_CONTEXTMENU => show_menu(m),
            _ => {}
        },
        WM_TRAY_CMD => {
            with_state(m, State::run_cmds);
        }
        WM_TIMER if wparam == RETRY_TIMER => {
            with_state(m, State::show);
        }
        // `taskkill` without `/F` sends this to every window of the process:
        // only `Tray::remove` takes the icon away.
        WM_CLOSE => {}
        // SAFETY: a plain call; ends this thread's message loop.
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            log::info!("Explorer restarted: adding the tray icon again");
            with_state(m, |s| {
                s.shown = false;
                s.remake_icon();
                s.show();
            });
        }
        // SAFETY: the arguments Windows gave this procedure.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

/// Asks for the rows, shows them at the icon and runs the picked one.
fn show_menu(m: Msg) {
    let hwnd = m.0;
    let (x, y) = anchor(m.2);
    let Some(Some(items)) = with_state(m, |s| guard("menu", &mut s.menu)) else { return };
    // SAFETY: a plain call; a null menu is a failure.
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    for row in menu_plan(&items) {
        let label = row.label.as_ref().map_or(ptr::null(), |l| l.as_ptr());
        // SAFETY: our menu; `label` is NUL-terminated and outlives the call.
        unsafe { AppendMenuW(menu, row.flags, row.id, label) };
    }
    // SAFETY: plain queries and calls on our menu and window. Windows closes
    // a tray menu on a click elsewhere only when its window is in front, and
    // the next menu opens properly only after a message once this one closed
    // (KB135788).
    let picked = unsafe {
        SetForegroundWindow(hwnd);
        let align = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 { TPM_RIGHTALIGN } else { TPM_LEFTALIGN };
        let picked = TrackPopupMenuEx(menu, align | TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, x, y, hwnd, ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        picked
    };
    let item = (picked as usize).checked_sub(1).and_then(|i| items.into_iter().nth(i));
    if let Some(MenuItem::Entry { action, .. }) = item {
        guard("menu action", action);
    }
}

/// Whether a click at `now` ends a double click begun at `last`.
fn second_click(last: Option<Instant>, now: Instant, double: Duration) -> bool {
    last.is_some_and(|t| now.duration_since(t) < double)
}

/// The icon's position in a version-4 callback's `wparam`
/// (`GET_X_LPARAM` / `GET_Y_LPARAM`): signed, a screen left of or above the
/// main one has negative coordinates.
fn anchor(wparam: WPARAM) -> (i32, i32) {
    ((wparam & 0xffff) as u16 as i16 as i32, ((wparam >> 16) & 0xffff) as u16 as i16 as i32)
}

/// One `AppendMenuW` call.
#[derive(Debug, PartialEq, Eq)]
struct PlannedRow {
    flags: MENU_ITEM_FLAGS,
    /// The command `TrackPopupMenuEx` returns: the row's index + 1 (0 is
    /// "nothing picked").
    id: usize,
    /// NUL-terminated; `None` for a separator.
    label: Option<Vec<u16>>,
}

/// The menu's `AppendMenuW` calls. `&` is doubled: a single one marks the
/// access key.
fn menu_plan(items: &[MenuItem]) -> Vec<PlannedRow> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| match item {
            MenuItem::Entry { label, .. } => {
                PlannedRow { flags: MF_STRING, id: i + 1, label: Some(label.replace('&', "&&").encode_utf16().chain([0]).collect()) }
            }
            MenuItem::Separator => PlannedRow { flags: MF_SEPARATOR, id: 0, label: None },
        })
        .collect()
}

/// `szTip`: `text` cut to 127 UTF-16 units (never inside a pair), then NUL.
fn tooltip(text: &str) -> [u16; 128] {
    let mut tip = [0u16; 128];
    let mut len = 0;
    for c in text.chars() {
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        if len + units.len() > 127 {
            break;
        }
        tip[len..len + units.len()].copy_from_slice(units);
        len += units.len();
    }
    tip
}

/// The size the notification area draws icons at (`SM_CXSMICON`), for the
/// thread's DPI awareness: call it on the tray's thread.
fn icon_size() -> u32 {
    // SAFETY: plain queries.
    let n = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    if n > 0 {
        n as u32
    } else {
        16
    }
}

/// The picture for `size`: that size, else the next larger one (shrinking
/// looks better than growing), else the largest.
fn pick(icons: &[Icon], size: u32) -> Option<&Icon> {
    icons.iter().filter(|i| i.width >= size).min_by_key(|i| i.width).or_else(|| icons.iter().max_by_key(|i| i.width))
}

/// `CreateIcon`'s two bitmaps for `icon`: the AND mask (1 bit a pixel, set
/// where it is fully transparent, rows padded to 16 bits) and the colours
/// (BGRA; Windows draws a 32-bit icon with its alpha channel).
fn icon_bits(icon: &Icon) -> (Vec<u8>, Vec<u8>) {
    let w = icon.width as usize;
    let stride = w.div_ceil(16) * 2;
    let mut mask = vec![0u8; stride * icon.height as usize];
    let mut color = Vec::with_capacity(icon.rgba.len());
    for (i, p) in icon.rgba.as_chunks::<4>().0.iter().enumerate() {
        color.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        if p[3] == 0 {
            let (x, y) = (i % w, i / w);
            mask[y * stride + x / 8] |= 0x80 >> (x % 8);
        }
    }
    (mask, color)
}

fn create_icon(icon: &Icon) -> HICON {
    let (mask, color) = icon_bits(icon);
    // SAFETY: `mask` and `color` hold the 1-bit and 32-bit bitmaps of a
    // `width × height` icon.
    unsafe { CreateIcon(module(), icon.width as i32, icon.height as i32, 1, 32, mask.as_ptr(), color.as_ptr()) }
}

/// The icon for the notification area now; null when there is no picture.
fn make_icon(icons: &[Icon]) -> HICON {
    let Some(icon) = pick(icons, icon_size()) else { return ptr::null_mut() };
    let hicon = create_icon(icon);
    if hicon.is_null() {
        log::warn!("could not make the tray icon: {}", io::Error::last_os_error());
    }
    hicon
}

fn destroy_icon(hicon: HICON) {
    if !hicon.is_null() {
        // SAFETY: an icon we made, destroyed once.
        unsafe { DestroyIcon(hicon) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    fn square(size: u32) -> Icon {
        Icon::from_rgba(size, size, vec![255; (size * size * 4) as usize]).unwrap()
    }

    #[test]
    fn icons_need_the_right_length() {
        assert!(Icon::from_rgba(2, 2, vec![0; 16]).is_some());
        assert!(Icon::from_rgba(2, 2, vec![0; 15]).is_none());
        assert!(Icon::from_rgba(0, 0, Vec::new()).is_none());
    }

    #[test]
    fn bits_are_bgra_with_a_mask_of_the_transparent_pixels() {
        // Red opaque, then fully transparent green.
        let icon = Icon::from_rgba(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0]).unwrap();
        let (mask, color) = icon_bits(&icon);
        assert_eq!(color, vec![0, 0, 255, 255, 0, 255, 0, 0]);
        // One row of 16 bits: the second pixel is set.
        assert_eq!(mask, vec![0b0100_0000, 0]);
        // 17 pixels a row take two 16-bit words.
        let (mask, _) = icon_bits(&Icon::from_rgba(17, 2, vec![0; 17 * 2 * 4]).unwrap());
        assert_eq!(mask.len(), 8);
        assert_eq!(&mask[..4], &[0xff, 0xff, 0x80, 0]);
    }

    #[test]
    fn the_size_that_fits_is_picked() {
        let icons = [square(16), square(24), square(64)];
        assert_eq!(pick(&icons, 16).unwrap().width, 16);
        assert_eq!(pick(&icons, 20).unwrap().width, 24);
        assert_eq!(pick(&icons, 96).unwrap().width, 64);
        assert!(pick(&[], 16).is_none());
    }

    #[test]
    fn windows_makes_the_icon_at_its_size() {
        let hicon = create_icon(&square(24));
        assert!(!hicon.is_null());
        let mut info: ICONINFO = unsafe { mem::zeroed() };
        assert!(unsafe { GetIconInfo(hicon, &mut info) } != 0);
        let mut bitmap: BITMAP = unsafe { mem::zeroed() };
        unsafe { GetObjectW(info.hbmColor, mem::size_of::<BITMAP>() as i32, (&mut bitmap as *mut BITMAP).cast()) };
        assert_eq!((bitmap.bmWidth, bitmap.bmHeight), (24, 24));
        unsafe {
            DeleteObject(info.hbmColor);
            DeleteObject(info.hbmMask);
        }
        destroy_icon(hicon);
    }

    #[test]
    fn menu_rows_get_commands_and_escaped_labels() {
        let items = [MenuItem::entry("Tom & Jerry", || {}), MenuItem::Separator, MenuItem::entry("Quit", || {})];
        let wide = |s: &str| Some(s.encode_utf16().chain([0]).collect::<Vec<_>>());
        assert_eq!(
            menu_plan(&items),
            vec![
                PlannedRow { flags: MF_STRING, id: 1, label: wide("Tom && Jerry") },
                PlannedRow { flags: MF_SEPARATOR, id: 0, label: None },
                PlannedRow { flags: MF_STRING, id: 3, label: wide("Quit") },
            ]
        );
    }

    #[test]
    fn tooltips_are_cut_between_characters() {
        let tip = tooltip("Heronite");
        assert_eq!(&tip[..9], &"Heronite\0".encode_utf16().collect::<Vec<_>>()[..]);
        // 126 units, then a pair that does not fit in the 127th.
        let tip = tooltip(&format!("{}😀", "a".repeat(126)));
        assert_eq!(tip[125], u16::from(b'a'));
        assert_eq!(tip[126], 0);
        assert_eq!(tip[127], 0);
    }

    #[test]
    fn the_second_click_of_a_double_click_does_not_count() {
        let t = Instant::now();
        let double = Duration::from_millis(500);
        assert!(!second_click(None, t, double));
        assert!(second_click(Some(t), t + Duration::from_millis(200), double));
        assert!(!second_click(Some(t), t + Duration::from_millis(800), double));
    }

    #[test]
    fn anchors_are_signed() {
        assert_eq!(anchor(0x0020_0010), (16, 32));
        // x = -10 on a screen left of the main one.
        assert_eq!(anchor(0x0005_fff6), (-10, 5));
    }
}
