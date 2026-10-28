//! The environment of a sandboxed program. The sandbox starts it with an
//! empty one (`--clearenv`) and hands over only what the program needs: the
//! variables the launch plan itself sets, and the ones of the session that
//! Wine, Proton, the graphics drivers and the sound server read. Anything
//! else the launcher was started with (tokens, API keys, agent sockets)
//! stays out, so a game cannot read it, or send it away with the network.
//!
//! The environment is looked at when the program runs, in the same `sh`
//! that looks up the session: the wrappers before it (gamescope, GameMode)
//! change it.

/// Variables of the launcher's environment that pass. `NAME*`: every
/// variable that starts so. None of them holds a secret.
const ALLOWED: &[&str] = &[
    // The user and the system.
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "TZ",
    "TZDIR",
    "LANG",
    "LANGUAGE",
    "LC_*",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    // The session.
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XDG_SESSION_DESKTOP",
    "XDG_CURRENT_DESKTOP",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "XCURSOR_*",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "QT_*",
    "GDK_*",
    // Sound and controllers.
    "PULSE_*",
    "PIPEWIRE_*",
    "SDL_*",
    // Graphics drivers, Vulkan layers and translation layers.
    "__GL_*",
    "__GLX_*",
    "__NV_*",
    "__VK_*",
    "__EGL_*",
    "DRI_*",
    "EGL_*",
    "GBM_*",
    "LIBGL_*",
    "MESA_*",
    "mesa_glthread",
    "vblank_mode",
    "AMD_*",
    "RADV_*",
    "VK_*",
    "ENABLE_*",
    "DISABLE_*",
    "DXVK_*",
    "VKD3D_*",
    "VKBASALT*",
    "GAMESCOPE_*",
    "MANGOHUD*",
    "LSFG*",
    // Wine, Proton, umu and the stores.
    "WINE*",
    "PROTON*",
    "STEAM_COMPAT_*",
    "STEAM_RUNTIME*",
    "SteamAppId",
    "SteamGameId",
    "SteamDeck",
    "UMU_*",
    "GAMEID",
    "STORE",
    "HEROIC_*",
    "DOTNET_*",
];

