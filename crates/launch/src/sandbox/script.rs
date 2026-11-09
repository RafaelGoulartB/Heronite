//! The `sh` program that starts a command in the sandbox. Built here (pure,
//! tested); what depends on the session is looked up when it runs, because
//! the wrappers before it change it: gamescope gives the game its own
//! display, and controllers come and go between launches.
//!
//! One line, without newlines: legendary gets it inside `--wrapper`.

use super::mounts::{Access, Mounts, SHADER_CACHES, SYSTEM};
use super::{Bwrap, Sandbox};

/// `$0` of the program: names it in `ps` and marks a wrapped command.
pub(super) const NAME: &str = "heronite-sandbox";

/// Runs the command, then waits until every process of the sandbox is gone:
/// the pid namespace ends with its first process, which would kill a game a
/// launcher started (or a wineserver still saving the registry).
const WAIT_ALL: &str = r#""$@"; rc=$?; while :; do n=; for p in /proc/[0-9]*; do case ${p#/proc/} in 1|$$) ;; *) n=1; break;; esac; done; [ -z "$n" ] && exit $rc; sleep 1; done"#;

/// The session's runtime folder, its Wayland socket and the X11 cookie.
const SESSION: &str = r#"r=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}; w=/nonexistent; case ${WAYLAND_DISPLAY-} in '') ;; /*) w=$WAYLAND_DISPLAY;; *) w=$r/$WAYLAND_DISPLAY;; esac; x=${XAUTHORITY:-/nonexistent}; "#;

/// `/bin`, `/lib`... as the host has them: links into `/usr` or folders.
const ROOT_LINKS: &str = r#"o=; for l in bin sbin lib lib32 lib64 libx32; do if [ -L "/$l" ]; then o="$o --symlink $(readlink "/$l") /$l"; elif [ -d "/$l" ]; then o="$o --ro-bind /$l /$l"; fi; done; "#;

/// NVIDIA's devices and the game controllers plugged in: joysticks by udev,
/// and the `js` / `hidraw` nodes the user may open (udev gives controllers
/// to the logged-in user; keyboards and mice stay out).
const DEVICES: &str = r#"v=; for f in /dev/nvidia*; do [ -e "$f" ] && v="$v --dev-bind $f $f"; done; for f in /dev/input/event*; do [ -r "$f" ] || continue; m=$(stat -c %T "$f" 2>/dev/null) || continue; grep -qs '^E:ID_INPUT_JOYSTICK=1' "/run/udev/data/c13:$((0x$m))" && v="$v --dev-bind $f $f"; done; for f in /dev/input/js*; do [ -r "$f" ] && v="$v --dev-bind $f $f"; done; for f in /dev/hidraw*; do [ -r "$f" ] && [ -w "$f" ] && v="$v --dev-bind $f $f"; done; "#;

/// With the network: the resolver's folder when `/etc/resolv.conf` points
/// into `/run` (systemd-resolved, NetworkManager).
const RESOLVER: &str = r#"n=; c=$(readlink -f /etc/resolv.conf); case $c in /run/*) n="--ro-bind-try ${c%/*} ${c%/*}";; esac; "#;

/// Variables naming what the sandbox hides (the session bus, the SSH and
/// keyring agents) or folders outside it.
pub(super) const UNSET: [&str; 13] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "DBUS_SYSTEM_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "GPG_AGENT_INFO",
    "GNOME_KEYRING_CONTROL",
    "SESSION_MANAGER",
    "KRB5CCNAME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "TMPDIR",
];

fn q(s: &str) -> String {
    shlex::try_quote(s).map(|c| c.into_owned()).unwrap_or_else(|_| "''".into())
}

/// `["/bin/sh", "-c", <program>, NAME]`: the command to run follows.
pub(super) fn prelude(bwrap: &Bwrap, sandbox: &Sandbox, m: &Mounts) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), program(bwrap, sandbox, m), NAME.into()]
}

/// How many of a wrapped command's arguments are the prelude's (`-c`, the
/// program, `NAME`): the command's own follow.
pub(super) const ARGS: usize = 3;

/// Whether `program` and `args` start as [`prelude`] makes them.
pub(super) fn starts_with_prelude(program: &str, args: &[String]) -> bool {
    program == "/bin/sh" && args.len() >= ARGS && args[0] == "-c" && args[ARGS - 1] == NAME
}

