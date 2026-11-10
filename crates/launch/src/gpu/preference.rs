//! Windows' own choice of graphics card for a program: the value named by
//! the program's full path under `HKCU\Software\Microsoft\DirectX\UserGpuPreferences`,
//! which Settings → Display → Graphics writes. Its data is a list of
//! `name=value;` fields; `GpuPreference=1;` is "Power saving" and
//! `GpuPreference=2;` "High performance" (other fields, like
//! `AutoHDREnable` or `SwapEffectUpgradeEnable`, are other settings of the
//! same page and are kept as they are).
//!
//! A game set to a [`Preference`] in its GPU tab ([`KEY_PREFERENCE`]) gets
//! the value for the program Heronite actually starts, right before it starts
//! ([`apply`]), and the value goes back to what it was once the game is over
//! ([`restore`]): put back when there was one, removed when there was none.
//! It goes back only while it still holds what Heronite wrote, so a choice
//! made in Windows' Settings meanwhile is never overwritten.
//!
//! What Heronite wrote and what it replaced are kept in
//! `local_library/gpu-preferences.json` before the registry is touched, so a
//! session the app could not finish (it crashed, the computer went off) is
//! put back later: when the game is picked up again and ends, at the next
//! start of the app ([`restore_all_except`]), or at the next launch of the
//! same program.
//!
//! Every registry access goes through [`Registry`]: [`SystemRegistry`] in
//! the app, [`FakeRegistry`] in tests, which never touch the real one.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use heronite_data::{json_store, DataPaths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::Gpu;
use crate::plan::LaunchPlan;

/// [`Preference::id`] of a game (Windows); empty: Windows' own choice.
pub const KEY_PREFERENCE: &str = "heroniteGpuPreference";

/// Where Windows keeps the per-program values (under `HKEY_CURRENT_USER`).
pub const REGISTRY_KEY: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";

/// The field of the value that holds the choice.
const FIELD: &str = "GpuPreference";

/// The card a game asks Windows for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    /// Nothing is written: whatever Windows (or the user, in its Settings)
    /// decides for the program.
    #[default]
    System,
    /// `GpuPreference=1;`: the integrated card of a laptop.
    PowerSaving,
    /// `GpuPreference=2;`: the dedicated card.
    HighPerformance,
}

impl Preference {
    pub const ALL: [Preference; 3] = [Preference::System, Preference::PowerSaving, Preference::HighPerformance];

    pub fn id(self) -> &'static str {
        match self {
            Preference::System => "",
            Preference::PowerSaving => "power-saving",
            Preference::HighPerformance => "high-performance",
        }
    }

    pub fn parse(s: &str) -> Self {
        Self::ALL.into_iter().find(|p| p.id() == s.trim()).unwrap_or_default()
    }

    /// The number of `GpuPreference` (`None`: nothing to write).
    pub fn value(self) -> Option<u32> {
        match self {
            Preference::System => None,
            Preference::PowerSaving => Some(1),
            Preference::HighPerformance => Some(2),
        }
    }
}

/// The per-program values of the registry.
pub trait Registry: Send + Sync {
    /// The data of `exe`'s value; `None` when there is none. A value of
    /// another type than text is an error: it is not Heronite's to replace.
    fn read(&self, exe: &str) -> io::Result<Option<String>>;
    fn write(&self, exe: &str, data: &str) -> io::Result<()>;
    fn delete(&self, exe: &str) -> io::Result<()>;
}

/// The real registry (Windows; elsewhere every call fails).
pub struct SystemRegistry;

#[cfg(windows)]
impl Registry for SystemRegistry {
    fn read(&self, exe: &str) -> io::Result<Option<String>> {
        use heronite_platform::registry::{self, Hive};
        // Value names are compared without case, as the registry does.
        let Some(values) = registry::values(Hive::CurrentUser, REGISTRY_KEY) else { return Ok(None) };
        match values.into_iter().find(|(name, _)| name.eq_ignore_ascii_case(exe)) {
            None => Ok(None),
            Some((_, registry::Value::Str(data))) => Ok(Some(data)),
            Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "the value is not text")),
        }
    }

    fn write(&self, exe: &str, data: &str) -> io::Result<()> {
        heronite_platform::registry::set_string(heronite_platform::registry::Hive::CurrentUser, REGISTRY_KEY, exe, data)
    }

    fn delete(&self, exe: &str) -> io::Result<()> {
        heronite_platform::registry::delete_value(heronite_platform::registry::Hive::CurrentUser, REGISTRY_KEY, exe)
    }
}

