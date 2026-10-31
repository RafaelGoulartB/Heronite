//! The login window on Windows: a plain Win32 window holding a WebView2 view
//! (wry's Windows backend), with the same command line, output and exit
//! statuses as the WebKitGTK window of Linux.
//!
//! * The WebView2 runtime is looked for first; without it the helper exits
//!   with [`NO_WEBVIEW`] and the app points the user to it or to the browser
//!   method.
//! * WebView2 keeps its files in a user data folder, by default next to the
//!   exe (read-only in an installed copy). It goes under Heronite's cache
//!   folder instead (inside the folder of a portable copy). The view is
//!   InPrivate like the Linux one is incognito: no cookie or session is kept
//!   from one login to the next.
//! * GOG and Amazon get WebView2's own user agent, the one of Microsoft Edge
//!   on Windows (they refuse unknown web views); Epic gets its launcher's.
//! * The window closes with the program that started it (the app quitting),
//!   as `PR_SET_PDEATHSIG` makes it on Linux.

use std::cell::RefCell;
use std::io::Write;
use std::num::NonZeroIsize;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use windows_sys::core::{w, PCWSTR};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM, LRESULT, RECT, WAIT_OBJECT_0, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, COLOR_WINDOW, HBRUSH, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, LoadCursorW, LoadImageW,
    MsgWaitForMultipleObjectsEx, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassExW, SetForegroundWindow, SetTimer,
    SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, IDC_ARROW, IMAGE_ICON, LR_DEFAULTCOLOR, MINMAXINFO, MSG,
    MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_SHOWNORMAL, WM_APP, WM_CLOSE, WM_DPICHANGED, WM_GETMINMAXINFO, WM_QUIT, WM_TIMER, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{PageLoadEvent, WebContext, WebView, WebViewBuilder};

use super::{code_from_page_text, parse_args, Store, CLOSED, EPIC_USER_AGENT};

/// Exit status when the WebView2 runtime is not installed (the app tells the
/// user to install it or use the browser method).
const NO_WEBVIEW: i32 = 5;
/// Exit status for a bad command line or a window that could not be made.
const FAILED: i32 = 2;

/// A code arrived: the window procedure quits the message loop with 0.
const WM_LOGIN_DONE: u32 = WM_APP + 1;
/// `--smoke`: how long the window stays open.
const SMOKE_TIMER: usize = 1;
const SMOKE_MS: u32 = 15_000;

/// The window's client area at 100 % scale; the smallest it can be resized to.
const SIZE: (i32, i32) = (520, 720);
const MIN_SIZE: (i32, i32) = (360, 420);

const SMOKE_PAGE: &str = "<html><body style='font-family:\"Segoe UI\",sans-serif;background:#222;color:#eee'><h2>Login helper</h2><p>Smoke test page.</p></body></html>";

/// The user agent of `store`'s page; `None` keeps WebView2's own (Microsoft
/// Edge's, with the matching client hints).
fn user_agent(store: Store) -> Option<&'static str> {
    match store {
        Store::Epic => Some(EPIC_USER_AGENT),
        Store::Gog | Store::Amazon => None,
    }
}

