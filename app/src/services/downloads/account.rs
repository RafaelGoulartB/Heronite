//! Epic login and logout.

use gpui_kit::*;
use heronite_epic::cmd::Legendary;
use heronite_epic::library;
use heronite_epic::user;
use heronite_i18n::tr;

use super::library_file::read_stats;
use super::{Downloads, DownloadsEvent, Stats};

impl Downloads {
    /// Logs in with what the user pasted or the webview returned.
    pub fn login(&mut self, input: String, cx: &mut Context<Self>) {
        if self.logging_in {
            return;
        }
        self.logging_in = true;
        self.last_error = None;
        cx.notify();
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let lg = Legendary::locate_or_install(&data_paths)?;
                    let info = user::login(&data_paths, &lg, &input)?;
                    // Heroic refreshes the library right after logging in.
                    let refresh_error = match library::refresh(&data_paths, &lg) {
                        Ok(r) => r.list_error,
                        Err(err) => Some(format!("{err:#}")),
                    };
                    anyhow::Ok((info, refresh_error, library::updates_available(&data_paths), read_stats(&data_paths)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.logging_in = false;
                match res {
                    Ok((info, refresh_error, updates, stats)) => {
                        let name = info.display_name.clone();
                        this.apply_account(Some(info));
                        this.updates = updates.into_iter().collect();
                        this.stats = stats;
                        this.toast(cx, tr!("Logged in as {name}", name), false);
                        // Signed in, but the games could not be listed: say
                        // so, or the library just stays empty.
                        if let Some(e) = refresh_error {
                            this.toast(
                                cx,
                                tr!("Could not refresh the Epic library: {error}", error = e.lines().next().unwrap_or("")),
                                true,
                            );
                            this.last_error = Some(e);
                        }
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                    }
                    Err(err) => {
                        let msg = format!("{err:#}");
                        this.last_error = Some(msg.clone());
                        this.toast(cx, msg, true);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        self.logging_out = true;
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&data_paths)?;
                    user::logout(&data_paths, &lg)
                })
                .await;
            this.update(cx, |this, cx| {
                this.logging_out = false;
                match res {
                    Ok(()) => {
                        this.account = Default::default();
                        this.last_error = None;
                        this.updates.clear();
                        this.stats = Stats::default();
                        this.toast(cx, tr!("Logged out of Epic Games"), false);
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                    }
                    Err(err) => this.toast(cx, format!("{err:#}"), true),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
