//! Native popup menus have no documented dark-mode switch. These uxtheme
//! exports are loaded only on Windows 10 1903+, where ordinal 135 has the
//! SetPreferredAppMode signature (1809 used a different function there).

use std::io;
use std::mem;
use std::ptr;
use std::sync::OnceLock;

use windows_sys::core::BOOL;
use windows_sys::w;
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE, HWND};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETHIGHCONTRAST};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuTheme {
    #[default]
    System,
    Light,
    Dark,
}

type RawProc = unsafe extern "system" fn() -> isize;
type Version = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
type PreferredMode = unsafe extern "system" fn(i32) -> i32;
type AllowWindow = unsafe extern "system" fn(HWND, BOOL) -> BOOL;
type Refresh = unsafe extern "system" fn();

struct Module(isize);

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: the module reference was acquired by LoadLibraryExW.
        unsafe { FreeLibrary(self.0 as HMODULE) };
    }
}

struct Api {
    _module: Module,
    preferred: PreferredMode,
    allow_window: AllowWindow,
    flush: Option<Refresh>,
    refresh_policy: Option<Refresh>,
}

fn supported_version(major: u32, minor: u32, build: u32) -> bool {
    major == 10 && minor == 0 && build >= 18362
}

fn preferred_mode(theme: MenuTheme, high_contrast: bool) -> i32 {
    if high_contrast {
        0 // Default: preserve the user's accessibility theme.
    } else {
        match theme {
            MenuTheme::System => 1, // AllowDark
            MenuTheme::Dark => 2,   // ForceDark
            MenuTheme::Light => 3,  // ForceLight
        }
    }
}

fn high_contrast() -> io::Result<bool> {
    // SAFETY: HIGHCONTRASTW is plain C data, with cbSize set before the query.
    let mut settings: HIGHCONTRASTW = unsafe { mem::zeroed() };
    settings.cbSize = mem::size_of::<HIGHCONTRASTW>() as u32;
    // SAFETY: the output buffer has the size required by this action.
    if unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, settings.cbSize, (&mut settings as *mut HIGHCONTRASTW).cast(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(settings.dwFlags & HCF_HIGHCONTRASTON != 0)
}

impl Api {
    fn load() -> io::Result<Self> {
        // SAFETY: ntdll is loaded for the lifetime of a Windows process.
        let ntdll = unsafe { GetModuleHandleW(w!("ntdll.dll")) };
        if ntdll.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the name is NUL-terminated and the module is loaded.
        let version = unsafe { GetProcAddress(ntdll, c"RtlGetVersion".as_ptr().cast()) }
            .ok_or_else(|| io::Error::other("RtlGetVersion is unavailable"))?;
        // SAFETY: the named export has the RtlGetVersion signature.
        let version = unsafe { mem::transmute::<RawProc, Version>(version) };
        // SAFETY: OSVERSIONINFOW is plain C data.
        let mut info: OSVERSIONINFOW = unsafe { mem::zeroed() };
        info.dwOSVersionInfoSize = mem::size_of::<OSVERSIONINFOW>() as u32;
        // SAFETY: the correctly sized version buffer is writable.
        if unsafe { version(&mut info) } < 0 {
            return Err(io::Error::other("RtlGetVersion failed"));
        }
        if !supported_version(info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber) {
            return Err(io::Error::other("native dark tray menus require Windows 10 1903 or newer"));
        }
        // SAFETY: restrict DLL lookup to System32, never the current directory.
        let module = unsafe { LoadLibraryExW(w!("uxtheme.dll"), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            return Err(io::Error::last_os_error());
        }
        let module = Module(module as isize);
        // SAFETY: GetProcAddress accepts ordinals encoded as low pointer values.
        let ordinal = |number: usize| unsafe { GetProcAddress(module.0 as HMODULE, number as *const u8) };
        let preferred = ordinal(135).ok_or_else(|| io::Error::other("SetPreferredAppMode is unavailable"))?;
        let allow_window = ordinal(133).ok_or_else(|| io::Error::other("AllowDarkModeForWindow is unavailable"))?;
        // SAFETY: the version gate above selects these known export signatures.
        let preferred = unsafe { mem::transmute::<RawProc, PreferredMode>(preferred) };
        let allow_window = unsafe { mem::transmute::<RawProc, AllowWindow>(allow_window) };
        // SAFETY: these optional exports take no arguments and return nothing.
        let flush = ordinal(136).map(|p| unsafe { mem::transmute::<RawProc, Refresh>(p) });
        let refresh_policy = ordinal(104).map(|p| unsafe { mem::transmute::<RawProc, Refresh>(p) });
        if flush.is_none() || refresh_policy.is_none() {
            log::warn!("some native menu theme refresh exports are unavailable");
        }
        Ok(Self { _module: module, preferred, allow_window, flush, refresh_policy })
    }
}

/// PreferredAppMode is process-wide, so restore it after this popup closes.
pub(super) struct Scope {
    api: &'static Api,
    hwnd: HWND,
    previous: i32,
}

impl Scope {
    pub(super) fn enter(hwnd: HWND, theme: MenuTheme) -> Option<Self> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        let api = API
            .get_or_init(|| Api::load().map_err(|e| log::warn!("native tray theming unavailable: {e}; using the Windows menu")).ok())
            .as_ref()?;
        let contrast = match high_contrast() {
            Ok(on) => on,
            Err(e) => {
                log::warn!("could not read Windows high contrast: {e}; leaving the menu theme unchanged");
                return None;
            }
        };
        // SAFETY: loaded exports outlive the scope; hwnd belongs to this tray.
        let previous = unsafe {
            if let Some(refresh) = api.refresh_policy {
                refresh();
            }
            let previous = (api.preferred)(preferred_mode(theme, contrast));
            (api.allow_window)(hwnd, BOOL::from(!contrast && theme != MenuTheme::Light));
            if let Some(flush) = api.flush {
                flush();
            }
            previous
        };
        Some(Self { api, hwnd, previous })
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // SAFETY: the hidden tray window and module still live; restore only our change.
        unsafe {
            (self.api.allow_window)(self.hwnd, 0);
            (self.api.preferred)(self.previous);
            if let Some(flush) = self.api.flush {
                flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinal_signature_is_gated_to_1903_and_later() {
        assert!(!supported_version(10, 0, 17763));
        assert!(supported_version(10, 0, 18362));
        assert!(supported_version(10, 0, 26100));
        assert!(!supported_version(6, 3, 9600));
        assert!(!supported_version(11, 0, 1));
    }

    #[test]
    fn high_contrast_wins_over_every_app_theme() {
        assert_eq!(preferred_mode(MenuTheme::System, false), 1);
        assert_eq!(preferred_mode(MenuTheme::Dark, false), 2);
        assert_eq!(preferred_mode(MenuTheme::Light, false), 3);
        for theme in [MenuTheme::System, MenuTheme::Light, MenuTheme::Dark] {
            assert_eq!(preferred_mode(theme, true), 0);
        }
    }
}
