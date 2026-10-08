//! The one-shot launch arguments that boot into a chosen state.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::HeroicPaths;

use super::listing::{collect_states, resolve, rom_keys_for};
use crate::sysfiles::HostEnv;

pub const PENDING_TTL: Duration = Duration::from_secs(60);

/// Extra emulator arguments that boot into a state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchStateArgs {
    pub args: Vec<String>,
    /// The state boots the game by itself; drop the ROM path from argv.
    pub replaces_image: bool,
}

impl LaunchStateArgs {
    /// Prepends the state arguments to the emulator argv (dropping the ROM
    /// path when the state replaces the image).
    pub fn apply(&self, argv: Vec<String>, rom_path: Option<&str>) -> Vec<String> {
        let rest: Vec<String> = if self.replaces_image {
            argv.into_iter().filter(|p| Some(p.as_str()) != rom_path).collect()
        } else {
            argv
        };
        self.args.iter().cloned().chain(rest).collect()
    }
}

/// The choice made in the Collection launch dialog, consumed by the next
/// launch of that game. It expires so an abandoned launch cannot leak into a
/// later one started from elsewhere (official Library, tray, etc.).
#[derive(Debug)]
pub struct PendingLaunchStates {
    ttl: Duration,
    inner: Mutex<HashMap<String, (LaunchStateArgs, String, Instant)>>,
}

impl Default for PendingLaunchStates {
    fn default() -> Self {
        Self::with_ttl(PENDING_TTL)
    }
}

impl PendingLaunchStates {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self { ttl, inner: Default::default() }
    }

    /// Process-wide instance for apps that do not carry their own.
    pub fn global() -> &'static PendingLaunchStates {
        static G: OnceLock<PendingLaunchStates> = OnceLock::new();
        G.get_or_init(Default::default)
    }

    pub fn clear(&self, app_name: &str) {
        self.inner.lock().unwrap().remove(app_name);
    }

    pub fn set(&self, app_name: &str, rom_path: &str, args: LaunchStateArgs) {
        self.inner
            .lock()
            .unwrap()
            .insert(app_name.into(), (args, rom_path.into(), Instant::now() + self.ttl));
    }

    /// `takePendingLaunchState`: consumes the entry (even when it does not
    /// apply, so it never leaks into a later launch).
    pub fn take(&self, app_name: &str, rom_path: Option<&str>) -> Option<LaunchStateArgs> {
        let (args, rom, expires) = self.inner.lock().unwrap().remove(app_name)?;
        if expires < Instant::now() {
            return None;
        }
        if rom_path.is_some_and(|r| !r.is_empty() && r != rom) {
            return None;
        }
        Some(args)
    }
}

/// `setPendingLaunchState`. With `state_path` the next launch boots into it;
/// without it, emulators that auto-load a state get "start fresh" arguments.
/// Returns `false` when the state no longer exists.
pub fn set_pending_launch_state(
    paths: &HeroicPaths,
    env: &HostEnv,
    pending: &PendingLaunchStates,
    app_name: &str,
    rom_path: Option<&str>,
    state_path: Option<&str>,
) -> Result<bool> {
    pending.clear(app_name);
    let Some(r) = resolve(paths, app_name, rom_path)? else {
        return Ok(state_path.is_none());
    };
    let launch = match state_path {
        Some(sp) => {
            let keys = rom_keys_for(&r, env);
            let states = if keys.is_empty() { vec![] } else { collect_states(&r, env, &keys, None) };
            let Some(state) = states.into_iter().find(|f| f.state.path == sp) else {
                return Ok(false);
            };
            r.engine
                .load_args(env, &r.ctx, &state.state.path, state.state.kind, state.state.slot)?
        }
        None => {
            let fresh = r.engine.fresh_args(env, &r.ctx)?;
            if fresh.is_empty() {
                return Ok(true);
            }
            LaunchStateArgs { args: fresh, replaces_image: false }
        }
    };
    pending.set(app_name, &r.rom_path, launch);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_prepends_and_can_drop_the_image() {
        let argv = vec!["-fullscreen".to_string(), "/r/a.iso".to_string()];
        let s1 = LaunchStateArgs { args: vec!["-statefile".into(), "/st".into()], replaces_image: false };
        assert_eq!(s1.apply(argv.clone(), Some("/r/a.iso")), ["-statefile", "/st", "-fullscreen", "/r/a.iso"]);
        let s2 = LaunchStateArgs { args: vec!["--savestate".into(), "/st".into()], replaces_image: true };
        assert_eq!(s2.apply(argv, Some("/r/a.iso")), ["--savestate", "/st", "-fullscreen"]);
    }

    #[test]
    fn pending_states_are_consumed_and_expire() {
        let p = PendingLaunchStates::with_ttl(Duration::from_millis(30));
        let a = LaunchStateArgs { args: vec!["x".into()], replaces_image: false };
        p.set("g", "/r/a.iso", a.clone());
        assert_eq!(p.take("g", Some("/r/a.iso")), Some(a.clone()));
        assert_eq!(p.take("g", Some("/r/a.iso")), None);
        p.set("g", "/r/a.iso", a.clone());
        assert_eq!(p.take("g", Some("/r/b.iso")), None);
        assert_eq!(p.take("g", None), None);
        p.set("g", "/r/a.iso", a.clone());
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(p.take("g", None), None);
        p.set("g", "/r/a.iso", a.clone());
        assert_eq!(p.take("g", None), Some(a));
    }
}