pub(crate) fn main() -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--smoke");
    let (store, start_url) = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(FAILED);
        }
    };
    match wry::webview_version() {
        Ok(version) => eprintln!("WebView2 runtime {version}"),
        Err(e) => {
            eprintln!("the Microsoft Edge WebView2 Runtime is not installed ({e})");
            std::process::exit(NO_WEBVIEW);
        }
    }
    let parent = Parent::watch();

    let hwnd = match create_window(store.title()) {
        Ok(hwnd) => hwnd,
        Err(e) => {
            eprintln!("could not create the window: {e}");
            std::process::exit(FAILED);
        }
    };
    let data_dir = data_folder();
    let mut context = WebContext::new(Some(data_dir));

    let (tx, rx) = mpsc::channel::<String>();
    // A code: keep it and tell the loop (from any thread; handles are not
    // `Send`, their value is).
    let hwnd_value = hwnd as isize;
    let done = move |code: String| {
        let _ = tx.send(code);
        // SAFETY: posting to a window of this process; a stale handle only fails.
        unsafe { PostMessageW(hwnd_value as HWND, WM_LOGIN_DONE, 0, 0) };
    };
    let view: Rc<RefCell<Option<WebView>>> = Rc::default();

    let done_nav = done.clone();
    let mut builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_incognito(true)
        // The page that carries the code is not loaded: take the code and stop there.
        .with_navigation_handler(move |url| match store.code_from_url(&url) {
            Some(code) => {
                done_nav(code);
                false
            }
            None => true,
        });
    if let Some(agent) = user_agent(store) {
        builder = builder.with_user_agent(agent);
    }
    builder = if smoke { builder.with_html(SMOKE_PAGE) } else { builder.with_url(&start_url) };
    let page_view = view.clone();
    let built = builder
        .with_on_page_load_handler(move |event, url| {
            if !matches!(event, PageLoadEvent::Finished) {
                return;
            }
            if smoke {
                println!("SMOKE-READY");
                let _ = std::io::stdout().flush();
                return;
            }
            if let Some(code) = store.code_from_url(&url) {
                done(code);
                return;
            }
            // Epic's JSON page of /id/api/redirect shows the code as text
            // (in a <pre> of its own when Edge's JSON viewer formats it).
            if store == Store::Epic && url.contains("/id/api/redirect") {
                if let Some(view) = page_view.borrow().as_ref() {
                    let done = done.clone();
                    let _ = view.evaluate_script_with_callback(
                        "(function(){var p=document.querySelector('pre');return p?p.innerText:(document.body?document.body.innerText:'');})()",
                        move |text| {
                            if let Some(code) = code_from_page_text(&text).or_else(|| json_in(&text).as_deref().and_then(code_from_page_text)) {
                                done(code);
                            }
                        },
                    );
                }
            }
        })
        .build(&Window(hwnd));
    match built {
        Ok(webview) => *view.borrow_mut() = Some(webview),
        Err(e) => {
            eprintln!("could not create the web view: {e}");
            // SAFETY: our window, destroyed once.
            unsafe { DestroyWindow(hwnd) };
            std::process::exit(FAILED);
        }
    }

    // SAFETY: our window.
    unsafe {
        ShowWindow(hwnd, SW_SHOWNORMAL);
        // Allowed: the app that started it is in the foreground.
        SetForegroundWindow(hwnd);
        if smoke {
            SetTimer(hwnd, SMOKE_TIMER, SMOKE_MS, None);
        }
    }

    let status = run_loop(parent.as_ref().map(|p| p.0));
    let code = rx.try_recv().ok();
    // The view first (it closes its WebView2 processes), then the window.
    drop(view.borrow_mut().take());
    // SAFETY: our window, destroyed once.
    unsafe { DestroyWindow(hwnd) };
    drop(parent);
    if status == 0 {
        if let Some(code) = code {
            println!("{code}");
            let _ = std::io::stdout().flush();
        }
    }
    std::process::exit(status);
}

/// The JSON object in `text` (Edge's JSON viewer may add text around it),
/// for [`code_from_page_text`].
fn json_in(text: &str) -> Option<String> {
    let text = serde_json::from_str::<String>(text).unwrap_or_else(|_| text.to_string());
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    (start < end).then(|| text[start..=end].to_string())
}