/// Variables that never pass, not even when the plan sets them: they name
/// what the sandbox hides (the session bus, the SSH and keyring agents) or
/// folders outside it.
pub(super) const HIDDEN: [&str; 13] = [
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

/// After [`collect`] and the arguments that end the command: puts the
/// program's own arguments (the first `cnt`) back at the end.
pub(super) const ROTATE: &str = r#"while [ $cnt -gt 0 ]; do set -- "$@" "$1"; shift; cnt=$((cnt-1)); done; "#;

fn is_name(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `sh` that appends to the positional parameters `emit` (a word list that
/// can use `$y`, the name, and `$z`, the value) for each variable in the
/// environment that [`ALLOWED`] lets through, or that the launch plan sets
/// (`plan`), except [`HIDDEN`] and `skip`. Starts counting the program's own
/// arguments in `cnt`: [`ROTATE`] ends it.
///
/// The names come from `env` and `sed` and are only identifiers; the values
/// come from `printenv`, so a value with a line that looks like `NAME=...`
/// cannot make a variable up.
pub(super) fn collect<'a>(plan: impl IntoIterator<Item = &'a str>, skip: &[&str], emit: &str) -> String {
    let names: Vec<&str> = plan.into_iter().filter(|n| is_name(n)).collect();
    let allowed: Vec<&str> = ALLOWED.iter().copied().filter(|n| !skip.contains(n)).collect();
    format!(
        "cnt=$#; p=' {} '; for y in $(env | sed -n 's/^\\([A-Za-z_][A-Za-z0-9_]*\\)=.*/\\1/p'); do \
         case $y in {}) continue;; esac; \
         case $y in {}) ;; *) case $p in *\" $y \"*) ;; *) continue;; esac;; esac; \
         z=$(printenv \"$y\") || continue; set -- \"$@\" {emit}; done; ",
        names.join(" "),
        HIDDEN.join("|"),
        allowed.join("|"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `collect` in `sh` with `env` as the whole environment, for
    /// the arguments `args`; what it left in `"$@"`.
    fn run(plan: &[&str], skip: &[&str], env: &[(&str, &str)], args: &[&str]) -> Vec<String> {
        let script = format!("{}{}exec printf '%s\\n' \"$@\"", collect(plan.iter().copied(), skip, r#"--setenv "$y" "$z""#), ROTATE);
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .arg("sh")
            .args(args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().lines().map(str::to_string).collect()
    }

    fn has(out: &[String], w: &[&str]) -> bool {
        out.windows(w.len()).any(|a| a == w)
    }

    #[test]
    fn the_session_passes_and_secrets_do_not() {
        let env = [
            ("DISPLAY", ":0"),
            ("LC_ALL", "C"),
            ("PROTON_LOG", "1"),
            ("DXVK_HUD", "fps"),
            ("GH_TOKEN", "ghp_secret"),
            ("AWS_SECRET_ACCESS_KEY", "aws_secret"),
            ("SSH_AUTH_SOCK", "/run/user/1000/ssh"),
            ("XDG_DATA_HOME", "/home/u/.local/share"),
        ];
        let out = run(&[], &[], &env, &["game", "an arg"]);
        assert!(has(&out, &["--setenv", "DISPLAY", ":0"]), "{out:?}");
        assert!(has(&out, &["--setenv", "LC_ALL", "C"]));
        assert!(has(&out, &["--setenv", "PROTON_LOG", "1"]));
        assert!(has(&out, &["--setenv", "DXVK_HUD", "fps"]));
        for secret in ["GH_TOKEN", "ghp_secret", "AWS_SECRET_ACCESS_KEY", "aws_secret", "SSH_AUTH_SOCK", "XDG_DATA_HOME"] {
            assert!(!out.iter().any(|a| a == secret), "{secret} passed: {out:?}");
        }
        assert_eq!(out[out.len() - 2..], ["game", "an arg"], "the program's arguments stay last");
    }

    #[test]
    fn what_the_plan_sets_passes_whatever_its_name() {
        let env = [("MY_GAME_OPTION", "a b"), ("GH_TOKEN", "x"), ("EMPTY_ONE", ""), ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/x")];
        let out = run(&["MY_GAME_OPTION", "EMPTY_ONE", "DBUS_SESSION_BUS_ADDRESS", "NOT-A-NAME", "NOT_SET"], &[], &env, &["game"]);
        assert!(has(&out, &["--setenv", "MY_GAME_OPTION", "a b"]), "{out:?}");
        assert!(has(&out, &["--setenv", "EMPTY_ONE", ""]), "set but empty passes");
        assert!(!out.iter().any(|a| a == "GH_TOKEN" || a == "NOT_SET" || a == "DBUS_SESSION_BUS_ADDRESS"), "{out:?}");
        assert_eq!(out.last().map(String::as_str), Some("game"));
    }

    #[test]
    fn skipped_names_and_values_that_look_like_variables_do_not_pass() {
        let out = run(&[], &["HOME"], &[("HOME", "/home/u"), ("PROTON_NOTE", "x\nGH_TOKEN=y\nPATH_X=z")], &[]);
        assert!(!has(&out, &["--setenv", "HOME"]), "{out:?}");
        assert!(has(&out, &["--setenv", "PROTON_NOTE", "x"]), "{out:?}");
        assert!(!has(&out, &["--setenv", "GH_TOKEN"]) && !has(&out, &["--setenv", "PATH_X"]), "{out:?}");
    }

    #[test]
    fn the_program_is_valid_shell() {
        let s = collect(["A"], &[], "x");
        let out = std::process::Command::new("/bin/sh").arg("-n").arg("-c").arg(&s).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
