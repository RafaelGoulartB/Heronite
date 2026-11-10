//! What the dialog does off the UI thread: read a package, download and
//! check a list's entry, install, and add or remove a list. Each ends by
//! updating the dialog and, when something changed, the hub.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata_ext::lists;
use heronite_metadata_ext::package::{MANIFEST, README, SCRIPT};
use heronite_metadata_ext::{Approval, ExtensionError, ExtensionsExt, InstallOutcome};

use super::{words, Confirm, ExtensionsDialog, Face, Offer, Package};
use crate::ui::fields::choose_path;

/// The package a chosen file stands for: a file of an extension's folder
/// stands for the folder (the file chooser picks files only).
pub(super) fn package_path(chosen: &Path) -> PathBuf {
    let name = chosen.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match chosen.parent() {
        Some(dir) if [MANIFEST, SCRIPT, README].contains(&name) => dir.to_path_buf(),
        _ => chosen.to_path_buf(),
    }
}

impl ExtensionsDialog {
    /// The installer of `path`, reading it off the UI thread.
    pub(super) fn open_file(&mut self, path: PathBuf, back: bool, cx: &mut Context<Self>) {
        let path = package_path(&path);
        self.face = Face::Confirm(Box::new(Confirm {
            from: Package::File(path.clone()),
            info: None,
            error: None,
            installing: false,
            back,
            keep_settings: false,
        }));
        let service = self.hub.read(cx).service.clone();
        cx.spawn(async move |this, cx| {
            let read = cx.background_spawn(async move { service.inspect_extension(&path) }).await;
            this.update(cx, |d, cx| {
                if let Face::Confirm(c) = &mut d.face {
                    match read {
                        Ok(info) => {
                            c.keep_settings = info.keeps_settings_by_default();
                            c.info = Some(Box::new(info));
                        }
                        Err(e) => c.error = Some(e),
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// "Install from a file…": the file chooser, then the installer.
    pub(super) fn pick_file(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(
            window,
            cx,
            false,
            tr!("Choose an extension: its .zip, or the source.toml in its folder"),
            move |d: &mut Self, p, _, cx| {
                d.open_file(PathBuf::from(p), back, cx);
            },
        );
    }

    /// Downloads and checks `offer`'s files, then shows the installer.
    pub(super) fn prepare(&mut self, offer: &Offer, cx: &mut Context<Self>) {
        if self.preparing.is_some() {
            return;
        }
        let id = offer.entry.id.clone();
        self.preparing = Some(id.clone());
        self.entry_error = None;
        let service = self.hub.read(cx).service.clone();
        let fake = self.hub.read(cx).extensions.fake.is_some();
        let (list, entry) = (offer.list.clone(), offer.entry.clone());
        let origin = words::list_origin(&offer.list, offer.builtin);
        cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    if fake {
                        return Err(ExtensionError::Download("made-up list: nothing to download".into()));
                    }
                    lists::prepare(&service, &list, &entry)
                })
                .await;
            this.update(cx, |d, cx| {
                d.preparing = None;
                match prepared {
                    Ok(p) => {
                        let keep_settings = p.info.keeps_settings_by_default();
                        let info = Some(Box::new(p.info.clone()));
                        let from = Package::List { prepared: Arc::new(p), origin };
                        d.face = Face::Confirm(Box::new(Confirm { from, info, error: None, installing: false, back: true, keep_settings }));
                    }
                    Err(e) => d.entry_error = Some((id, e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Installs what the installer shows (the user saw what it is, so the
    /// same or an older version goes ahead too): that package, by its
    /// SHA-256, keeping its saved settings or not as the user chose.
    pub(super) fn install(&mut self, cx: &mut Context<Self>) {
        let Face::Confirm(c) = &mut self.face else { return };
        let Some(info) = &c.info else { return };
        if c.installing {
            return;
        }
        let approval = Approval { sha256: info.sha256.clone(), keep_settings: !info.asks_about_settings() || c.keep_settings };
        c.installing = true;
        c.error = None;
        let service = self.hub.read(cx).service.clone();
        let job: Box<dyn FnOnce() -> Result<InstallOutcome, ExtensionError> + Send> = match &c.from {
            Package::File(path) => {
                let path = path.clone();
                Box::new(move || service.install_approved(&path, &approval))
            }
            Package::List { prepared, .. } => {
                let prepared = prepared.clone();
                Box::new(move || lists::install_approved(&service, &prepared, &approval))
            }
        };
        cx.spawn(async move |this, cx| {
            let done = cx.background_spawn(async move { job() }).await;
            this.update(cx, |d, cx| {
                match done {
                    Ok(InstallOutcome::Installed(ext) | InstallOutcome::Updated { extension: ext, .. }) => {
                        d.hub.update(cx, |h, cx| h.extension_installed(&ext.id, cx));
                        d.close(cx);
                    }
                    // Asked with `confirmed`: never comes back.
                    Ok(InstallOutcome::NeedsConfirmation { .. }) => {}
                    Err(e) => {
                        if let Face::Confirm(c) = &mut d.face {
                            c.installing = false;
                            c.error = Some(e);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Reads the lists again from the network.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.hub.update(cx, |h, cx| h.read_extensions(true, cx));
    }

    /// Adds the list typed in "Lists…" and reads it.
    pub(super) fn add_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.list_input.read(cx).value().trim().to_string();
        if url.is_empty() || self.hub.read(cx).extensions.fake.is_some() {
            return;
        }
        let service = self.hub.read(cx).service.clone();
        let lists = self.hub.read(cx).extensions.lists.clone();
        cx.spawn_in(window, async move |this, cx| {
            let added = cx.background_spawn(async move { lists.add(&service, &url) }).await;
            this.update_in(cx, |d, window, cx| {
                match added {
                    Ok(_) => {
                        d.list_error = None;
                        d.list_input.update(cx, |s, cx| s.set_value("", window, cx));
                        d.refresh(cx);
                    }
                    Err(e) => d.list_error = Some(words::list_error(&e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stops reading the list at `url` (what came from it stays installed).
    pub(super) fn remove_list(&mut self, url: String, cx: &mut Context<Self>) {
        if self.hub.read(cx).extensions.fake.is_some() {
            return;
        }
        let service = self.hub.read(cx).service.clone();
        let lists = self.hub.read(cx).extensions.lists.clone();
        cx.spawn(async move |this, cx| {
            let removed = cx.background_spawn(async move { lists.remove(&service, &url) }).await;
            this.update(cx, |d, cx| {
                d.list_error = removed.err().map(|e| words::list_error(&e));
                d.hub.update(cx, |h, cx| h.read_extensions(false, cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::package_path;

    #[test]
    fn a_file_of_a_folder_stands_for_the_folder() {
        assert_eq!(package_path(Path::new("/x/mydb/source.toml")), Path::new("/x/mydb"));
        assert_eq!(package_path(Path::new("/x/mydb/source.js")), Path::new("/x/mydb"));
        assert_eq!(package_path(Path::new("/x/mydb.zip")), Path::new("/x/mydb.zip"));
        assert_eq!(package_path(Path::new("/x/other.toml")), Path::new("/x/other.toml"));
    }
}