/// Runs the window's messages until the loop is told to quit (its status:
/// 0 with a code, [`CLOSED`] when the user closed the window) or the
/// program that started the helper exits ([`CLOSED`] too).
fn run_loop(parent: Option<HANDLE>) -> i32 {
    let handles: Vec<HANDLE> = parent.into_iter().collect();
    loop {
        // SAFETY: the handles stay open while we wait.
        let woke =
            unsafe { MsgWaitForMultipleObjectsEx(handles.len() as u32, handles.as_ptr(), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        if !handles.is_empty() && woke == WAIT_OBJECT_0 {
            eprintln!("the program that opened the login window exited");
            return CLOSED;
        }
        // SAFETY: zeroed is a valid MSG.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: `msg` is ours to fill.
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                return msg.wParam as i32;
            }
            // SAFETY: a message we just took from the queue.
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // The window is destroyed after the loop, once the view is gone.
        WM_CLOSE => {
            PostQuitMessage(CLOSED);
            0
        }
        WM_LOGIN_DONE => {
            PostQuitMessage(0);
            0
        }
        WM_TIMER if wparam == SMOKE_TIMER => {
            PostQuitMessage(0);
            0
        }
        // Moved to a monitor of another scale: take the size Windows suggests.
        WM_DPICHANGED => {
            let r = &*(lparam as *const RECT);
            SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            0
        }
        WM_GETMINMAXINFO => {
            let dpi = GetDpiForWindow(hwnd);
            if dpi != 0 {
                let (w, h) = outer_size(MIN_SIZE, dpi);
                let info = &mut *(lparam as *mut MINMAXINFO);
                info.ptMinTrackSize.x = w;
                info.ptMinTrackSize.y = h;
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// The window size for a client area of `size` (at 100 %) at `dpi`.
fn outer_size(size: (i32, i32), dpi: u32) -> (i32, i32) {
    let scale = |v: i32| (v as i64 * dpi as i64 / 96) as i32;
    let mut rect = RECT { left: 0, top: 0, right: scale(size.0), bottom: scale(size.1) };
    // SAFETY: `rect` is ours.
    unsafe { AdjustWindowRectExForDpi(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0, dpi) };
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// The top-level window, hidden, centred on the monitor of the window in the
/// foreground (the app's) and sized for that monitor's scale.
fn create_window(title: &str) -> Result<HWND, std::io::Error> {
    // SAFETY: plain Win32 calls on values we own; failures are checked.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut dpi_y) = (96, 96);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) != 0 {
            dpi = 96;
        }
        let work = info.rcWork;
        let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
        let (w, h) = outer_size(SIZE, dpi);
        let (w, h) = if work_w > 0 && work_h > 0 { (w.min(work_w), h.min(work_h)) } else { (w, h) };
        let (x, y) = (work.left + (work_w - w).max(0) / 2, work.top + (work_h - h).max(0) / 2);

        // Heronite's icon (resource 1, see build.rs), at the sizes of this scale.
        let icon = |cx, cy| {
            LoadImageW(instance, 1 as PCWSTR, IMAGE_ICON, GetSystemMetricsForDpi(cx, dpi), GetSystemMetricsForDpi(cy, dpi), LR_DEFAULTCOLOR)
        };
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: icon(SM_CXICON, SM_CYICON),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
            lpszMenuName: std::ptr::null(),
            lpszClassName: w!("HeroniteLoginWindow"),
            hIconSm: icon(SM_CXSMICON, SM_CYSMICON),
        };
        if RegisterClassExW(&class) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        let hwnd = CreateWindowExW(
            0,
            class.lpszClassName,
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        if apps_use_dark_theme() {
            let on: i32 = 1;
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&on as *const i32).cast(), 4);
        }
        Ok(hwnd)
    }
}

/// Whether Windows is set to dark mode for apps (the title bar follows it).
fn apps_use_dark_theme() -> bool {
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: a DWORD read into `value`, whose size `size` gives.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    err == 0 && value == 0
}

/// WebView2's user data folder: `login-webview` in Heronite's cache folder
/// (`%LOCALAPPDATA%\heronite\cache`, or `cache` of a portable copy), else in
/// the user's temp folder.
fn data_folder() -> PathBuf {
    let cache = heronite_platform::dirs::cache("heronite").map(|d| d.join("login-webview"));
    let temp = heronite_platform::dirs::user_temp("heronite-login-webview");
    cache.into_iter().find(|d| std::fs::create_dir_all(d).is_ok()).unwrap_or(temp)
}

/// The top-level window, for wry.
struct Window(HWND);

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
        // SAFETY: the window outlives the view (destroyed after it).
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// The program that started the helper (the app), to close with it.
struct Parent(HANDLE);

impl Parent {
    /// The parent process, unless it is already gone (its id may then
    /// belong to a newer process: one started after us is not our parent).
    fn watch() -> Option<Self> {
        let pid = parent_pid()?;
        // SAFETY: a handle we close in `drop`; null is a failure.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let parent = Self(handle);
        // SAFETY: the pseudo handle of this process and an open one.
        let (ours, theirs) = unsafe { (started(GetCurrentProcess()), started(parent.0)) };
        match (ours, theirs) {
            (Some(ours), Some(theirs)) if theirs <= ours => Some(parent),
            _ => None,
        }
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        // SAFETY: our handle, closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// When `process` started (100 ns since 1601).
unsafe fn started(process: HANDLE) -> Option<u64> {
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    if GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
        return None;
    }
    Some((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
}

/// The id of the process that started this one.
fn parent_pid() -> Option<u32> {
    // SAFETY: a snapshot handle we close; the entry is sized for the API.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return None;
        }
        let me = GetCurrentProcessId();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            if entry.th32ProcessID == me {
                found = Some(entry.th32ParentProcessID);
                break;
            }
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        found.filter(|&pid| pid != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_found_in_viewer_text() {
        let page = r#"Pretty-print {"redirectUrl":"https://localhost/launcher/authorized?code=x","authorizationCode":"thecode"}"#;
        assert_eq!(json_in(page).as_deref().and_then(code_from_page_text).as_deref(), Some("thecode"));
        let encoded = serde_json::to_string(page).unwrap();
        assert_eq!(json_in(&encoded).as_deref().and_then(code_from_page_text).as_deref(), Some("thecode"));
        assert_eq!(json_in("no json here"), None);
    }

    #[test]
    fn the_window_grows_with_the_scale() {
        let (w100, h100) = outer_size(SIZE, 96);
        let (w150, h150) = outer_size(SIZE, 144);
        assert!(w100 >= SIZE.0 && h100 >= SIZE.1);
        assert!(w150 >= SIZE.0 * 3 / 2 && h150 >= SIZE.1 * 3 / 2);
    }
}
