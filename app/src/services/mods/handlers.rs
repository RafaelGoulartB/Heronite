//! Who opens `nxm://` (Nexus Mods) and `ror2mm://` (Thunderstore) links.
//! Linux: the desktop entry that makes Heronite do it, and `xdg-mime` to read
//! and set it. Windows: the schemes under `HKCU\Software\Classes`, for this
//! user only. Either way they are only written when the user asks for it
//! (Settings → Mods, the manager's first-run card), never at start.
//! All of it blocks: it runs off the UI thread.

use gpui_kit::*;

use super::{ModsEvent, ModsService};

#[cfg(not(windows))]
use xdg::{read_state, register};

#[cfg(not(windows))]
mod xdg {
    use std::path::Path;
    use std::process::{Command, Stdio};

    use heronite_i18n::tr;
    use heronite_shortcuts::desktop::{quote_exec_arg, Roots};

    use crate::services::mods::HandlerState;

    /// The entry of ours that opens the links.
    pub(super) const DESKTOP_FILE: &str = "com.heronite.rs-mods.desktop";
    const SCHEMES: [&str; 2] = ["x-scheme-handler/nxm", "x-scheme-handler/ror2mm"];

    pub(super) fn entry_text(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Heronite (mods)\nComment=Opens the mod links of Nexus Mods and Thunderstore\nExec={} %u\nIcon=com.heronite.rs\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/nxm;x-scheme-handler/ror2mm;\n",
            quote_exec_arg(&exe.to_string_lossy())
        )
    }

    /// What `xdg-mime query default` printed: the entry's file name, or nothing.
    pub(super) fn parse_default(output: &str) -> Option<String> {
        output.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
    }

    /// `nxm` and `ror2mm` handlers together: ours only when both are.
    pub(super) fn state_of(handlers: &[Option<String>]) -> HandlerState {
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

    pub(in crate::services::mods) fn read_state() -> HandlerState {
        let handlers: Vec<Option<String>> = SCHEMES.iter().map(|s| query_default(s)).collect();
        state_of(&handlers)
    }

    pub(super) fn write_entry(applications: &Path, exe: &Path) -> Result<(), String> {
        let file = applications.join(DESKTOP_FILE);
        std::fs::create_dir_all(applications)
            .and_then(|()| std::fs::write(&file, entry_text(exe)))
            .map_err(|e| tr!("Could not write {path}: {error}", path = file.display(), error = e))
    }

    /// Writes the entry and makes it the default for both schemes.
    pub(in crate::services::mods) fn register(exe: &Path) -> Result<(), String> {
        let applications = Roots::detect().applications;
        write_entry(&applications, exe)?;
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
        let _ = Command::new("update-desktop-database")
            .arg(&applications)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
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
}

#[cfg(windows)]
use classes::{read_state, register};

/// Windows: `HKCU\Software\Classes\<scheme>` with `URL Protocol` and
/// `shell\open\command`, like `system/protocol.rs` does for `heronite://`.
/// What to write and how to read it back are plain functions, so the tests
/// never touch the registry.
#[cfg_attr(not(windows), allow(dead_code))]
mod classes {
    use std::path::Path;

    use crate::services::mods::HandlerState;

    pub(super) const SCHEMES: [(&str, &str); 2] = [("nxm", "URL:Nexus Mods"), ("ror2mm", "URL:Thunderstore Mod Manager")];

    /// The command line Windows runs for a link.
    pub(super) fn command(exe: &Path) -> String {
        format!("\"{}\" \"%1\"", exe.display())
    }

    /// `(key under HKEY_CURRENT_USER, value name, value)` for one scheme.
    pub(super) fn values(scheme: &str, label: &str, exe: &Path) -> Vec<(String, &'static str, String)> {
        let key = format!(r"Software\Classes\{scheme}");
        vec![
            (key.clone(), "", label.to_string()),
            (key.clone(), "URL Protocol", String::new()),
            (format!(r"{key}\DefaultIcon"), "", format!("\"{}\",0", exe.display())),
            (format!(r"{key}\shell\open\command"), "", command(exe)),
        ]
    }

    /// The schemes' commands together: ours only when both run this
    /// executable (another copy of Heronite counts as another program).
    pub(super) fn state_of(commands: &[Option<String>], exe: &Path) -> HandlerState {
        let ours_cmd = command(exe).to_lowercase();
        let ours = |c: &Option<String>| c.as_deref().is_some_and(|c| c.trim().to_lowercase() == ours_cmd);
        if commands.iter().all(ours) {
            HandlerState::Ours
        } else if commands.iter().any(|c| c.as_deref().is_some_and(|c| !c.trim().is_empty()) && !ours(c)) {
            HandlerState::Other
        } else {
            HandlerState::None
        }
    }

    #[cfg(windows)]
    pub(in crate::services::mods) fn read_state() -> HandlerState {
        use heronite_platform::registry::{string, Hive};
        let exe = crate::system::shortcuts::own_exe();
        let commands: Vec<Option<String>> =
            SCHEMES.iter().map(|(s, _)| string(Hive::CurrentUser, &format!(r"Software\Classes\{s}\shell\open\command"), "")).collect();
        state_of(&commands, &exe)
    }

    #[cfg(windows)]
    pub(in crate::services::mods) fn register(exe: &Path) -> Result<(), String> {
        use heronite_platform::registry::{set_string, Hive};
        for (scheme, label) in SCHEMES {
            for (key, name, value) in values(scheme, label, exe) {
                set_string(Hive::CurrentUser, &key, name, &value).map_err(|e| {
                    heronite_i18n::tr!("Could not set Heronite for the {scheme} links: {error}", scheme = scheme, error = e)
                })?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn each_scheme_opens_this_executable_with_the_link() {
            let exe = Path::new(r"C:\Program Files\Heronite\heronite-rs.exe");
            let v = values("nxm", "URL:Nexus Mods", exe);
            assert_eq!(v[0], (r"Software\Classes\nxm".to_string(), "", "URL:Nexus Mods".to_string()));
            assert_eq!(v[1], (r"Software\Classes\nxm".to_string(), "URL Protocol", String::new()));
            assert_eq!(v[3].0, r"Software\Classes\nxm\shell\open\command");
            assert_eq!(v[3].2, r#""C:\Program Files\Heronite\heronite-rs.exe" "%1""#);
            assert!(v.iter().all(|(key, _, _)| key.starts_with(r"Software\Classes\")), "per user, never HKLM");
        }

        #[test]
        fn the_state_needs_both_schemes_to_run_this_executable() {
            let exe = Path::new(r"C:\Apps\heronite-rs.exe");
            let ours = Some(r#""c:\apps\HERONITE-RS.exe" "%1""#.to_string());
            let vortex = Some(r#""C:\Program Files\Black Tree Gaming Ltd\Vortex\Vortex.exe" -d "%1""#.to_string());
            assert_eq!(state_of(&[ours.clone(), ours.clone()], exe), HandlerState::Ours);
            assert_eq!(state_of(&[ours.clone(), None], exe), HandlerState::None);
            assert_eq!(state_of(&[None, None], exe), HandlerState::None);
            assert_eq!(state_of(&[vortex.clone(), None], exe), HandlerState::Other);
            assert_eq!(state_of(&[ours, vortex], exe), HandlerState::Other);
            let elsewhere = Some(r#""D:\Old\heronite-rs.exe" "%1""#.to_string());
            assert_eq!(state_of(&[elsewhere.clone(), elsewhere], exe), HandlerState::Other, "another copy of Heronite");
        }
    }
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

    /// Only from the user's own click: makes Heronite open the mod links.
    pub(super) fn write_handlers(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async {
                    register(&crate::system::shortcuts::own_exe())?;
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
