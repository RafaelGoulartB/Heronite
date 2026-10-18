//! The `task` steps (Wine and DOSBox) through `heronite_launch`'s Wine
//! commands, so a script's setup runs exactly like the game will: same
//! Wine/Proton, umu, prefix and environment.
//!
//! A Windows setup program may start another one and exit; the step only
//! ends once every program of the prefix has (`wineserver -w`, or umu, which
//! returns when its container is empty). Waiting also matters because what
//! is left of a finished command's process group is killed: a wineserver
//! killed that way loses the registry changes it had not saved yet.

use std::path::Path;
use std::process::Command;

use heronite_data::game_config;
use heronite_i18n::tr;
use heronite_launch::settings::GameSettings;
use heronite_launch::wine_tools::{self, WineTool};
use heronite_launch::{Probe, SystemProbe};
use serde_json::{json, Value};

use crate::engine::{split_args, State};
use crate::steps::Task;
use crate::text::get_nonempty;

/// Runs the command (program and arguments after `$0`), then waits for the
/// prefix's wineserver.
const WAIT_SCRIPT: &str = "\"$0\" \"$@\"; rc=$?; WINEPREFIX=\"$HERONITE_WAIT_PREFIX\" \"$HERONITE_WAIT_SERVER\" -w; exit $rc";

fn std_command(c: &heronite_launch::Command, wait: Option<&heronite_launch::Command>) -> Command {
    let mut cmd = match wait {
        Some(w) => {
            let mut s = Command::new("sh");
            s.arg("-c").arg(WAIT_SCRIPT).arg(&c.program).args(&c.args);
            s.env("HERONITE_WAIT_SERVER", &w.program);
            s.env("HERONITE_WAIT_PREFIX", w.env.get("WINEPREFIX").map(String::as_str).unwrap_or(""));
            s
        }
        None => {
            let mut s = Command::new(&c.program);
            s.args(&c.args);
            s
        }
    };
    cmd.envs(&c.env);
    heronite_launch::wine::drop_host_certs(c, &mut cmd);
    if let Some(dir) = c.cwd.as_deref().filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    cmd
}

/// `set_regedit`'s value as a `.reg` file writes it (`None` deletes it).
pub fn reg_value(kind: &str, value: Option<&str>) -> Result<String, String> {
    let Some(v) = value else { return Ok("-".into()) };
    Ok(match kind.trim().to_ascii_uppercase().as_str() {
        "REG_SZ" => format!("\"{v}\""),
        "REG_DWORD" => format!("dword:{v}"),
        "REG_BINARY" => format!("hex:{}", v.replace(' ', ",")),
        "REG_MULTI_SZ" => format!("hex(2):{v}"),
        "REG_EXPAND_SZ" => format!("hex(7):{v}"),
        other => return Err(tr!("Unknown registry value type {kind}", kind = other)),
    })
}

/// `HKEY_CURRENT_USER/Software/Wine` -> `HKEY_CURRENT_USER\Software\Wine`.
fn reg_path(path: &str) -> String {
    path.trim().replace('/', "\\")
}

