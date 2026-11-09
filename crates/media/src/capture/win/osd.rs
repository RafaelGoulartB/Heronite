//! The on-screen notice after a capture, Windows' counterpart of Omarchy's
//! OSD on Hyprland: a small dark box in the bottom right corner of the
//! captured monitor for two seconds.
//!
//! It never takes the focus from the game (`WS_EX_NOACTIVATE`, shown with
//! `SW_SHOWNOACTIVATE`), the mouse goes through it (`WS_EX_TRANSPARENT`), it
//! is not in the taskbar or Alt+Tab (`WS_EX_TOOLWINDOW`), and it is left out
//! of captures (`WDA_EXCLUDEFROMCAPTURE`), so a second screenshot taken
//! while it shows does not contain it. Drawn with GDI at the monitor's scale.

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetMonitorInfoW,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_TOP, FF_SWISS, FW_NORMAL, FW_SEMIBOLD, HFONT, HMONITOR, MONITORINFO, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetLayeredWindowAttributes, SetTimer, SetWindowDisplayAffinity, SetWindowPos, ShowWindow, HWND_TOPMOST, LWA_ALPHA,
    MA_NOACTIVATE, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WM_CLOSE, WM_DESTROY,
    WM_MOUSEACTIVATE, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};

use super::screen::PerMonitorDpi;

const CLASS: PCWSTR = w!("HeroniteCaptureNotice");
const SHOWN_FOR: Duration = Duration::from_secs(2);
const BACKGROUND: COLORREF = rgb(24, 24, 28);
const ACCENT: COLORREF = rgb(88, 166, 255);
const TITLE: COLORREF = rgb(245, 245, 247);
const DETAIL: COLORREF = rgb(190, 190, 198);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

/// The notice on screen now (its window), closed when the next one shows.
static CURRENT: AtomicIsize = AtomicIsize::new(0);

struct Content {
    title: Vec<u16>,
    detail: Vec<u16>,
    /// Monitor scale (1.0 at 96 DPI).
    scale: f32,
}

thread_local! {
    /// What the notice of this thread shows (one notice per thread).
    static CONTENT: RefCell<Option<Content>> = const { RefCell::new(None) };
}

/// Shows `title` and `detail` on the monitor `monitor` (an `HMONITOR`).
/// False when the notice could not be shown.
pub fn show(monitor: isize, title: &str, detail: &str) -> bool {
    let (shown, result) = mpsc::channel();
    let (title, detail) = (title.to_owned(), detail.to_owned());
    let spawned = std::thread::Builder::new().name("heronite-capture-notice".into()).spawn(move || run(monitor, &title, &detail, shown));
    spawned.is_ok()
        && result.recv_timeout(Duration::from_secs(1)).unwrap_or_else(|_| {
            log::warn!("the screenshot notice did not show in time");
            false
        })
}

fn s(v: f32, scale: f32) -> i32 {
    (v * scale).round() as i32
}

