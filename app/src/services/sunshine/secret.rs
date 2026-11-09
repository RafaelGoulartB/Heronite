//! Where the Sunshine password lives, never in the clear: on Windows sealed
//! with DPAPI for this user in `sunshine.json` (`heronite_sunshine::prefs`);
//! elsewhere in the desktop's keyring (the Secret Service, through GPUI's
//! credentials API). Neither the password nor its sealed form is logged.

use gpui_kit::*;
use heronite_sunshine::prefs::Prefs;

/// The keyring item's address (Linux).
#[cfg_attr(windows, allow(dead_code))]
const KEY: &str = "heronite://sunshine";

/// The kept password, if any.
pub fn load(prefs: &Prefs, cx: &App) -> Task<Result<Option<String>, String>> {
    #[cfg(windows)]
    {
        let _ = cx;
        if prefs.sealed_password.is_empty() {
            return Task::ready(Ok(None));
        }
        Task::ready(
            heronite_sunshine::prefs::unseal(&prefs.sealed_password)
                .map(Some)
                .ok_or_else(|| "the saved password cannot be opened".to_string()),
        )
    }
    #[cfg(not(windows))]
    {
        let _ = prefs;
        let read = cx.read_credentials(KEY);
        cx.background_spawn(async move {
            match read.await {
                Ok(Some((_, bytes))) => String::from_utf8(bytes).map(Some).map_err(|_| "the saved password is not text".to_string()),
                Ok(None) => Ok(None),
                Err(e) => Err(format!("{e:#}")),
            }
        })
    }
}

/// Keeps `password`. On Windows the sealed form goes into `prefs` (the
/// caller saves them); elsewhere into the keyring.
pub fn store(prefs: &mut Prefs, password: &str, cx: &App) -> Task<Result<(), String>> {
    #[cfg(windows)]
    {
        let _ = cx;
        Task::ready(heronite_sunshine::prefs::seal(password).map(|s| prefs.sealed_password = s).map_err(|e| e.to_string()))
    }
    #[cfg(not(windows))]
    {
        prefs.sealed_password.clear();
        // One item: an older one (another user name) goes first.
        let delete = cx.delete_credentials(KEY);
        let (user, password) = (prefs.username.clone(), password.to_string());
        cx.spawn(async move |cx| {
            let _ = delete.await;
            let write = cx.update(|cx| cx.write_credentials(KEY, &user, password.as_bytes()));
            write.await.map_err(|e| format!("{e:#}"))
        })
    }
}