impl State<'_> {
    /// The settings a task runs with: the new game's defaults, the chosen
    /// Wine, `prefix` (default: the game's), the script's environment and
    /// the task's own.
    pub(crate) fn wine_settings(&self, prefix: Option<&str>, env: &[(String, String)]) -> Result<GameSettings, String> {
        let job = self.job;
        let version = job.wine_version.clone().ok_or_else(|| tr!("No Wine or Proton version was chosen"))?;
        let prefix = match prefix {
            Some(p) => self.path(p).to_string_lossy().into_owned(),
            None => crate::runners::prefix_of(&job.script.game, &|s| self.sub(s), &job.game_dir),
        };
        let mut map = game_config::load_game(&job.paths, &job.app_name, &job.home);
        map.insert("wineVersion".into(), version);
        map.insert("winePrefix".into(), json!(prefix));
        let mut s = GameSettings::from_map(&map, &job.home);
        s.env_options.extend(self.script_env());
        s.env_options.extend(env.iter().map(|(k, v)| (k.clone(), self.sub(v))));
        if self.win32() && s.wine_version.kind == "wine" {
            s.env_options.push(("WINEARCH".into(), "win32".into()));
        }
        Ok(s)
    }

    /// The script wants a 32-bit prefix (`game.arch: win32`).
    pub(crate) fn win32(&self) -> bool {
        get_nonempty(&self.job.script.game, "arch").is_some_and(|a| a.trim().eq_ignore_ascii_case("win32"))
    }

    /// Checks the chosen Wine and gets umu when Proton needs it.
    pub(crate) fn prepare_wine(&mut self) -> Result<(), String> {
        let s = self.wine_settings(None, &[])?;
        let wine = heronite_launch::wine::prepare(&self.plan, &SystemProbe, &s)?;
        let wants_umu = s.wine_version.kind == "proton" && s.disable_umu != Some(true);
        let umu = heronite_launch::wine::umu_path(&self.plan, &SystemProbe);
        if wants_umu && !SystemProbe.exists(&umu) {
            self.progress(None, tr!("Downloading umu"));
            let cancel = || self.cancelled();
            heronite_launch::runtimes::download(
                &self.job.paths,
                &heronite_launch::net::SystemNet,
                heronite_launch::runtimes::Runtime::Umu,
                &cancel,
            )?;
        }
        for note in wine.notes {
            self.log(&note);
        }
        Ok(())
    }

    pub(crate) fn run_task(&mut self, task: &Task) -> Result<(), String> {
        let probe = SystemProbe;
        match task {
            Task::CreatePrefix { prefix, arch: _, overrides } => {
                let mut env = Vec::new();
                if !overrides.is_empty() {
                    env.push(("WINEDLLOVERRIDES".to_string(), crate::finish::dll_overrides(overrides)));
                }
                let s = self.wine_settings(prefix.as_deref(), &env)?;
                if let Some(parent) = Path::new(&s.wine_prefix).parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                // The group of a finished command is killed: each one waits for
                // the wineserver first, which saves the registry as it exits.
                let wait = wine_tools::wait_prefix_command(&self.plan, &probe, &s)?;
                // umu's `createprefix` makes the prefix and then fails trying to
                // run "createprefix" as a program (umu 1.4): `wineboot -u` does
                // the same cleanly.
                let umu = heronite_launch::wine::prepare(&self.plan, &probe, &s)?.runtime == heronite_launch::WineRuntime::Umu;
                let commands = if umu {
                    vec![wine_tools::wine_command(&self.plan, &probe, &s, vec!["wineboot".into(), "-u".into()])?]
                } else {
                    wine_tools::create_prefix_commands(&self.plan, &probe, &s)?
                };
                let made = || ["user.reg", "pfx/user.reg"].iter().any(|f| Path::new(&s.wine_prefix).join(f).exists());
                for c in commands {
                    if let Err(e) = self.run(&mut std_command(&c, wait.as_ref())) {
                        if e == crate::engine::CANCELLED || !made() {
                            return Err(e);
                        }
                        self.log(&e);
                    }
                }
                if !made() {
                    return Err(tr!("The Wine prefix was not created in {path}", path = s.wine_prefix));
                }
                Ok(())
            }
            Task::WineExec { executable, args, prefix, working_dir, env } => {
                let exe = self.existing_path(executable);
                self.wine_exec(&exe, args, prefix.as_deref(), working_dir.as_deref(), env)
            }
            Task::Winetricks { apps, prefix, env } => {
                let s = self.wine_settings(prefix.as_deref(), env)?;
                let verbs = split_args(&self.sub(apps));
                self.ensure_winetricks(&s)?;
                let built = if verbs.is_empty() {
                    wine_tools::build_tool_command(&self.plan, &probe, &s, &WineTool::Winetricks)?
                } else {
                    wine_tools::winetricks_verbs_command(&self.plan, &probe, &s, &verbs)?
                };
                for n in &built.notes {
                    self.log(n);
                }
                let wait = wine_tools::wait_prefix_command(&self.plan, &probe, &s)?;
                self.run(&mut std_command(&built.command, wait.as_ref()))
            }
            Task::Winecfg { prefix } => {
                let s = self.wine_settings(prefix.as_deref(), &[])?;
                let built = wine_tools::build_tool_command(&self.plan, &probe, &s, &WineTool::Winecfg)?;
                let wait = wine_tools::wait_prefix_command(&self.plan, &probe, &s)?;
                self.run(&mut std_command(&built.command, wait.as_ref()))
            }
            Task::Winekill { prefix } => {
                let s = self.wine_settings(prefix.as_deref(), &[])?;
                let c = wine_tools::kill_prefix_command(&self.plan, &probe, &s)?;
                if let Err(e) = self.run(&mut std_command(&c, None)) {
                    log::info!("winekill: {e}");
                }
                Ok(())
            }
            Task::SetRegedit { path, key, value, kind, prefix } => {
                let value = reg_value(kind, value.as_deref().map(|v| self.sub(v)).as_deref())?;
                let text = format!("REGEDIT4\n\n[{}]\n\"{}\"={value}\n", reg_path(&self.sub(path)), self.sub(key));
                self.import_reg(&text, prefix.as_deref())
            }
            Task::SetRegeditFile { filename, prefix } => {
                let file = self.existing_path(filename);
                self.regedit(&file, prefix.as_deref())
            }
            Task::DeleteRegistryKey { key, prefix } => {
                let text = format!("REGEDIT4\n\n[-{}]\n", reg_path(&self.sub(key)));
                self.import_reg(&text, prefix.as_deref())
            }
            Task::EjectDisc => Ok(()),
            Task::DosExec { executable, args, working_dir, config_file, exit } => {
                let bin =
                    crate::exec::which_any(&["dosbox", "dosbox-staging", "dosbox-x"]).ok_or_else(|| tr!("DOSBox is not installed"))?;
                let exe = self.existing_path(executable);
                let mut cmd = Command::new(bin);
                cmd.arg(&exe);
                if let Some(conf) = config_file {
                    cmd.arg("-conf").arg(self.existing_path(conf));
                }
                cmd.args(split_args(&self.sub(args)));
                if *exit {
                    cmd.arg("-exit");
                }
                let dir = working_dir.as_deref().map(|w| self.path(w)).or_else(|| exe.parent().map(Path::to_path_buf));
                if let Some(dir) = dir.filter(|d| d.is_dir()) {
                    cmd.current_dir(dir);
                }
                self.run(&mut cmd)
            }
        }
    }

    /// Runs a Windows program in the prefix (`.msi` through `msiexec`,
    /// `.bat` / `.cmd` through `cmd`) and waits for the prefix to be idle.
    pub(crate) fn wine_exec(
        &self,
        exe: &Path,
        args: &str,
        prefix: Option<&str>,
        working_dir: Option<&str>,
        env: &[(String, String)],
    ) -> Result<(), String> {
        if !exe.is_file() {
            return Err(tr!("{path} does not exist", path = exe.display()));
        }
        let s = self.wine_settings(prefix, env)?;
        let file = exe.to_string_lossy().into_owned();
        let lower = file.to_ascii_lowercase();
        let mut parts = if lower.ends_with(".msi") {
            vec!["msiexec".to_string(), "/i".into(), file.clone()]
        } else if lower.ends_with(".bat") || lower.ends_with(".cmd") {
            vec!["cmd".to_string(), "/c".into(), file.clone()]
        } else {
            vec![file.clone()]
        };
        parts.extend(split_args(&self.sub(args)));
        let probe = SystemProbe;
        let install_path = self.job.game_dir.to_string_lossy().into_owned();
        let mut c = wine_tools::wine_command_for(&self.plan, &probe, &s, parts, Some(&install_path))?;
        let dir = working_dir.map(|w| self.path(w)).filter(|d| d.is_dir()).or_else(|| exe.parent().map(Path::to_path_buf));
        c.cwd = dir.map(|d| d.to_string_lossy().into_owned());
        let wait = wine_tools::wait_prefix_command(&self.plan, &probe, &s)?;
        self.run(&mut std_command(&c, wait.as_ref()))
    }

    fn import_reg(&self, text: &str, prefix: Option<&str>) -> Result<(), String> {
        let file = self.job.cache_dir().join("winekeys.reg");
        crate::fsops::write_text(&file, text, false)?;
        let result = self.regedit(&file, prefix);
        let _ = std::fs::remove_file(&file);
        result
    }

    fn regedit(&self, file: &Path, prefix: Option<&str>) -> Result<(), String> {
        let s = self.wine_settings(prefix, &[])?;
        let parts = vec!["regedit".to_string(), "/S".into(), file.to_string_lossy().into_owned()];
        let c = wine_tools::wine_command(&self.plan, &SystemProbe, &s, parts)?;
        let wait = wine_tools::wait_prefix_command(&self.plan, &SystemProbe, &s)?;
        self.run(&mut std_command(&c, wait.as_ref()))
    }

    /// Winetricks without umu runs the script from the Wine tools; it is
    /// fetched when missing.
    fn ensure_winetricks(&self, s: &GameSettings) -> Result<(), String> {
        let wine = heronite_launch::wine::prepare(&self.plan, &SystemProbe, s)?;
        if wine.runtime == heronite_launch::WineRuntime::Umu {
            return Ok(());
        }
        if Path::new(&wine_tools::winetricks_path(&self.plan)).exists() {
            return Ok(());
        }
        self.progress(None, tr!("Downloading Winetricks"));
        let cancel = || self.cancelled();
        heronite_launch::tools::download_winetricks(&self.job.paths, &heronite_launch::net::SystemNet, &cancel).map(|_| ())
    }
}