fn program(bwrap: &Bwrap, sandbox: &Sandbox, m: &Mounts) -> String {
    let mut s = String::new();
    if !m.make.is_empty() {
        let dirs: Vec<String> = m.make.iter().map(|d| q(d)).collect();
        s += &format!("for d in {}; do mkdir -p -- \"$d\" || exit 126; done; ", dirs.join(" "));
    }
    s += SESSION;
    s += ROOT_LINKS;
    if !sandbox.devices {
        s += DEVICES;
    }
    if sandbox.network {
        s += RESOLVER;
    }
    if !m.home.is_empty() {
        for (i, (var, dir)) in SHADER_CACHES.iter().enumerate() {
            s += &format!("k{i}=${{{var}:-{}}}; ", q(&format!("{}/{dir}", m.home)));
        }
    }
    let mut a: Vec<String> = vec![q(&bwrap.path), "--unshare-user".into(), "--unshare-pid".into(), "--unshare-uts".into()];
    a.push("--unshare-cgroup-try".into());
    if !sandbox.network {
        a.push("--unshare-net".into());
    }
    // A program could type into the terminal the app was started from.
    a.push("--new-session".into());
    a.extend(["--ro-bind".into(), "/usr".into(), "/usr".into(), "$o".into()]);
    for d in SYSTEM.iter().skip(1) {
        let bind = if *d == "/etc" { "--ro-bind" } else { "--ro-bind-try" };
        a.extend([bind.into(), (*d).into(), (*d).into()]);
    }
    a.extend(["--ro-bind".into(), "/sys".into(), "/sys".into(), "--proc".into(), "/proc".into()]);
    if sandbox.devices {
        a.extend(["--dev-bind".into(), "/dev".into(), "/dev".into()]);
    } else {
        a.extend(["--dev".into(), "/dev".into(), "--dev-bind-try".into(), "/dev/dri".into(), "/dev/dri".into(), "$v".into()]);
    }
    a.extend(["--tmpfs".into(), "/tmp".into(), "--ro-bind-try".into(), "/tmp/.X11-unix".into(), "/tmp/.X11-unix".into()]);
    a.extend(["--tmpfs".into(), "/run".into()]);
    for d in ["/run/udev", "/run/opengl-driver"] {
        a.extend(["--ro-bind-try".into(), d.into(), d.into()]);
    }
    if sandbox.network {
        a.push("$n".into());
    }
    a.extend(["--perms".into(), "0700".into(), "--dir".into(), "\"$r\"".into()]);
    for sock in ["\"$w\"", "\"$r/pipewire-0\"", "\"$r/pulse/native\"", "\"$r/heronite/mangohud\""] {
        a.extend(["--ro-bind-try".into(), sock.into(), sock.into()]);
    }
    a.extend(["--setenv".into(), "XDG_RUNTIME_DIR".into(), "\"$r\"".into()]);
    if !m.home.is_empty() {
        a.extend(["--bind".into(), q(&m.private_home), q(&m.home)]);
        // `k<n>`: the user's own path, else the folder in the home (looked
        // at when the program runs, see below).
        for (i, (var, _)) in SHADER_CACHES.iter().enumerate() {
            a.extend(["--setenv".into(), (*var).into(), format!("\"$k{i}\"")]);
        }
    }
    // The X11 cookie after the home: it may be anywhere in it.
    a.extend(["--ro-bind-try".into(), "\"$x\"".into(), "\"$x\"".into()]);
    for b in &m.binds {
        let (src, dest) = (q(b.source.as_deref().unwrap_or(&b.path)), q(&b.path));
        match (b.access, b.required) {
            (Access::Throwaway, _) => a.extend(["--overlay-src".into(), src, "--tmp-overlay".into(), dest]),
            (Access::ReadWrite, true) => a.extend(["--bind".into(), src, dest]),
            (Access::ReadWrite, false) => a.extend(["--bind-try".into(), src, dest]),
            (Access::ReadOnly, _) => a.extend(["--ro-bind-try".into(), src, dest]),
        }
    }
    for var in UNSET {
        a.extend(["--unsetenv".into(), var.into()]);
    }
    a.extend(["--setenv".into(), super::ENV_INSIDE.into(), "1".into()]);
    a.extend(["--".into(), "/bin/sh".into(), "-c".into(), q(WAIT_ALL), "sh".into(), "\"$@\"".into()]);
    s += "exec ";
    s += &a.join(" ");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::mounts::Bind;

    fn mounts() -> Mounts {
        Mounts {
            home: "/home/u".into(),
            private_home: "/home/u/.local/share/heronite/sandbox/prefix-My Game-0000abcd".into(),
            binds: vec![
                Bind { path: "/home/u/.cache/winetricks".into(), access: Access::Throwaway, required: false, source: None },
                Bind { path: "/home/u/Games/My Game".into(), access: Access::ReadOnly, required: false, source: None },
                Bind { path: "/home/u/Games/My Game/pfx".into(), access: Access::ReadWrite, required: true, source: None },
            ],
            make: vec!["/home/u/.local/share/heronite/sandbox/prefix-My Game-0000abcd".into(), "/home/u/Games/My Game/pfx".into()],
        }
    }

    fn bwrap() -> Bwrap {
        Bwrap { path: "/usr/bin/bwrap".into(), version: (0, 11, 0), error: None, overlay: true }
    }

    #[test]
    fn the_program_is_one_line_that_runs_bwrap_with_the_command() {
        let p = prelude(&bwrap(), &Sandbox { enabled: true, ..Default::default() }, &mounts());
        assert_eq!(p[..2], ["/bin/sh", "-c"]);
        assert_eq!(p[3], NAME);
        let s = &p[2];
        assert!(!s.contains('\n'));
        assert!(s.starts_with(
            "for d in '/home/u/.local/share/heronite/sandbox/prefix-My Game-0000abcd' '/home/u/Games/My Game/pfx'; do mkdir -p"
        ));
        assert!(s.contains("exec /usr/bin/bwrap --unshare-user --unshare-pid"));
        assert!(s.contains("--unshare-net --new-session"));
        assert!(s.contains("--bind '/home/u/.local/share/heronite/sandbox/prefix-My Game-0000abcd' /home/u"));
        assert!(s.contains("--overlay-src /home/u/.cache/winetricks --tmp-overlay /home/u/.cache/winetricks"));
        assert!(s.contains("--ro-bind-try '/home/u/Games/My Game' '/home/u/Games/My Game'"));
        assert!(s.contains("--bind '/home/u/Games/My Game/pfx' '/home/u/Games/My Game/pfx'"));
        assert!(s.contains("--unsetenv DBUS_SESSION_BUS_ADDRESS"));
        assert!(s.contains("--dev /dev --dev-bind-try /dev/dri /dev/dri $v"));
        assert!(s.ends_with(r#"sh "$@""#));
        // Nothing of the network part.
        assert!(!s.contains("resolv.conf"));
    }

    #[test]
    fn exceptions_change_the_program() {
        let open = Sandbox { enabled: true, network: true, devices: true, ..Default::default() };
        let s = &prelude(&bwrap(), &open, &mounts())[2];
        assert!(!s.contains("--unshare-net"));
        assert!(s.contains("readlink -f /etc/resolv.conf"));
        assert!(s.contains("--dev-bind /dev /dev"));
        assert!(!s.contains("/dev/input/event*"));
    }

    /// The program runs: `sh -n` accepts it.
    #[cfg(unix)]
    #[test]
    fn the_program_is_valid_shell() {
        let open = Sandbox { enabled: true, network: true, ..Default::default() };
        for sb in [Sandbox { enabled: true, ..Default::default() }, open] {
            let p = prelude(&bwrap(), &sb, &mounts());
            let out = std::process::Command::new("/bin/sh").arg("-n").arg("-c").arg(&p[2]).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
    }

    /// The program runs: with a `bwrap` that prints what it is given, what
    /// it looks up when it runs (the folders it makes, the session, the
    /// shader caches) comes out as arguments, and the command follows.
    #[cfg(unix)]
    #[test]
    fn the_program_runs_and_hands_its_arguments_to_bwrap() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let fake = dir.path().join("bwrap");
        std::fs::write(&fake, "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let b = Bwrap { path: fake.to_string_lossy().into_owned(), ..bwrap() };
        let home = format!("{root}/home dir");
        let private = format!("{root}/private home");
        let m = Mounts {
            home: home.clone(),
            private_home: private.clone(),
            make: vec![private.clone(), format!("{private}/.cache/dxvk")],
            ..Default::default()
        };
        let p = prelude(&b, &Sandbox { enabled: true, ..Default::default() }, &m);
        let run = |extra: &[(&str, &str)]| {
            let out = std::process::Command::new(&p[0])
                .args(&p[1..])
                .args(["/usr/bin/game", "an arg"])
                .env("XDG_RUNTIME_DIR", "/run/user/1000")
                .env("WAYLAND_DISPLAY", "wayland-1")
                .env_remove("DXVK_STATE_CACHE_PATH")
                .envs(extra.iter().copied())
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap().lines().map(str::to_string).collect::<Vec<_>>()
        };
        let args = run(&[]);
        let has = |w: &[&str]| args.windows(w.len()).any(|a| a == w);
        assert!(std::path::Path::new(&format!("{private}/.cache/dxvk")).is_dir(), "made before bwrap");
        assert!(has(&["--bind", &private, &home]), "{args:?}");
        assert!(has(&["--dir", "/run/user/1000"]));
        assert!(has(&["--ro-bind-try", "/run/user/1000/wayland-1", "/run/user/1000/wayland-1"]));
        assert!(has(&["--setenv", "DXVK_STATE_CACHE_PATH", &format!("{home}/.cache/dxvk")]));
        assert!(has(&["--setenv", "VKD3D_SHADER_CACHE_PATH", &format!("{home}/.cache/vkd3d-proton")]));
        assert!(has(&["--setenv", super::super::ENV_INSIDE, "1"]));
        assert_eq!(args[args.len() - 6..], ["/bin/sh", "-c", WAIT_ALL, "sh", "/usr/bin/game", "an arg"]);
        // A cache folder the user set is kept.
        let args = run(&[("DXVK_STATE_CACHE_PATH", "/mine")]);
        assert!(args.windows(3).any(|a| a == ["--setenv", "DXVK_STATE_CACHE_PATH", "/mine"]), "{args:?}");
    }
}
