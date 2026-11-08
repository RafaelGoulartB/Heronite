//! Shortcuts of the clients under Wine (Linux): `heronite-client-<id>.desktop`
//! on the Desktop and in the application menu, starting this program with
//! `--open-client <id>` (the client opens, Heronite's window does not), with
//! the icon of the client's own program. The file is named after the client's
//! settings app, not its title, so a launcher of another tool named
//! "Battle.net" is never replaced. `HERONITE_DESKTOP_DIR` /
//! `HERONITE_APPLICATIONS_DIR` move both places (see `Roots::detect`).

use std::path::{Path, PathBuf};

use gpui_kit::*;
use heronite_clients::Client;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_shortcuts::desktop::{self, Roots};
use heronite_shortcuts::pe_icon::{self, IconImage};

use super::{Clients, Notice};

/// Where a client's shortcuts are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shortcuts {
    pub desktop: bool,
    pub menu: bool,
    /// The Desktop folder exists (`false`: only the menu is offered; one is
    /// never made just for this).
    pub has_desktop: bool,
}

fn stem(client: Client) -> &'static str {
    client.config_app()
}

/// The PNG the shortcuts show, made from the client's program.
fn icon_path(paths: &DataPaths, client: Client) -> PathBuf {
    paths.icons_dir().join(format!("{}.png", stem(client)))
}

/// Where the shortcuts of `client` are now. Blocking (a few file checks).
pub(super) fn placed(client: Client) -> Shortcuts {
    let roots = Roots::detect();
    let (on_desktop, in_menu) = desktop::files_named(&roots, stem(client));
    Shortcuts {
        desktop: on_desktop.as_ref().is_some_and(|f| f.is_file()),
        menu: in_menu.is_file(),
        has_desktop: on_desktop.as_ref().and_then(|f| f.parent()).is_some_and(Path::is_dir),
    }
}

/// The client program's icon as PNG bytes; `None` when it has none.
fn png_icon(exe: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(exe).ok()?;
    match pe_icon::main_icon(&bytes)? {
        IconImage::Png(png) => Some(png),
        IconImage::Ico(ico) => {
            let img = image::load_from_memory_with_format(&ico, image::ImageFormat::Ico).ok()?;
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
            Some(png)
        }
    }
}

/// Writes the icon next to the games' (`icons/`), made again every time:
/// an update of the client may change it. `None` leaves the shortcut on the
/// desktop's default icon.
fn write_icon(paths: &DataPaths, client: Client, exe: &Path) -> Option<PathBuf> {
    let Some(png) = png_icon(exe) else {
        log::info!("{}: no icon in {}", client.name(), exe.display());
        return None;
    };
    let dest = icon_path(paths, client);
    let written = dest.parent().map_or(Ok(()), std::fs::create_dir_all).map_err(anyhow::Error::from).and_then(|()| {
        heronite_data::temps::write_atomic(&dest, &png, None)?;
        Ok(())
    });
    match written {
        Ok(()) => Some(dest),
        Err(e) => {
            log::warn!("{}: writing the shortcut icon {}: {e:#}", client.name(), dest.display());
            None
        }
    }
}

/// The `.desktop` text of `client`'s shortcut.
fn entry(client: Client, own_exe: &Path, icon: Option<&Path>) -> String {
    let exec = format!("{} --open-client {}", desktop::quote_exec_arg(&own_exe.to_string_lossy()), client.id());
    desktop::desktop_file_text(client.name(), &exec, icon.map(|p| p.to_string_lossy()).as_deref())
}

/// Adds (`Some(true)`) or removes (`Some(false)`) the shortcut of `client`
/// on the Desktop and in the menu; `None` leaves that one as it is. `exe` is
/// the client's program (its icon). Blocking.
fn set(paths: &DataPaths, client: Client, exe: Option<&Path>, desktop: Option<bool>, menu: Option<bool>) -> Result<Shortcuts, String> {
    let roots = Roots::detect();
    let (add_desktop, add_menu) = (desktop == Some(true), menu == Some(true));
    if add_desktop || add_menu {
        let icon = exe.and_then(|exe| write_icon(paths, client, exe));
        let text = entry(client, &crate::system::shortcuts::own_exe(), icon.as_deref());
        desktop::add_named(&roots, stem(client), &text, add_desktop, add_menu).map_err(|e| format!("{e:#}"))?;
    }
    let (drop_desktop, drop_menu) = (desktop == Some(false), menu == Some(false));
    if drop_desktop || drop_menu {
        desktop::remove_named(&roots, stem(client), drop_desktop, drop_menu).map_err(|e| format!("{e:#}"))?;
    }
    let now = placed(client);
    if !now.desktop && !now.menu {
        remove_icon(paths, client);
    }
    Ok(now)
}

fn remove_icon(paths: &DataPaths, client: Client) {
    match std::fs::remove_file(icon_path(paths, client)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("{}: removing the shortcut icon: {e}", client.name()),
    }
}

/// Removes both shortcuts and the icon (the client was forgotten: they
/// would open a client that is no longer set up). Blocking.
pub(super) fn remove_all(paths: &DataPaths, client: Client) {
    if let Err(e) = desktop::remove_named(&Roots::detect(), stem(client), true, true) {
        log::warn!("{}: removing its shortcuts: {e:#}", client.name());
    }
    remove_icon(paths, client);
}

impl Clients {
    /// Adds or removes the shortcuts of `client` (see [`set`]). A client no
    /// longer installed (its prefix deleted by hand) can only lose them.
    pub fn set_shortcuts(&mut self, client: Client, desktop: Option<bool>, menu: Option<bool>, cx: &mut Context<Self>) {
        let st = self.state(client);
        let adds = desktop == Some(true) || menu == Some(true);
        if self.frozen == Some(client) || st.shortcut_busy || st.busy() || (adds && !st.installed()) {
            return;
        }
        let exe = st.status.as_ref().and_then(|s| s.exe.clone());
        let st = self.state_mut(client);
        st.shortcut_busy = true;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { set(&paths, client, exe.as_deref(), desktop, menu) }).await;
            this.update(cx, |this, cx| {
                let st = this.state_mut(client);
                st.shortcut_busy = false;
                // A look that read the files before this change is stale.
                st.shortcuts_changes += 1;
                match res {
                    Ok(now) => st.shortcuts = now,
                    Err(e) => {
                        log::warn!("{}: shortcut: {e}", client.name());
                        st.notice = Some(Notice::error(tr!("Could not change the shortcut: {error}", error = e)));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use heronite_clients::Client;

    use super::{entry, png_icon};

    #[test]
    fn the_shortcut_opens_the_client_with_this_program() {
        let text =
            entry(Client::BattleNet, Path::new("/opt/Heronite App/heronite-rs"), Some(Path::new("/d/icons/heronite-client-battlenet.png")));
        assert!(text.contains("Name=Battle.net\n"), "{text}");
        assert!(text.contains("Exec=\"/opt/Heronite App/heronite-rs\" --open-client battlenet\n"), "{text}");
        assert!(text.contains("Icon=/d/icons/heronite-client-battlenet.png\n"), "{text}");
    }

    #[test]
    fn a_file_that_is_not_a_program_has_no_icon() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.exe");
        std::fs::write(&file, b"MZ not really").unwrap();
        assert_eq!(png_icon(&file), None);
        assert_eq!(png_icon(&dir.path().join("missing.exe")), None);
    }
}
