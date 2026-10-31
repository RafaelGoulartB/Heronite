//! Which monitor to capture and how: DXGI Desktop Duplication, else GDI.
//!
//! # The monitor
//! The one showing the game: a visible window of a process in the job of a
//! game this app launched (`heronite_platform::process::in_tracked_group`),
//! the focused one first. A game started through a store client (Steam
//! starts it, not us) has no such window, so then the focused window's
//! monitor, unless that window is this app's or the desktop's. Else the
//! monitor under the pointer.
//!
//! # Desktop Duplication
//! It copies what the monitor shows from the compositor, on the GPU that
//! drives it: windowed, borderless and flip-model fullscreen games (what
//! DirectX 12 and most DirectX 11 games use, "fullscreen optimisations"
//! included) come out right, where GDI's `BitBlt` often gets black or an old
//! frame. It draws no border and no pointer, needs no permission, and works
//! since Windows 8. Windows.Graphics.Capture would do as well for a monitor,
//! but it needs WinRT, shows a yellow border before Windows 11 and adds the
//! pointer unless asked not to. Not covered by either: a game in legacy
//! exclusive fullscreen (the duplication is lost while it holds the display)
//! and protected video, which comes out black.
//!
//! On an HDR monitor the frame is asked for in scRGB half floats and turned
//! into 8-bit sRGB in [`super::pixels`] (see there). Coordinates and sizes
//! are physical pixels: the capture thread is per-monitor DPI aware, so a
//! monitor at 150% is captured at its full resolution.
//!
//! GDI is the fallback (Remote Desktop, a driver without duplication).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::core::{Interface, BOOL};
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270, DXGI_MODE_ROTATION_ROTATE90,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5, IDXGIOutput6, IDXGIOutputDuplication,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, MonitorFromPoint,
    MonitorFromWindow, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
};

use super::pixels::{self, Format, Frame, Rgb};

/// How a capture was made (for the logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Duplication,
    Gdi,
}

/// A captured monitor.
pub struct Shot {
    pub image: Rgb,
    /// The `HMONITOR` captured (the on-screen notice goes there).
    pub monitor: isize,
    pub method: Method,
}

/// The thread is per-monitor DPI aware while this lives: physical pixels
/// everywhere, whatever the scale of each monitor.
pub struct PerMonitorDpi(DPI_AWARENESS_CONTEXT);