fn run(monitor: isize, title: &str, detail: &str, shown: mpsc::Sender<bool>) {
    let _dpi = PerMonitorDpi::enter();
    let fail = |why: String| {
        log::warn!("the screenshot notice could not be shown: {why}");
        let _ = shown.send(false);
    };
    let Some(class) = register_class() else { return fail("RegisterClassExW failed".into()) };
    let monitor = HMONITOR(monitor as *mut _);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: `info` has its size set; a stale monitor handle just fails.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return fail("GetMonitorInfoW failed".into());
    }
    let (mut dpi_x, mut dpi_y) = (96, 96);
    // SAFETY: out pointers to locals.
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let scale = dpi_x.max(96) as f32 / 96.;
    let (w, h, margin) = (s(360., scale), s(68., scale), s(24., scale));
    let work = info.rcWork;
    let (x, y) = (work.right - w - margin, work.bottom - h - margin);
    CONTENT.with(|c| {
        *c.borrow_mut() = Some(Content { title: title.encode_utf16().collect(), detail: detail.encode_utf16().collect(), scale })
    });
    // SAFETY: a window of the class registered above, owned by this thread,
    // which runs its message loop until it is destroyed.
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(Into::into);
        let style = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT;
        let hwnd = match CreateWindowExW(style, class, w!("Heronite"), WS_POPUP, x, y, w, h, None, None, instance, None) {
            Ok(hwnd) => hwnd,
            Err(e) => return fail(format!("CreateWindowExW: {e}")),
        };
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 240, LWA_ALPHA);
        // Windows 10 2004 and later; older ones keep it in captures.
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        let corner = s(14., scale);
        SetWindowRgn(hwnd, Some(CreateRoundRectRgn(0, 0, w + 1, h + 1, corner, corner)), false);
        let previous = CURRENT.swap(hwnd.0 as isize, Ordering::SeqCst);
        if previous != 0 {
            let _ = PostMessageW(Some(HWND(previous as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        SetTimer(Some(hwnd), 1, SHOWN_FOR.as_millis() as u32, None);
        log::debug!("screenshot notice shown at {x},{y} ({w}x{h})");
        let _ = shown.send(true);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            DispatchMessageW(&msg);
        }
        let _ = CURRENT.compare_exchange(hwnd.0 as isize, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// The window class, registered once per process.
fn register_class() -> Option<PCWSTR> {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    let ok = *REGISTERED.get_or_init(|| {
        // SAFETY: a class description with a static name and our procedure.
        unsafe {
            let Ok(instance) = GetModuleHandleW(None) else { return false };
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassExW(&class) != 0
        }
    });
    ok.then_some(CLASS)
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY (whole function): called by Windows on this window's thread with
    // a live window handle.
    unsafe {
        match msg {
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_TIMER | WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                log::debug!("screenshot notice closed");
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn font(height: i32, weight: i32) -> HFONT {
    // SAFETY: a font description; deleted by the caller.
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            FF_SWISS.0 as u32,
            w!("Segoe UI"),
        )
    }
}

fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: painting our own window between BeginPaint and EndPaint; every
    // GDI object made here is deleted before returning.
    unsafe {
        let dc = BeginPaint(hwnd, &mut ps);
        CONTENT.with(|c| {
            let content = c.borrow();
            let Some(content) = content.as_ref() else { return };
            let scale = content.scale;
            let mut area = RECT::default();
            let _ = GetClientRect(hwnd, &mut area);
            let background = CreateSolidBrush(BACKGROUND);
            FillRect(dc, &area, background);
            let accent = CreateSolidBrush(ACCENT);
            let bar = RECT { left: 0, top: 0, right: s(5., scale), bottom: area.bottom };
            FillRect(dc, &bar, accent);
            SetBkMode(dc, TRANSPARENT);
            let left = s(20., scale);
            let right = area.right - s(16., scale);
            let flags = DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;
            let title_font = font(s(15., scale), FW_SEMIBOLD.0 as i32);
            let old = SelectObject(dc, title_font.into());
            SetTextColor(dc, TITLE);
            let mut r = RECT { left, top: s(13., scale), right, bottom: s(36., scale) };
            let mut title = content.title.clone();
            DrawTextW(dc, &mut title, &mut r, flags);
            let detail_font = font(s(13., scale), FW_NORMAL.0 as i32);
            SelectObject(dc, detail_font.into());
            SetTextColor(dc, DETAIL);
            let mut r = RECT { left, top: s(37., scale), right, bottom: area.bottom - s(8., scale) };
            let mut detail = content.detail.clone();
            DrawTextW(dc, &mut detail, &mut r, flags);
            SelectObject(dc, old);
            let _ = DeleteObject(title_font.into());
            let _ = DeleteObject(detail_font.into());
            let _ = DeleteObject(background.into());
            let _ = DeleteObject(accent.into());
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetForegroundWindow, IsWindowVisible};

    use super::*;

    /// The notice shows on the primary monitor, leaves the focus where it
    /// was and goes away by itself.
    #[test]
    fn the_notice_shows_and_goes() {
        if !super::super::screen::has_output() {
            eprintln!("no desktop here: no notice to show");
            return;
        }
        // SAFETY: plain queries.
        let (monitor, focus) = unsafe { (MonitorFromPoint(Default::default(), MONITOR_DEFAULTTOPRIMARY), GetForegroundWindow()) };
        assert!(show(monitor.0 as isize, "Screenshot saved", "Test"));
        // SAFETY: plain queries.
        unsafe {
            let notice = FindWindowW(CLASS, None).expect("the notice window");
            assert!(IsWindowVisible(notice).as_bool());
            assert_eq!(GetForegroundWindow(), focus);
        }
        std::thread::sleep(SHOWN_FOR + Duration::from_millis(700));
        // SAFETY: as above.
        assert!(unsafe { FindWindowW(CLASS, None) }.is_err());
    }
}