#[cfg(not(windows))]
impl Registry for SystemRegistry {
    fn read(&self, _: &str) -> io::Result<Option<String>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn write(&self, _: &str, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn delete(&self, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// A registry in memory, for tests. Names are compared without case.
#[derive(Default)]
pub struct FakeRegistry {
    values: Mutex<BTreeMap<String, (String, String)>>,
    /// Every write and delete fails.
    pub read_only: bool,
}

impl FakeRegistry {
    pub fn with(self, exe: &str, data: &str) -> Self {
        self.set(exe, data);
        self
    }

    /// What `exe`'s value holds now.
    pub fn get(&self, exe: &str) -> Option<String> {
        self.values.lock().unwrap().get(&exe.to_lowercase()).map(|(_, d)| d.clone())
    }

    /// Changes the value as the user would in Windows' Settings.
    pub fn set(&self, exe: &str, data: &str) {
        self.values.lock().unwrap().insert(exe.to_lowercase(), (exe.to_string(), data.to_string()));
    }

    pub fn len(&self) -> usize {
        self.values.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Registry for FakeRegistry {
    fn read(&self, exe: &str) -> io::Result<Option<String>> {
        Ok(self.get(exe))
    }

    fn write(&self, exe: &str, data: &str) -> io::Result<()> {
        if self.read_only {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.set(exe, data);
        Ok(())
    }

    fn delete(&self, exe: &str) -> io::Result<()> {
        if self.read_only {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.values.lock().unwrap().remove(&exe.to_lowercase());
        Ok(())
    }
}

/// `current` with its `GpuPreference` set to `value`; the other fields stay,
/// in their order. Every field ends with `;`, as Windows writes them.
pub fn with_preference(current: Option<&str>, value: u32) -> String {
    let ours = format!("{FIELD}={value}");
    let mut fields: Vec<String> = current.unwrap_or("").split(';').map(str::trim).filter(|f| !f.is_empty()).map(str::to_string).collect();
    match fields.iter_mut().find(|f| is_field(f)) {
        Some(f) => *f = ours,
        None => fields.push(ours),
    }
    fields.iter().map(|f| format!("{f};")).collect()
}

/// The `GpuPreference` of a value's data.
pub fn preference_of(data: &str) -> Option<u32> {
    data.split(';').find(|f| is_field(f)).and_then(|f| f.split_once('=')).and_then(|(_, v)| v.trim().parse().ok())
}

fn is_field(field: &str) -> bool {
    field.split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(FIELD))
}

/// The value name for a program the launch starts: a full path to an `.exe`
/// with Windows' separators. `None` for anything else (a command found on
/// `PATH`, a script, a URI), which the value cannot name.
pub fn target_exe(program: &str) -> Option<String> {
    let path = program.trim().trim_matches('"').replace('/', "\\");
    let b = path.as_bytes();
    let absolute = (b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') || path.starts_with(r"\\");
    let exe = Path::new(&path).extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    (absolute && exe && !path.contains(r"\..\")).then_some(path)
}

/// The file of what Heronite wrote.
pub fn ledger_path(paths: &DataPaths) -> PathBuf {
    paths.local_library().join("gpu-preferences.json")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LedgerFile {
    /// By the program's path in lower case.
    #[serde(default)]
    written: BTreeMap<String, Written>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// One value Heronite wrote and has not put back yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Written {
    exe: String,
    /// The data Heronite wrote.
    wrote: String,
    /// The data before; `None`: there was no value.
    previous: Option<String>,
}

/// One change at a time to the file and the registry, across sessions.
static LOCK: Mutex<()> = Mutex::new(());

/// What [`apply`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// The value now holds `data` (and is put back after the game).
    Wrote { data: String, previous: Option<String> },
    /// It already asked for this card: nothing written.
    AlreadySet,
}

/// Sets `exe`'s value to ask for `preference`'s card, right before the game
/// starts. Nothing is written for [`Preference::System`].
pub fn apply(reg: &dyn Registry, file: &Path, exe: &str, preference: Preference) -> anyhow::Result<Applied> {
    let Some(value) = preference.value() else { return Ok(Applied::AlreadySet) };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key = exe.to_lowercase();
    let mut f: LedgerFile = json_store::read(file)?;
    let current = reg.read(exe)?;
    // Left by a session that could not put it back: what was there before
    // that session is still what goes back. A value changed since is the
    // user's.
    let stale = f.written.remove(&key).filter(|w| current.as_deref() == Some(w.wrote.as_str()));
    let previous = match &stale {
        Some(w) => w.previous.clone(),
        None => current.clone(),
    };
    let data = with_preference(current.as_deref(), value);
    if current.as_deref() == Some(data.as_str()) && stale.is_none() {
        json_store::write(file, &f)?;
        return Ok(Applied::AlreadySet);
    }
    // Written down first, so even a crash right after leaves a way back.
    f.written.insert(key.clone(), Written { exe: exe.to_string(), wrote: data.clone(), previous: previous.clone() });
    json_store::write(file, &f)?;
    if current.as_deref() != Some(data.as_str()) {
        if let Err(e) = reg.write(exe, &data) {
            f.written.remove(&key);
            f.written.extend(stale.map(|w| (key, w)));
            json_store::write(file, &f)?;
            return Err(e.into());
        }
    }
    Ok(Applied::Wrote { data, previous })
}

/// What [`restore`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restored {
    /// Heronite had written nothing for the program.
    Nothing,
    /// The value is back to what it was.
    PutBack,
    /// There was no value before: it is gone again.
    Removed,
    /// The value changed since Heronite wrote it (in Windows' Settings): it
    /// stays as it is.
    LeftAlone,
}

/// Puts `exe`'s value back after the game, if Heronite wrote it and it still
/// holds what Heronite wrote.
pub fn restore(reg: &dyn Registry, file: &Path, exe: &str) -> anyhow::Result<Restored> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut f: LedgerFile = json_store::read(file)?;
    let Some(w) = f.written.get(&exe.to_lowercase()).cloned() else { return Ok(Restored::Nothing) };
    let done = put_back(reg, &w)?;
    f.written.remove(&exe.to_lowercase());
    json_store::write(file, &f)?;
    Ok(done)
}

fn put_back(reg: &dyn Registry, w: &Written) -> io::Result<Restored> {
    if reg.read(&w.exe)?.as_deref() != Some(w.wrote.as_str()) {
        return Ok(Restored::LeftAlone);
    }
    match &w.previous {
        Some(p) => reg.write(&w.exe, p).map(|_| Restored::PutBack),
        None => reg.delete(&w.exe).map(|_| Restored::Removed),
    }
}

/// Puts back every value of a session that ended without doing it, except
/// the programs of games that still run (`running`). Called when the app
/// starts. Returns the programs put back or left alone.
pub fn restore_all_except(reg: &dyn Registry, file: &Path, running: &dyn Fn(&str) -> bool) -> Vec<(String, Restored)> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut f: LedgerFile = match json_store::read(file) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("reading {}: {e:#}", file.display());
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    f.written.retain(|_, w| {
        if running(&w.exe) {
            return true;
        }
        match put_back(reg, w) {
            Ok(r) => {
                out.push((w.exe.clone(), r));
                false
            }
            Err(e) => {
                log::warn!("putting back the GPU preference of {}: {e}", w.exe);
                true
            }
        }
    });
    if !out.is_empty() {
        if let Err(e) = json_store::write(file, &f) {
            log::warn!("writing {}: {e:#}", file.display());
        }
    }
    out
}

/// Asks the runtime to set the value for the program it starts (Windows).
/// Not for a game another program starts (a store client, Steam): the
/// program that runs it is not the one Heronite starts. Nor for emulators.
pub fn attach(plan: &mut LaunchPlan, g: &Gpu) {
    attach_on(plan, g, cfg!(windows));
}

fn attach_on(plan: &mut LaunchPlan, g: &Gpu, windows: bool) {
    if !windows || g.preference == Preference::System {
        return;
    }
    // An emulator's program draws every game it runs: its own GPU setting
    // is Windows'.
    let p = match plan {
        LaunchPlan::Native(p) | LaunchPlan::Legendary { process: p, .. } => p,
        _ => return,
    };
    if p.handoff.is_some() {
        p.notes.push("GPU preference: another program starts the game; Windows' own choice applies".into());
        return;
    }
    p.gpu_preference = Some(g.preference);
}

#[cfg(test)]
#[path = "preference_tests.rs"]
mod tests;
