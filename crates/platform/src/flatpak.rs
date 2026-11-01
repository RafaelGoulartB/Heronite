//! Running as a Flatpak (`packaging/flatpak`).
//!
//! The app then lives in a sandbox: `/usr` is the runtime's, the data,
//! settings and caches go to `~/.var/app/<id>`, and a program of the host
//! (another Flatpak, Steam) is out of reach except through
//! `flatpak-spawn --host` ([`host`]). What the sandbox cannot do at all
//! (bubblewrap inside it, the app replacing its own files) is off in
//! [`crate::features`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The file Flatpak puts at the root of every sandbox.
const INFO_FILE: &str = "/.flatpak-info";

/// What `/.flatpak-info` says about this sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Info {
    /// The application id (`[Application] name`).
    id: String,
    /// Where the app's files are on the host (`[Instance] app-path`):
    /// `<installation>/app/<id>/<arch>/<branch>/<commit>/files`.
    app_path: Option<PathBuf>,
}

fn info() -> Option<&'static Info> {
    static INFO: OnceLock<Option<Info>> = OnceLock::new();
    INFO.get_or_init(|| if cfg!(target_os = "linux") { std::fs::read_to_string(INFO_FILE).ok().map(|t| parse_info(&t)) } else { None })
        .as_ref()
}

/// The keys of `/.flatpak-info` this module reads (a GKeyFile).
fn parse_info(text: &str) -> Info {
    let mut info = Info::default();
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match (section, key.trim()) {
            ("Application", "name") => info.id = value.trim().to_string(),
            ("Instance", "app-path") => info.app_path = Some(PathBuf::from(value.trim())),
            _ => {}
        }
    }
    info
}

/// Whether this process runs inside a Flatpak sandbox.
pub fn active() -> bool {
    info().is_some()
}

/// `cmd` as the host runs it: inside the sandbox, through
/// `flatpak-spawn --host` (the manifest lets the app talk to
/// `org.freedesktop.Flatpak`), with the variables and working folder `cmd`
/// sets; elsewhere `cmd` itself. The host starts it with the session's own
/// environment, not this process's. Call it before anything that cannot be
/// carried over (`pre_exec`, stdio).
pub fn host(cmd: Command) -> Command {
    if !active() {
        return cmd;
    }
    let mut out = Command::new("flatpak-spawn");
    out.args(host_args(&cmd));
    out
}

/// The arguments of `flatpak-spawn` that run `cmd` on the host.
fn host_args(cmd: &Command) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--host".into(), "--watch-bus".into()];
    if let Some(dir) = cmd.get_current_dir() {
        let mut arg = OsString::from("--directory=");
        arg.push(dir);
        args.push(arg);
    }
    for (key, value) in cmd.get_envs() {
        let mut arg = OsString::from(if value.is_some() { "--env=" } else { "--unset-env=" });
        arg.push(key);
        if let Some(value) = value {
            arg.push("=");
            arg.push(value);
        }
        args.push(arg);
    }
    args.push(cmd.get_program().into());
    args.extend(cmd.get_args().map(Into::into));
    args
}

/// The host's command that starts this app, for the entries the host opens
/// (game shortcuts, the autostart entry, Steam shortcuts): the launcher
/// Flatpak exports for it, `<installation>/exports/bin/<id>`. `None` outside
/// a sandbox, or when the sandbox does not say where it is installed.
pub fn launcher() -> Option<PathBuf> {
    let info = info()?;
    launcher_of(&info.id, info.app_path.as_deref()?)
}

fn launcher_of(id: &str, app_path: &Path) -> Option<PathBuf> {
    if id.is_empty() {
        return None;
    }
    // The installation is the folder above `app/<id>`.
    let mut installation = app_path;
    loop {
        let parent = installation.parent()?;
        if installation.file_name()? == id && parent.file_name()? == "app" {
            return Some(parent.parent()?.join("exports").join("bin").join(id));
        }
        installation = parent;
    }
}

/// The host's own XDG folder that `HOST_<var>` names (Flatpak sets it when
/// the host sets `<var>`), else `$HOME/<fallback>`: inside the sandbox
/// `$XDG_DATA_HOME` and `$XDG_CONFIG_HOME` point into `~/.var/app/<id>`,
/// which the host's menu and autostart never read.
pub fn host_xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    let set = std::env::var_os(format!("HOST_{var}")).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
    set.or_else(|| crate::dirs::home().map(|h| h.join(fallback)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_id_and_where_the_app_is() {
        let text = "[Application]\nname=com.heronite.rs\nruntime=runtime/org.gnome.Platform/x86_64/51\n\n[Instance]\ninstance-id=1\napp-path=/var/lib/flatpak/app/com.heronite.rs/x86_64/stable/abc/files\nbranch=stable\n";
        let info = parse_info(text);
        assert_eq!(info.id, "com.heronite.rs");
        assert_eq!(info.app_path.as_deref(), Some(Path::new("/var/lib/flatpak/app/com.heronite.rs/x86_64/stable/abc/files")));
        assert_eq!(parse_info("[Instance]\nname=x\n"), Info::default(), "keys of another section");
    }

    #[test]
    fn the_launcher_is_in_the_installations_exports() {
        let system = Path::new("/var/lib/flatpak/app/com.heronite.rs/x86_64/stable/abc/files");
        assert_eq!(launcher_of("com.heronite.rs", system), Some(PathBuf::from("/var/lib/flatpak/exports/bin/com.heronite.rs")));
        let user = Path::new("/home/u/.local/share/flatpak/app/com.heronite.rs/x86_64/master/abc/files");
        assert_eq!(launcher_of("com.heronite.rs", user), Some(PathBuf::from("/home/u/.local/share/flatpak/exports/bin/com.heronite.rs")));
        // `flatpak-builder --run`: the build folder, not an installation.
        assert_eq!(launcher_of("com.heronite.rs", Path::new("/src/.flatpak-builder/build/files")), None);
        assert_eq!(launcher_of("", system), None);
    }

    #[test]
    fn host_commands_carry_their_folder_and_variables() {
        let mut cmd = Command::new("flatpak");
        cmd.args(["run", "org.DolphinEmu.dolphin-emu"]).current_dir("/games").env("A", "1").env_remove("B");
        let args: Vec<String> = host_args(&cmd).into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(
            args,
            ["--host", "--watch-bus", "--directory=/games", "--env=A=1", "--unset-env=B", "flatpak", "run", "org.DolphinEmu.dolphin-emu"]
        );
    }
}