impl PerMonitorDpi {
    pub fn enter() -> Self {
        // SAFETY: changes this thread's awareness; restored when dropped.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for PerMonitorDpi {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the context this thread had before.
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

/// Captures the monitor the game is on (see the module docs).
pub fn capture_game_monitor() -> Result<Shot, String> {
    let _dpi = PerMonitorDpi::enter();
    capture_monitor(game_monitor())
}

/// Captures the primary monitor.
#[cfg(test)]
pub fn capture_primary_monitor() -> Result<Shot, String> {
    let _dpi = PerMonitorDpi::enter();
    // SAFETY: no preconditions.
    capture_monitor(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
}

/// Whether a graphics adapter shows a desktop on some monitor (a CI machine
/// or a service may have none).
#[cfg(test)]
pub fn has_output() -> bool {
    // SAFETY: plain DXGI enumeration.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return false };
        (0..).map_while(|a| factory.EnumAdapters1(a).ok()).any(|adapter| {
            (0..).map_while(|o| adapter.EnumOutputs(o).ok()).any(|o| o.GetDesc().is_ok_and(|d| d.AttachedToDesktop.as_bool()))
        })
    }
}

fn capture_monitor(monitor: HMONITOR) -> Result<Shot, String> {
    let raw = monitor.0 as isize;
    match duplicate(monitor) {
        Ok(image) => Ok(Shot { image, monitor: raw, method: Method::Duplication }),
        Err(dup) => {
            log::warn!("Desktop Duplication failed ({dup}); capturing with GDI");
            match gdi(monitor) {
                Ok(image) => Ok(Shot { image, monitor: raw, method: Method::Gdi }),
                Err(gdi) => Err(format!("Desktop Duplication: {dup}; GDI: {gdi}")),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The monitor

/// The monitor of the game's window, else the focused window's, else the
/// pointer's.
fn game_monitor() -> HMONITOR {
    let own = std::process::id();
    // SAFETY: no preconditions.
    let foreground = unsafe { GetForegroundWindow() };
    let windows = top_level_windows();
    let mut in_game: HashMap<u32, bool> = HashMap::new();
    let mut is_game = |pid: u32| *in_game.entry(pid).or_insert_with(|| pid != own && heronite_platform::process::in_tracked_group(pid));
    let game = windows
        .iter()
        .find(|(hwnd, pid)| *hwnd == foreground && is_game(*pid))
        .or_else(|| windows.iter().find(|(_, pid)| is_game(*pid)))
        .map(|(hwnd, _)| *hwnd);
    if let Some(hwnd) = game {
        log::info!("capturing the monitor of the game's window");
        // SAFETY: any window handle; a gone window gives the nearest monitor.
        return unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    }
    if let Some((hwnd, _)) = windows.iter().find(|(hwnd, pid)| *hwnd == foreground && *pid != own && !is_shell(*hwnd)) {
        log::info!("capturing the monitor of the focused window");
        // SAFETY: as above.
        return unsafe { MonitorFromWindow(*hwnd, MONITOR_DEFAULTTONEAREST) };
    }
    let mut pointer = POINT::default();
    // SAFETY: `pointer` is a valid POINT; on failure (0, 0) means the primary.
    unsafe {
        let _ = GetCursorPos(&mut pointer);
        MonitorFromPoint(pointer, MONITOR_DEFAULTTOPRIMARY)
    }
}

/// The visible, unminimised, uncloaked top-level windows that have an area
/// and are not tool windows, front to back, with their process.
fn top_level_windows() -> Vec<(HWND, u32)> {
    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the Vec below, alive for the whole EnumWindows.
        let found = unsafe { &mut *(found.0 as *mut Vec<(HWND, u32)>) };
        if is_candidate(hwnd) {
            let mut pid = 0;
            // SAFETY: `pid` receives the id.
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            found.push((hwnd, pid));
        }
        BOOL(1)
    }
    let mut found: Vec<(HWND, u32)> = Vec::new();
    // SAFETY: the callback only touches `found`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut Vec<(HWND, u32)> as isize)) };
    found
}

fn is_candidate(hwnd: HWND) -> bool {
    // SAFETY: queries of a window handle EnumWindows gave; each tolerates a
    // window that went away meanwhile.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return false;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        let size = std::mem::size_of::<u32>() as u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), size).is_ok() && cloaked != 0 {
            return false;
        }
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).is_ok() && r.right > r.left && r.bottom > r.top
    }
}

/// The desktop and the taskbar.
fn is_shell(hwnd: HWND) -> bool {
    let mut name = [0u16; 64];
    // SAFETY: writes at most `name.len()` units.
    let n = unsafe { GetClassNameW(hwnd, &mut name) }.max(0) as usize;
    let class = String::from_utf16_lossy(&name[..n]);
    matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

// ---------------------------------------------------------------------------
// Desktop Duplication

/// The output (and its adapter) that shows `monitor`.
fn find_output(monitor: HMONITOR) -> Result<(IDXGIAdapter1, IDXGIOutput, DXGI_OUTPUT_DESC), String> {
    // SAFETY: plain DXGI enumeration.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
        for adapter in (0..).map_while(|a| factory.EnumAdapters1(a).ok()) {
            for output in (0..).map_while(|o| adapter.EnumOutputs(o).ok()) {
                if let Ok(desc) = output.GetDesc() {
                    if desc.Monitor == monitor {
                        return Ok((adapter, output, desc));
                    }
                }
            }
        }
    }
    Err("no graphics adapter shows this monitor".into())
}

fn duplicate(monitor: HMONITOR) -> Result<Rgb, String> {
    let (adapter, output, desc) = find_output(monitor)?;
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    // SAFETY: out pointers to locals; the adapter is the monitor's.
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
    let (device, context) = device.zip(context).ok_or("D3D11CreateDevice gave no device")?;
    // SAFETY: COM calls on live interfaces.
    let hdr = unsafe { output.cast::<IDXGIOutput6>().ok().and_then(|o| o.GetDesc1().ok()) }
        .is_some_and(|d| d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020);
    let duplication = open_duplication(&output, &device, hdr)?;
    // SAFETY: COM call on a live interface.
    let rotation = unsafe { duplication.GetDesc() }.Rotation;
    let (staging, tex) = next_frame(&duplication, &device, &context)?;
    let format = match tex.Format {
        DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Format::Bgra8,
        DXGI_FORMAT_R16G16B16A16_FLOAT => Format::Rgba16Float,
        DXGI_FORMAT(other) => return Err(format!("unexpected desktop format {other}")),
    };
    let white = if format == Format::Rgba16Float { sdr_white_level(&desc.DeviceName).unwrap_or(1.) } else { 1. };
    if format == Format::Rgba16Float {
        log::info!("HDR monitor: tone mapping the capture (SDR white {:.0} nits)", white * 80.);
    }
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: a CPU-readable staging texture of this device; unmapped below.
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.map_err(|e| format!("Map: {e}"))?;
    let pitch = mapped.RowPitch as usize;
    let bytes = pitch * tex.Height as usize;
    // SAFETY: the mapping holds `RowPitch * Height` bytes until Unmap.
    let data = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, bytes) };
    let image = pixels::to_rgb(&Frame { data, width: tex.Width, height: tex.Height, pitch, format }, white);
    // SAFETY: mapped above.
    unsafe { context.Unmap(&staging, 0) };
    let turns = match rotation {
        DXGI_MODE_ROTATION_ROTATE90 => 1,
        DXGI_MODE_ROTATION_ROTATE180 => 2,
        DXGI_MODE_ROTATION_ROTATE270 => 3,
        _ => 0,
    };
    Ok(pixels::rotate(image?, turns))
}

