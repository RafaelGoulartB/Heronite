//! Upkeep of the app's own data while it runs:
//! - the scheduled backup (`heronite_backup::run`), checked [`AFTER_START`]
//!   after start and then every [`CHECK_EVERY`], so an app left open for
//!   weeks still backs up; skipped while a game launched from here runs;
//! - the size cap of the image caches (Settings → Advanced), every
//!   [`TRIM_EVERY`]: a quarter for the resized copies, the rest for the
//!   downloads, least recently used first.
//!
//! "Back up now" runs here too, so the settings page and the schedule share
//! one state.

use std::sync::Mutex;
use std::time::Duration;

use chrono::Local;
use gpui_kit::*;
use heronite_backup::restore::Restored;
use heronite_backup::run::Outcome;
use heronite_data::{settings, AppPaths, DataPaths};
use heronite_i18n::tr;

const AFTER_START: Duration = Duration::from_secs(90);
const CHECK_EVERY: Duration = Duration::from_secs(3600);
const TRIM_EVERY: Duration = Duration::from_secs(6 * 3600);

pub struct Upkeep {
    data_paths: DataPaths,
    app_paths: AppPaths,
    running: bool,
    /// The last backup this process made or tried.
    last: Option<Outcome>,
}

struct UpkeepGlobal(Entity<Upkeep>);
impl Global for UpkeepGlobal {}

/// Starts the service and its timer (once, at start).
pub fn init(cx: &mut App, data_paths: DataPaths) {
    if cx.has_global::<UpkeepGlobal>() {
        return;
    }
    let app_paths = AppPaths::for_data(&data_paths);
    let entity = cx.new(|cx| {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AFTER_START).await;
            let mut since_trim = TRIM_EVERY;
            loop {
                let alive = this
                    .update(cx, |u: &mut Upkeep, cx: &mut Context<Upkeep>| {
                        if crate::system::restart_blocker(cx).is_none() {
                            u.back_up(false, cx);
                        }
                        if since_trim >= TRIM_EVERY {
                            u.trim_caches(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                if since_trim >= TRIM_EVERY {
                    since_trim = Duration::ZERO;
                }
                cx.background_executor().timer(CHECK_EVERY).await;
                since_trim += CHECK_EVERY;
            }
        })
        .detach();
        Upkeep { data_paths, app_paths, running: false, last: None }
    });
    cx.set_global(UpkeepGlobal(entity));
}

/// The service, when started (not in a one-shot launch).
pub fn get(cx: &App) -> Option<Entity<Upkeep>> {
    cx.try_global::<UpkeepGlobal>().map(|g| g.0.clone())
}

impl Upkeep {
    pub fn running(&self) -> bool {
        self.running
    }

    pub fn last(&self) -> Option<&Outcome> {
        self.last.as_ref()
    }

    /// "Back up now".
    pub fn back_up_now(&mut self, cx: &mut Context<Self>) {
        self.back_up(true, cx);
    }

    fn back_up(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        if force {
            self.running = true;
            cx.notify();
        }
        cx.spawn(async move |this, cx| {
            let outcome = cx.background_spawn(async move { heronite_backup::run::run(&data, &app, force, Local::now()) }).await;
            this.update(cx, |u, cx| {
                u.running = false;
                // A schedule that was not due is not news.
                if force || outcome.ran || outcome.error.is_some() {
                    u.last = Some(outcome);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Applies the image cache cap now (also after the setting changes).
    pub fn trim_caches(&mut self, _cx: &mut Context<Self>) {
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        std::thread::spawn(move || {
            let limit_mb = settings::load(&data).map(|s| s.image_cache_limit_mb).unwrap_or(settings::DEFAULT_IMAGE_CACHE_LIMIT_MB);
            if limit_mb == 0 {
                return;
            }
            let limit = u64::from(limit_mb) * 1024 * 1024;
            let thumbs = heronite_images::evict_lru(&app.thumbs_dir(), limit / 4);
            let downloads = heronite_images::evict_downloads(&data.images_cache_dir(), limit - limit / 4);
            for (name, st) in [("resized images", thumbs), ("downloaded images", downloads)] {
                match st {
                    Ok(st) if st.files_removed > 0 => {
                        log::info!("image cache cap: removed {} {name} ({} KB)", st.files_removed, st.bytes_removed / 1024)
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => log::warn!("trimming the {name}: {e}"),
                }
            }
        });
    }
}

/// What the restore at this start did, for the Collection's first toast.
static RESTORE_NOTICE: Mutex<Option<(String, bool)>> = Mutex::new(None);

/// Records the result of `heronite_backup::restore::apply_pending`.
pub fn set_restore_result(result: &anyhow::Result<Restored>) {
    let notice = match result {
        Ok(r) => {
            let when = crate::pages::settings::format_time(&r.created_at);
            (tr!("Restored the backup from {when}.", when), false)
        }
        Err(e) => (tr!("Could not restore the backup: {error}", error = heronite_backup::describe(e)), true),
    };
    *RESTORE_NOTICE.lock().unwrap_or_else(|e| e.into_inner()) = Some(notice);
}

/// The notice of [`set_restore_result`], once.
pub fn take_restore_notice() -> Option<(String, bool)> {
    RESTORE_NOTICE.lock().unwrap_or_else(|e| e.into_inner()).take()
}
