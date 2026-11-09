//! COM on the current thread for as long as a guard lives (Windows).

use std::marker::PhantomData;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT};

/// COM for the current thread while it lives, in the apartment `flags` asks
/// for. A thread already in COM in the other apartment keeps its own
/// (`RPC_E_CHANGED_MODE` is not an error then), and only an initialization
/// that succeeded (`S_OK` or `S_FALSE`) is undone on drop. It stays on the
/// thread that made it (not `Send`).
pub(crate) struct Com {
    initialized: bool,
    _thread: PhantomData<*const ()>,
}

impl Com {
    pub(crate) fn init(flags: COINIT) -> Self {
        // SAFETY: balanced by `CoUninitialize` in `drop` when it succeeded.
        let hr = unsafe { CoInitializeEx(None, flags) };
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            log::warn!("CoInitializeEx: {hr:?}");
        }
        Com { initialized: hr.is_ok(), _thread: PhantomData }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: this thread's successful `CoInitializeEx`.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::S_FALSE;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};

    #[test]
    fn a_thread_in_the_other_apartment_keeps_it() {
        std::thread::spawn(|| {
            // SAFETY: balanced by the two `CoUninitialize` below.
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
            let com = Com::init(COINIT_MULTITHREADED);
            assert!(!com.initialized, "RPC_E_CHANGED_MODE is not an initialization");
            drop(com);
            // Still the single-threaded apartment: joining it again is S_FALSE.
            // SAFETY: as above.
            unsafe {
                assert_eq!(CoInitializeEx(None, COINIT_APARTMENTTHREADED), S_FALSE);
                CoUninitialize();
                CoUninitialize();
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_new_thread_is_initialized_and_released() {
        std::thread::spawn(|| {
            let com = Com::init(COINIT_MULTITHREADED);
            assert!(com.initialized);
            drop(com);
            // Released: the thread may now join the other apartment.
            // SAFETY: balanced below.
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
                CoUninitialize();
            }
        })
        .join()
        .unwrap();
    }
}