/// The duplication, in scRGB on an HDR monitor. `DuplicateOutput1` (Windows
/// 10 1703) chooses the format; the older `DuplicateOutput` is the fallback.
fn open_duplication(output: &IDXGIOutput, device: &ID3D11Device, hdr: bool) -> Result<IDXGIOutputDuplication, String> {
    let formats: &[DXGI_FORMAT] =
        if hdr { &[DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_B8G8R8A8_UNORM] } else { &[DXGI_FORMAT_B8G8R8A8_UNORM] };
    // SAFETY: COM calls on live interfaces.
    unsafe {
        let first = match output.cast::<IDXGIOutput5>() {
            Ok(o5) => match o5.DuplicateOutput1(device, 0, formats) {
                Ok(d) => return Ok(d),
                Err(e) => format!("DuplicateOutput1: {e}"),
            },
            Err(_) => "no IDXGIOutput5".to_owned(),
        };
        let o1: IDXGIOutput1 = output.cast().map_err(|e| format!("{first}; IDXGIOutput1: {e}"))?;
        o1.DuplicateOutput(device).map_err(|e| format!("{first}; DuplicateOutput: {e}"))
    }
}

/// Copies the next desktop image into a staging texture. The first frame
/// after the duplication starts holds the whole desktop; one whose
/// `LastPresentTime` is 0 may not be complete, so a later one is waited for
/// (briefly: a running game presents all the time).
fn next_frame(
    duplication: &IDXGIOutputDuplication,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
) -> Result<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC), String> {
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut copied: Option<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC)> = None;
    loop {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        // SAFETY: out pointers to locals; every acquired frame is released.
        match unsafe { duplication.AcquireNextFrame(100, &mut info, &mut resource) } {
            Ok(()) => {
                let copy = resource.ok_or_else(|| "no desktop image".to_owned()).and_then(|r| copy_frame(&r, device, context, &mut copied));
                // SAFETY: a frame was acquired above.
                let _ = unsafe { duplication.ReleaseFrame() };
                copy?;
                if info.LastPresentTime != 0 {
                    break;
                }
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
            Err(e) => return Err(format!("AcquireNextFrame: {e}")),
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    copied.ok_or_else(|| "the desktop gave no image in time".into())
}

fn copy_frame(
    resource: &windows::Win32::Graphics::Dxgi::IDXGIResource,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    copied: &mut Option<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC)>,
) -> Result<(), String> {
    // SAFETY: COM calls on live interfaces; the descriptor is a local.
    unsafe {
        let texture: ID3D11Texture2D = resource.cast().map_err(|e| format!("desktop texture: {e}"))?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        if copied.as_ref().is_none_or(|(_, d)| d.Width != desc.Width || d.Height != desc.Height || d.Format != desc.Format) {
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: desc.Width,
                Height: desc.Height,
                MipLevels: 1,
                ArraySize: 1,
                Format: desc.Format,
                SampleDesc: desc.SampleDesc,
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            device.CreateTexture2D(&staging_desc, None, Some(&mut staging)).map_err(|e| format!("CreateTexture2D: {e}"))?;
            *copied = Some((staging.ok_or("CreateTexture2D gave no texture")?, desc));
        }
        let (staging, _) = copied.as_ref().expect("set above");
        context.CopyResource(staging, &texture);
    }
    Ok(())
}

/// The SDR white level of the display `\\.\DISPLAYn`, in scRGB units (1.0 is
/// 80 nits): where Windows puts SDR white on that HDR monitor.
fn sdr_white_level(gdi_name: &[u16; 32]) -> Option<f32> {
    let name_len = gdi_name.iter().position(|&c| c == 0).unwrap_or(gdi_name.len());
    let (mut n_paths, mut n_modes) = (0u32, 0u32);
    // SAFETY: sizes into locals, then arrays of those sizes.
    unsafe {
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) != ERROR_SUCCESS {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, paths.as_mut_ptr(), &mut n_modes, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return None;
        }
        for path in &paths[..n_paths as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let len = source.viewGdiDeviceName.iter().position(|&c| c == 0).unwrap_or(32);
            if source.viewGdiDeviceName[..len] != gdi_name[..name_len] {
                continue;
            }
            let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL::default();
            white.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
            white.header.size = std::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32;
            white.header.adapterId = path.targetInfo.adapterId;
            white.header.id = path.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut white.header) != 0 || white.SDRWhiteLevel == 0 {
                return None;
            }
            // 1000 is 80 nits.
            return Some(white.SDRWhiteLevel as f32 / 1000.);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// GDI

fn gdi(monitor: HMONITOR) -> Result<Rgb, String> {
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: `info` has its size set.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Err("GetMonitorInfoW failed".into());
    }
    let r = info.rcMonitor;
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        return Err("the monitor has no area".into());
    }
    let mut data = vec![0u8; w as usize * h as usize * 4];
    // SAFETY: GDI objects created here and released before returning; the
    // bitmap info asks for top-down 32-bit rows, which `data` holds.
    let lines = unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC failed".into());
        }
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(memory, bitmap.into());
        let blit = BitBlt(memory, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY | CAPTUREBLT);
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        SelectObject(memory, old);
        let lines =
            if blit.is_ok() { GetDIBits(memory, bitmap, 0, h as u32, Some(data.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS) } else { 0 };
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        blit.map_err(|e| format!("BitBlt: {e}"))?;
        lines
    };
    if lines != h {
        return Err("GetDIBits failed".into());
    }
    pixels::to_rgb(&Frame { data: &data, width: w as u32, height: h as u32, pitch: w as usize * 4, format: Format::Bgra8 }, 1.)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real capture of the primary monitor, where there is one (not on a
    /// machine without a desktop, like a service or some CI runners).
    #[test]
    fn captures_the_primary_monitor() {
        if !has_output() {
            eprintln!("no monitor shows a desktop here: nothing to capture");
            return;
        }
        let shot = capture_primary_monitor().expect("capture");
        let img = &shot.image;
        assert!(img.width > 0 && img.height > 0);
        assert_eq!(img.pixels.len(), img.width as usize * img.height as usize * 3);
        let png = pixels::encode_png(img).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("primary.png");
        std::fs::write(&path, &png).unwrap();
        let back = image::open(&path).unwrap();
        assert_eq!((back.width(), back.height()), (img.width, img.height));
        eprintln!("captured {}x{} with {:?}", img.width, img.height, shot.method);
    }

    #[test]
    fn some_monitor_is_picked() {
        let _dpi = PerMonitorDpi::enter();
        // With no game running: the focused window's or the pointer's.
        assert!(!game_monitor().is_invalid() || !has_output());
    }
}
