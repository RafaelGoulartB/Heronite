//! Where the EA account's session (`heronite_clients::ea_account::Session`,
//! the web session's cookies) lives, never in the clear: on Windows sealed
//! with DPAPI for this user in `local_library/ea-account.bin`; elsewhere in
//! the desktop's keyring (the Secret Service, through GPUI's credentials
//! API), which is only asked when the account is read. Neither the session
//! nor its sealed form is logged.

use gpui_kit::*;
use heronite_clients::ea_account::Session;
use heronite_data::DataPaths;

/// The keyring item's address (Linux).
#[cfg_attr(windows, allow(dead_code))]
const KEY: &str = "heronite://ea-account";

#[cfg_attr(not(windows), allow(dead_code))]
fn sealed_file(paths: &DataPaths) -> std::path::PathBuf {
    paths.local_meta().with_file_name("ea-account.bin")
}

/// The kept session; `None` when there is none (or it cannot be opened:
/// another user's, another PC's, which is the same as signed out).
pub fn load(paths: &DataPaths, cx: &App) -> Task<Result<Option<Session>, String>> {
    #[cfg(windows)]
    {
        let file = sealed_file(paths);
        cx.background_spawn(async move {
            let Ok(sealed) = std::fs::read(&file) else { return Ok(None) };
            match heronite_platform::secret::unprotect(&sealed) {
                Ok(plain) => Ok(serde_json::from_slice(&plain).ok()),
                Err(e) => {
                    log::warn!("EA account: the saved sign-in cannot be opened ({e}); signed out");
                    Ok(None)
                }
            }
        })
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        let read = cx.read_credentials(KEY);
        cx.background_spawn(async move {
            match read.await {
                Ok(Some((_, bytes))) => Ok(serde_json::from_slice(&bytes).ok()),
                Ok(None) => Ok(None),
                Err(e) => Err(format!("{e:#}")),
            }
        })
    }
}

/// Keeps `session` in place of the one kept before.
pub fn store(paths: &DataPaths, session: &Session, cx: &App) -> Task<Result<(), String>> {
    let bytes = match serde_json::to_vec(session) {
        Ok(b) => b,
        Err(e) => return Task::ready(Err(e.to_string())),
    };
    #[cfg(windows)]
    {
        let file = sealed_file(paths);
        cx.background_spawn(async move {
            let sealed = heronite_platform::secret::protect(&bytes).map_err(|e| format!("sealing the EA sign-in: {e}"))?;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            heronite_data::temps::write_atomic(&file, &sealed, None).map_err(|e| format!("writing {}: {e}", file.display()))
        })
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        let write = cx.write_credentials(KEY, "ea", &bytes);
        cx.background_spawn(async move { write.await.map_err(|e| format!("{e:#}")) })
    }
}

/// Forgets the session.
pub fn clear(paths: &DataPaths, cx: &App) -> Task<Result<(), String>> {
    #[cfg(windows)]
    {
        let file = sealed_file(paths);
        cx.background_spawn(async move {
            match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        })
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        let delete = cx.delete_credentials(KEY);
        cx.background_spawn(async move { delete.await.map_err(|e| format!("{e:#}")) })
    }
}
