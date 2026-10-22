//! Who opens `nxm://` (Nexus Mods) and `ror2mm://` (Thunderstore) links: the
//! desktop entry that makes Heronite do it, and `xdg-mime` to read and set it.
//! All of it blocks: it runs off the UI thread.

use std::path::Path;
use std::process::{Command, Stdio};

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_shortcuts::desktop::{quote_exec_arg, Roots};

use super::{HandlerState, ModsEvent, ModsService};

/// The entry of ours that opens the links.
pub(super) const DESKTOP_FILE: &str = "com.heronite.rs-mods.desktop";
const SCHEMES: [&str; 2] = ["x-scheme-handler/nxm", "x-scheme-handler/ror2mm"];

fn entry_text(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Heronite (mods)\nComment=Opens the mod links of Nexus Mods and Thunderstore\nExec={} %u\nIcon=com.heronite.rs\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/nxm;x-scheme-handler/ror2mm;\n",
        quote_exec_arg(&exe.to_string_lossy())
    )
}

/// What `xdg-mime query default` printed: the entry's file name, or nothing.
fn parse_default(output: &str) -> Option<String> {
    output.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// `nxm` and `ror2mm` handlers together: ours only when both are.
fn state_of(handlers: &[Option<String>]) -> HandlerState {
    let ours = |h: &Option<String>| h.as_deref() == Some(DESKTOP_FILE);
    if handlers.iter().all(ours) {
        HandlerState::Ours
    } else if handlers.iter().any(|h| h.as_deref().is_some_and(|name| name != DESKTOP_FILE)) {
        HandlerState::Other
    } else {
        HandlerState::None
    }
}

fn query_default(scheme: &str) -> Option<String> {
    let out = Command::new("xdg-mime").args(["query", "default", scheme]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    parse_default(&String::from_utf8_lossy(&out.stdout))
}

fn read_state() -> HandlerState {
    let handlers: Vec<Option<String>> = SCHEMES.iter().map(|s| query_default(s)).collect();
    state_of(&handlers)
}

fn write_entry(applications: &Path, exe: &Path) -> Result<(), String> {
    let file = applications.join(DESKTOP_FILE);
    std::fs::create_dir_all(applications)
        .and_then(|()| std::fs::write(&file, entry_text(exe)))
        .map_err(|e| tr!("Could not write {path}: {error}", path = file.display(), error = e))
}

/// Writes the entry and makes it the default for both schemes.
fn register(applications: &Path, exe: &Path) -> Result<(), String> {
    write_entry(applications, exe)?;
    let set = Command::new("xdg-mime")
        .arg("default")
        .arg(DESKTOP_FILE)
        .args(SCHEMES)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match set {
        Ok(s) if s.success() => {}
        Ok(s) => return Err(tr!("xdg-mime could not set Heronite for the mod links (it exited with {status})", status = s)),
        Err(e) => return Err(tr!("xdg-mime is needed to set Heronite for the mod links: {error}", error = e)),
    }
    // Updates the desktop's cache of types; not every system has the tool.
    let _ =
        Command::new("update-desktop-database").arg(applications).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    Ok(())
}

impl ModsService {
    pub(super) fn read_handlers(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = cx.background_spawn(async { read_state() }).await;
            this.update(cx, |s, cx| {
                if s.handlers != state {
                    s.handlers = state;
                    cx.emit(ModsEvent::Account);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn write_handlers(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async {
                    register(&Roots::detect().applications, &crate::system::shortcuts::own_exe())?;
                    Ok::<_, String>(read_state())
                })
                .await;
            this.update(cx, |s, cx| {
                match result {
                    Ok(state) => s.handlers = state,
                    Err(message) => cx.emit(ModsEvent::Notice { game_key: String::new(), message, error: true }),
                }
                cx.emit(ModsEvent::Account);
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{entry_text, parse_default, state_of, write_entry, DESKTOP_FILE};
    use crate::services::mods::HandlerState;
    use std::path::Path;

    #[test]
    fn the_entry_opens_links_with_the_executable() {
        let text = entry_text(Path::new("/opt/My Apps/heronite-rs"));
        assert!(text.starts_with("[Desktop Entry]\nType=Application\nName=Heronite (mods)\n"), "{text}");
        assert!(text.contains("Exec=\"/opt/My Apps/heronite-rs\" %u\n"), "{text}");
        assert!(text.contains("NoDisplay=true\n"));
        assert!(text.contains("MimeType=x-scheme-handler/nxm;x-scheme-handler/ror2mm;\n"));
    }

    #[test]
    fn xdg_mime_answers_are_read() {
        assert_eq!(parse_default("com.heronite.rs-mods.desktop\n").as_deref(), Some(DESKTOP_FILE));
        assert_eq!(parse_default("\n"), None);
        assert_eq!(parse_default(""), None);
        assert_eq!(parse_default("  vortex-nxm.desktop \n").as_deref(), Some("vortex-nxm.desktop"));
    }

    #[test]
    fn the_state_needs_both_schemes() {
        let ours = Some(DESKTOP_FILE.to_string());
        let other = Some("vortex.desktop".to_string());
        assert_eq!(state_of(&[ours.clone(), ours.clone()]), HandlerState::Ours);
        assert_eq!(state_of(&[ours.clone(), None]), HandlerState::None);
        assert_eq!(state_of(&[None, None]), HandlerState::None);
        assert_eq!(state_of(&[other.clone(), None]), HandlerState::Other);
        assert_eq!(state_of(&[ours, other]), HandlerState::Other);
    }

    #[test]
    fn the_entry_is_written_where_asked() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("applications");
        write_entry(&dir, Path::new("/x y/heronite-rs")).unwrap();
        assert!(std::fs::read_to_string(dir.join(DESKTOP_FILE)).unwrap().contains("Exec=\"/x y/heronite-rs\" %u"));
    }
}
