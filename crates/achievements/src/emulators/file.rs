//! Reading and writing the login in one emulator config file (and its
//! token file, when the emulator keeps the token apart).

use super::ini::Ini;
use super::secret;
use super::spec::{Spec, Token};
use super::Login;
use std::fs;
use std::path::{Path, PathBuf};

/// What the machine offers for DuckStation's token encryption: `None` when
/// there is no machine id.
pub type MachineKey<'a> = &'a dyn Fn() -> Option<String>;

fn read_ini(path: &Path) -> Result<Ini, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Ini::parse(&text))
}

/// Replaces `path` atomically, keeping its permissions (`mode` for a new file).
/// A symlink (a dotfiles manager) is followed, so the link stays.
fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mode = fs::metadata(&real).ok().and_then(|m| heronite_platform::fs::mode(&m)).unwrap_or(mode);
    heronite_data::temps::write_atomic(&real, bytes, Some(mode)).map_err(|e| format!("{}: {e}", path.display()))
}

fn save_ini(path: &Path, ini: &Ini) -> Result<(), String> {
    write_file(path, ini.render().as_bytes(), 0o600)
}

/// Sets `key` only when the file already has it.
fn set_existing(ini: &mut Ini, section: &str, key: &str, value: &str, quote: bool) {
    if ini.get(section, key).is_some() {
        ini.set(section, key, value, quote);
    }
}

fn side_file(config: &Path, name: &str) -> PathBuf {
    config.with_file_name(name)
}

/// `secrets.ini` next to a PCSX2 config, when PCSX2 made one.
fn secrets_ini(config: &Path) -> Option<PathBuf> {
    Some(side_file(config, "secrets.ini")).filter(|p| p.is_file())
}

pub fn sign_in(spec: &Spec, config: &Path, login: &Login, now: u64, machine_key: MachineKey) -> Result<(), String> {
    let mut ini = read_ini(config)?;
    let (s, q) = (spec.section, spec.quote);
    ini.set(s, spec.enabled, spec.bools.word(true), q);
    ini.set(s, spec.hardcore, spec.bools.word(login.hardcore), q);
    ini.set(s, spec.username, login.username, q);
    if let Some(key) = spec.timestamp {
        ini.set(s, key, &now.to_string(), q);
    }
    for key in spec.blank {
        set_existing(&mut ini, s, key, "", q);
    }
    match spec.token {
        Token::Plain(key) => ini.set(s, key, login.token, q),
        Token::Encrypted(key) => {
            let machine = machine_key().ok_or("no /etc/machine-id to encrypt the token with")?;
            ini.set(s, key, &secret::encrypt(login.token, login.username, &machine), q);
        }
        Token::SideFile(name) => write_file(&side_file(config, name), login.token.as_bytes(), 0o600)?,
        Token::Secrets(key) => match secrets_ini(config) {
            Some(secrets) => {
                let mut sec = read_ini(&secrets)?;
                sec.set(s, key, login.token, q);
                save_ini(&secrets, &sec)?;
                set_existing(&mut ini, s, key, "", q);
            }
            None => ini.set(s, key, login.token, q),
        },
    }
    save_ini(config, &ini)
}

pub fn sign_out(spec: &Spec, config: &Path) -> Result<(), String> {
    let mut ini = read_ini(config)?;
    let (s, q) = (spec.section, spec.quote);
    set_existing(&mut ini, s, spec.enabled, spec.bools.word(false), q);
    set_existing(&mut ini, s, spec.username, "", q);
    for key in spec.timestamp.iter().chain(spec.blank) {
        set_existing(&mut ini, s, key, "", q);
    }
    match spec.token {
        Token::Plain(key) | Token::Encrypted(key) => set_existing(&mut ini, s, key, "", q),
        Token::SideFile(name) => match fs::remove_file(side_file(config, name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{name}: {e}")),
        },
        Token::Secrets(key) => {
            set_existing(&mut ini, s, key, "", q);
            if let Some(secrets) = secrets_ini(config) {
                let mut sec = read_ini(&secrets)?;
                if sec.get(s, key).is_some_and(|v| !v.is_empty()) {
                    sec.set(s, key, "", q);
                    save_ini(&secrets, &sec)?;
                }
            }
        }
    }
    if ini.render() == fs::read_to_string(config).unwrap_or_default() {
        return Ok(());
    }
    save_ini(config, &ini)
}

/// Whether the file holds a login of `username` with a token, achievements on.
pub fn holds(spec: &Spec, config: &Path, username: &str, machine_key: MachineKey) -> Result<bool, String> {
    let ini = read_ini(config)?;
    let s = spec.section;
    let user = ini.get(s, spec.username).unwrap_or_default();
    if user.is_empty() || !user.eq_ignore_ascii_case(username) || ini.get_bool(s, spec.enabled) != Some(true) {
        return Ok(false);
    }
    let token = match spec.token {
        Token::Plain(key) => ini.get(s, key).unwrap_or_default(),
        Token::Encrypted(key) => {
            let stored = ini.get(s, key).unwrap_or_default();
            match machine_key() {
                Some(machine) => secret::decrypt(&stored, &user, &machine).unwrap_or_default(),
                None => String::new(),
            }
        }
        Token::SideFile(name) => fs::read_to_string(side_file(config, name)).unwrap_or_default(),
        Token::Secrets(key) => {
            let in_secrets = secrets_ini(config).and_then(|p| read_ini(&p).ok()).and_then(|i| i.get(s, key));
            in_secrets.filter(|t| !t.is_empty()).or_else(|| ini.get(s, key)).unwrap_or_default()
        }
    };
    Ok(!token.trim().is_empty())
}