/// The `wineVersion` value of a game's settings, as stored.
pub fn wine_name(version: &Value) -> String {
    version.get("name").and_then(Value::as_str).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_values() {
        assert_eq!(reg_value("REG_SZ", Some("x")).unwrap(), "\"x\"");
        assert_eq!(reg_value("reg_dword", Some("00000001")).unwrap(), "dword:00000001");
        assert_eq!(reg_value("REG_BINARY", Some("01 02")).unwrap(), "hex:01,02");
        assert_eq!(reg_value("REG_SZ", None).unwrap(), "-");
        assert!(reg_value("REG_WHAT", Some("x")).is_err());
        assert_eq!(reg_path("HKEY_CURRENT_USER/Software/Wine"), "HKEY_CURRENT_USER\\Software\\Wine");
    }

    #[test]
    fn waiting_wraps_the_command() {
        let c = heronite_launch::Command {
            program: "/w/wine".into(),
            args: vec!["setup.exe".into(), "/S".into()],
            env: Default::default(),
            cwd: None,
        };
        let mut wait_env = heronite_launch::env::Env::new();
        wait_env.insert("WINEPREFIX".into(), "/pfx".into());
        let w = heronite_launch::Command { program: "/w/wineserver".into(), args: vec!["-w".into()], env: wait_env, cwd: None };
        let cmd = std_command(&c, Some(&w));
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(cmd.get_program(), "sh");
        assert_eq!(args[2..], ["/w/wine", "setup.exe", "/S"]);
        let out = Command::new("sh")
            .arg("-c")
            .arg(WAIT_SCRIPT)
            .arg("sh")
            .arg("-c")
            .arg("exit 3")
            .env("HERONITE_WAIT_SERVER", "true")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(3), "the command's exit code is kept");
    }
}
